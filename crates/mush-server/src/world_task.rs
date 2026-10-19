//! The single-writer world task (design 01 §4.1) plus the M2 write-through
//! persistence seam.
//!
//! One tokio task owns the [`Engine`] (and therefore the `World`) outright —
//! zero locks on game state. Everything else sends [`WorldMsg`] over an mpsc
//! and receives output through the `mush_session::SessionManager`.
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
//!
//! ## No `unreachable!` in the world loop (WS01-S5)
//!
//! An "impossible" state reached inside this task is not a bug report — it is
//! a process kill. The task that owns the world is the one that would unwind,
//! while the store writer keeps its queue and the players keep their
//! connections, so a wrong `unreachable!` here costs the game rather than the
//! turn. Every such site is therefore converted to the posture the rest of the
//! file already takes for an integrity breach: log at `error!`, then either
//! `apply_integrity_freeze` (loud, non-auto-lifting, staff-visible) or the
//! local fail-closed answer (refuse the message, refuse the flush, suppress
//! the effect). `#![deny(clippy::unreachable)]` below keeps it that way for
//! this module; it cannot be a crate-level deny while `lib.rs`, `persist.rs`
//! and `quarantine.rs` still carry production `unreachable!` of their own.
//!
//! Five production `expect` deliberately remain, all of them boot-time or
//! arithmetic-precondition rather than world-state assertions, and none of
//! them reachable from a player line:
//!
//! * [`base_registry`] x2 — command-table collisions at construction, before
//!   the loop exists; a collision is a build-time wiring error.
//! * the delayed-decision recovery arm — `CanonicalId::new` over a literal.
//! * `flush_dirty` — the serial `checked_add`, whose `u64::MAX` case is
//!   refused a few lines above the site.
//! * `@discord/link` — `normalize_link_code` over a code this function just
//!   minted.
//!
//! ## The never-panic contract (WS01-S11)
//!
//! The rule the paragraph above applies locally is a workspace contract, and
//! it is worth stating plainly: **this task must never panic and must never
//! block.** It owns every object in the game. A panic here does not fail a
//! command — it unwinds the only holder of world state while the store writer
//! still has a queue and every player still has a socket, so the blast radius
//! of a wrong `unwrap()` in this file is the game, not the turn. A blocking
//! call is the same outage more slowly: nothing else can touch the world while
//! it waits.
//!
//! Three mechanisms hold the line, and none of them is a code review:
//!
//! 1. **Posture.** An integrity breach is logged at `error!` and answered with
//!    `apply_integrity_freeze` or the local fail-closed refusal — never with
//!    a macro that ends the process. `#![deny(clippy::unreachable)]` below
//!    keeps the strongest of those macros out of this module by compilation.
//! 2. **A boundary** (WS01-S4). The task runs inside a catch, so a panic that
//!    does happen is classified [`crate::WorldOutcome::Faulted`] and becomes an
//!    orderly non-zero exit that drains the writer and fsyncs the quarantine
//!    rather than a silently dead task the gateway keeps talking to. That is
//!    also why no profile may set `panic = "abort"`: abort deletes the catch,
//!    replaces the exit with `SIGABRT`, and loses the batch in hand.
//! 3. **A budget** (WS01-S11). Every production crate's count of `unwrap` /
//!    `expect` / `panic!` / `unreachable!` / `todo!` / `unimplemented!` is
//!    written down in `panic_budget/<crate>.txt` and enforced by
//!    `crates/mush-contract-tests/tests/panic_budget.rs`. The surface may
//!    shrink freely; growing it is a diff a reviewer signs, and in the pre-auth
//!    gateway crates it may not grow at all without a written reason.
//!
//! New code in this task therefore carries its errors: refuse the message,
//! refuse the flush, freeze the world if truth is already broken — and say so
//! in the log. Adding an `expect` here means raising a number in a file whose
//! diff somebody reads.
#![deny(clippy::unreachable)]

use mush_core::composed::ComposedInstallation as PreparedInstallation;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_config::ipguard::{IpGuard, SITEBAN_REFRESH};
use mush_config::metrics::{LlmFeatureStat, LlmTodaySpend, Metrics, WorldSample};
use mush_core::commands::movement::find_exit;
use mush_core::{
    Audience, ControlMsg, DeliveryClass, DeliveryOutcome, DisconnectKind, Effect, Effects, Engine,
    Flags, Inbound, IncidentId, LegalActionQuery, OUTPUT_PREFIX_TAG, OUTPUT_SUFFIX_TAG, ObjId,
    ObjKind, OfferExecutionContext, OfferRouteTable, OfferSubmission, Outbox, OutputBrackets,
    PerceivedRef, PrincipalId, Privileges, ProjectedLegalAction, ReceiptProjectionTable, Registry,
    Role, ScheduledKind, StagedAction, StepOutcome, TickCadence, TypedStateLens, World,
    WorldCommand, WorldEvent, can_perceive, event_subject, is_ignoring, narrative_actor,
    register_core_commands, register_engine_commands, spoof_for_listener,
};
use mush_ipc::{
    ActionSend, DiscordControlOp, GatewayAction, GatewayInput, GatewayRequest,
    MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS, SendTarget,
};
use mush_llm::LlmService;
use mush_proto::{ActionStatusId, OutputEvent, ProjectedActionStatus, RichText};
use mush_store::{
    AccountRec, AuthoritativeReceipt, AuthoritativeStore, BoundIdempotencyStatus, BundleStatus,
    DiscordBanRec, DiscordCodeRecord, InMemoryStore, IntentReceipt, ObjectStore, SshKeyRec,
};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::action_coordinator::{
    ActionCoordinator, ActionPermit, ActionPreflight, MAX_PREFLIGHT_ATTEMPTS,
    PREFLIGHT_RETRY_WINDOW, StagedAuthoritativeAction, StagedDecisionAttempt, StagedDecisionLease,
    StagedDecisionResponse,
};
use crate::decision_dispatch::{
    DecisionDispatchReservation, DecisionDispatcher, DecisionProviderRegistry,
    DecisionWorkerCompletion, MAX_DECISION_COMPLETIONS,
};
use crate::durable_outbox::{
    DecisionWakeDispatch, DecisionWakeOutboxAdapter, DurableOutboxPolicy, DurableOutboxRegistry,
    decision_wake_mailbox, run_authoritative_outbox_worker,
};
use crate::gateway_sink::GatewaySink;
use crate::persist::{
    self, AuthoritativeCommitFailure, PersistenceIntegrityLatch, StoreJob, StoreWriter,
    WorldExitStatus,
};
use crate::retained::RetainedSeals;
use crate::staff::{StaffOp, StaffOpKind, StaffQueue};

/// The server-side view of the delivery seam: an `Arc<dyn GatewaySink>` the
/// world loop hands its resolved output and session-control to. The all-in-one
/// binary passes an in-process sink (routes to the shared `SessionManager`); the
/// forked server passes the socket sink (serializes `ToGateway` frames).
pub type Gateway = std::sync::Arc<dyn GatewaySink>;

/// The longest relayed line the server speaks (programme D4), characters. The
/// bridge bounds its lines the same way; this is the server's own guard.
pub const MAX_RELAY_LINE_CHARS: usize = 1_000;
use mush_config::{unix_now_ms, unix_now_s};

/// How long the world loop gives an already-dequeued exact commit to reach its
/// receipt after a shutdown was requested, before it preserves the retained
/// seals and stops anyway (`Wake::ShutdownDeadline`).
///
/// Public because it is half of a *caller's* bound: whoever joins the world
/// task has to wait at least this long or it will report a healthy world as a
/// hung one. It was written inline twice below and nowhere else, so the two
/// bounds could drift apart in silence — see [`WORLD_JOIN_TIMEOUT`].
pub const WORLD_SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

/// What a joiner allows the world task ON TOP of its own grace: the post-loop
/// work between the last `break` and the task returning (seal preservation, the
/// final metrics publish, dropping the engine).
pub const WORLD_JOIN_SLACK: Duration = Duration::from_secs(15);

/// The bound every join of the world task uses.
///
/// Strictly longer than the world's own grace, because a joiner that expired
/// first would classify the *designed* slow path — a shutdown that waited out
/// its commit grace — as a hung world, and an operator would page on a box that
/// was doing exactly the right thing. The slack above it is the post-loop tail.
pub const WORLD_JOIN_TIMEOUT: Duration =
    Duration::from_secs(WORLD_SHUTDOWN_GRACE.as_secs() + WORLD_JOIN_SLACK.as_secs());

/// Capacity of the world task's inbound channel.
const INBOUND_CAPACITY: usize = 1024;

/// Barrier waiters retained behind one exact receipt. Further callers fail
/// closed (`WorldHandle::barrier` returns false) instead of growing an
/// unbounded side queue during a prolonged store outage.
const MAX_PENDING_BARRIERS: usize = 256;

/// **How many typed lines one connection may have waiting behind a commit**
/// (16b cut B.5, `B-13`; owner default Q7).
///
/// A line that meets another action's commit waits in the [`DeferQueue`] rather
/// than being refused at once. Four is a burst a player actually types or pastes
/// (four web lines in one send lost three before this); a fifth from the same
/// connection is refused with [`RefusalReason::CommitPending`] exactly as every
/// line was before. Counts [`Inbound::Cmd`] only: a query or a world read counts
/// against [`COMMAND_DEFER_CAP`] alone.
const COMMAND_DEFER_DEPTH: usize = 4;

/// **Every entry the [`DeferQueue`] may hold at once**, lines, queries, world
/// reads and barriers together (16b cut B.5). The bound that keeps the queue
/// from being the unbounded side channel doc 09 forbids: 64 fixed-shape entries
/// behind one commit, never more.
const COMMAND_DEFER_CAP: usize = 64;

/// **How long a deferred entry may wait from its arrival**, milliseconds (16b
/// cut B.5; Q7). A line still queued this long after it arrived, while a commit
/// is still pending, is refused in a player's words
/// ([`RefusalReason::CommitTimedOut`]); a line the resolved commit can serve is
/// served, however long that took.
const COMMAND_DEFER_MS: u64 = 250;

/// [`COMMAND_DEFER_MS`] as the `Duration` the queue measures with.
const COMMAND_DEFER: Duration = Duration::from_millis(COMMAND_DEFER_MS);

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

/// Bounded polling cadence for authoritative durable schedule discovery.
/// The typed cursor is truth; this interval is only a wake hint.
const DURABLE_SCHEDULE_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
        /// **The account the character is being made for** (16b cut F.4). The
        /// live account oracle learns `character -> account` in this same turn,
        /// before the reply leaves, so nothing has to wait for a restart to use
        /// an account-scoped surface. `None` only when the caller could not
        /// resolve an account id at all; the character is then unmapped and
        /// every account policy fails closed on it.
        account: Option<mush_core::AccountId>,
        /// Where the result goes.
        reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
    },
    /// **Forget a character in the live account oracle** (16b cut F.4): the
    /// compensating half of [`Self::CreatePlayer`]'s insert, sent by the caller
    /// whose own account-document write then failed. It touches the oracle and
    /// nothing else — no object, no store row — so it can be answered in any
    /// world mode that still runs turns.
    UnlinkPlayer {
        /// The character to forget.
        character: ObjId,
        /// `true` = the oracle held it and no longer does.
        reply: oneshot::Sender<Result<bool, WorldReadError>>,
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
    /// A Discord component interaction the gateway bridge forwarded
    /// (programme D3, 03 §7a.4): dispatched by `custom_id` prefix to the
    /// module handler registered at wiring. A handler may mutate, so this is
    /// a world mutation; the reply carries its bounded text (or why none) and
    /// is sent as soon as the handler returns — what it changed rides this
    /// step's ordinary commit.
    RelaySpeak {
        /// The forwarded line (the bridge authenticated the user and chose the
        /// character; the oracle re-checks the pairing here).
        req: mush_ipc::RelaySpeak,
        /// Spoken, or why not; the outer error is the world busy or gone.
        reply: oneshot::Sender<Result<Result<(), mush_ipc::RelayError>, WorldReadError>>,
    },
    /// A Discord component interaction the gateway bridge forwarded
    /// (programme D3, 03 §7a.4): dispatched by `custom_id` prefix to the
    /// module handler registered at wiring.
    DiscordInteraction {
        /// The forwarded interaction (the bridge authenticated the user).
        interaction: mush_ipc::DiscordInteraction,
        /// The handler's reply.
        reply: oneshot::Sender<
            Result<Result<String, mush_ipc::DiscordInteractionError>, WorldReadError>,
        >,
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
    /// Project the current legal actions for one authenticated actor and
    /// perceived subject inside the authoritative single-writer.
    LegalActions {
        conn: u64,
        actor: ObjId,
        subject: PerceivedRef,
        reply: oneshot::Sender<Result<Vec<ProjectedLegalAction>, LegalActionReadError>>,
    },
    /// Execute one already-preflighted authoritative action inside the world
    /// single-writer. The opaque permit proves the store lookup happened
    /// before provider resolution or truth reads.
    AuthoritativeAction(Box<AuthoritativeActionRequest>),
    /// Re-check one privilege on the world loop for an off-loop executor
    /// holding an authorization taken a queue hop ago (WS02-A1). The staff
    /// executor captures no world borrow, so this is the only way it can ask;
    /// a revocation in the window is recorded as a second `Denied` audit row
    /// against `command` before the `false` goes back. Busy/unavailable is
    /// typed so the caller fails closed instead of disclosing on a guess.
    RecheckPrivilege {
        /// Whose privilege to re-check (the requester, not the subject).
        actor: ObjId,
        /// The bit the command was authorized on.
        privilege: Privileges,
        /// What the op is about, for the audit row.
        subject: Option<ObjId>,
        /// The command name recorded in the audit detail.
        command: &'static str,
        /// `true` = still held.
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

/// Boxed world message payload for one preflighted coordinator action.
pub struct AuthoritativeActionRequest {
    permit: Box<ActionPermit>,
    submission: OfferSubmission,
    reply: oneshot::Sender<Result<AuthoritativeActionWatch, AuthoritativeActionStartError>>,
}

/// A typed refusal for a world read that cannot safely observe authoritative
/// state right now.
///
/// In particular, [`WorldMode::CommitPending`] has already installed the
/// tentative in-RAM post-image but has not received its exact durable receipt.
/// Returning `false` from `NameFree` in that window would lie that a name is
/// taken; returning an input-context default could show the attach look (the
/// `room` query, 16b cut B.4) over a live editor. Callers must keep these
/// outcomes distinct from a real
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

/// Legal-action discovery can distinguish world availability from a subject
/// representation that has no safe truth projection yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LegalActionReadError {
    #[error("the world is busy committing another action")]
    Busy,
    #[error("the world is unavailable")]
    Unavailable,
    #[error("the perceived subject has no authoritative projection path")]
    InvalidSubject,
}

fn legal_action_query(
    engine: &Engine,
    principal: PrincipalId,
    actor: ObjId,
    subject: PerceivedRef,
) -> Result<LegalActionQuery, LegalActionReadError> {
    engine
        .legal_action_query(principal, actor, subject)
        .map_err(|error| match error {
            mush_core::LegalActionSubjectError::Invalid => LegalActionReadError::InvalidSubject,
            mush_core::LegalActionSubjectError::Unavailable => LegalActionReadError::Unavailable,
        })
}

/// Immediate refusal before an authoritative intent can exist.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthoritativeActionStartError {
    #[error("the world is busy with another authoritative action")]
    Busy,
    #[error("the world is unavailable for authoritative actions")]
    Unavailable,
    #[error("the authoritative action was rejected before durable ownership: {0}")]
    Rejected(String),
}

/// Two-phase result of a world-admitted authoritative action.
///
/// `owned` resolves only after exact intent reservation. `finished` resolves
/// after exact finalization and infallible RAM installation. Keeping them
/// separate prevents IPC from confusing writer-queue acceptance with request
/// ownership.
pub struct AuthoritativeActionWatch {
    pub owned: oneshot::Receiver<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>,
    pub finished: oneshot::Receiver<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>,
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
    /// **A deferred entry outlived [`COMMAND_DEFER_MS`] behind a commit that is
    /// still pending** (16b cut B.5, `B-13`). Only a typed line hears its own
    /// sentence; every other message is refused exactly as
    /// [`Self::CommitPending`] refuses it (see [`refuse_world_msg`]).
    CommitTimedOut,
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
            // Ends in the phrase every test rig's commit-pending retry needle
            // matches, so a rig that retries the old refusal retries this one.
            Self::CommitTimedOut => {
                "The world was busy too long for that line; please retry in a moment."
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
        // A composing hint reads the world (the actor, their room) and
        // mutates nothing; it is admitted wherever a read is.
        Inbound::Query { .. } | Inbound::Composing { .. } => IngressClass::WorldRead,
        Inbound::ConnUp { .. } | Inbound::ConnDown { .. } | Inbound::SessionGone { .. } => {
            IngressClass::ConnectionHooks
        }
        // A client's answer to a server-initiated request (interface plan
        // I10) resumes the opening module's logic, which may mutate; it is
        // admitted only where a write is, exactly like async re-entry, so a
        // Respond in a restricted mode is refused and the deadline default
        // fires later.
        Inbound::Respond { .. } | Inbound::Invoke { .. } => IngressClass::AsyncApply,
        Inbound::Apply(_) => IngressClass::AsyncApply,
        Inbound::Control(ControlMsg::Shutdown) => IngressClass::Shutdown,
        Inbound::Control(ControlMsg::Dump) => IngressClass::StaffMutation,
    }
}

fn classify_world_msg(msg: &WorldMsg) -> IngressClass {
    match msg {
        WorldMsg::Inbound(msg) => classify_inbound(msg),
        WorldMsg::ReAttach { .. } => IngressClass::TransportBookkeeping,
        WorldMsg::CreatePlayer { .. }
        | WorldMsg::AuthoritativeAction(_)
        | WorldMsg::DiscordInteraction { .. }
        // A revoked re-check writes an audit row, so it is classed with the
        // writes: refused (fail-closed) in a restricted mode rather than
        // answering `true` off a world it may not read.
        | WorldMsg::RecheckPrivilege { .. }
        | WorldMsg::RelaySpeak { .. } => IngressClass::WorldMutation,
        // **16b cut F.4**: forgetting a character touches the account oracle and
        // nothing else — no object, no store row, no tentative post-image — and
        // its only effect is to make an account policy fail closed, which is
        // always the safe direction. It is classed with the reads because that
        // is the weakest class whose refusal is typed and transient; a caller
        // refused mid-commit says so rather than retrying.
        WorldMsg::UnlinkPlayer { .. }
        | WorldMsg::NameFree { .. }
        | WorldMsg::InInputContext { .. }
        | WorldMsg::ConnectionReady { .. }
        | WorldMsg::LegalActions { .. } => IngressClass::WorldRead,
        WorldMsg::Barrier { .. } => IngressClass::OrderingBarrier,
    }
}

/// Work whose producer has already transferred semantic/lifecycle custody and
/// therefore cannot be answered Busy or dropped merely because another exact
/// receipt is pending. One such item may be held locally to preserve FIFO.
///
/// Nothing else takes that slot. A typed line, a query and the reply-carrying
/// world reads wait in the bounded [`DeferQueue`] instead
/// ([`waits_in_defer_queue`], 16b cut B.5), which never stops `rx` being polled
/// while the commit is in flight, so they cannot head-of-line block later
/// operational traffic; typed service mutations are still refused immediately.
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

/// **What waits in the [`DeferQueue`] when it meets a commit** (16b cut B.5,
/// `B-13`): a typed line, a query, and the five world reads that carry a reply
/// channel. Each is retryable input that has transferred no custody, so a
/// bounded wait costs nothing and saves the player a retry loop they do not
/// have; each is refused as before once the bound is reached or passed.
///
/// Deliberately NOT here: [`Inbound::Composing`] (best-effort, dropped as
/// before); `CreatePlayer`, `AuthoritativeAction`, `RelaySpeak`,
/// `DiscordInteraction`, `RecheckPrivilege`, [`Inbound::Respond`], dumps and
/// staff operations, whose refusals are typed and whose callers retry by design;
/// and the four lifecycle messages [`must_defer_behind_commit`] holds.
fn waits_in_defer_queue(msg: &WorldMsg) -> bool {
    matches!(
        msg,
        WorldMsg::Inbound(Inbound::Cmd { .. } | Inbound::Query { .. } | Inbound::Invoke { .. })
            | WorldMsg::NameFree { .. }
            | WorldMsg::UnlinkPlayer { .. }
            | WorldMsg::InInputContext { .. }
            | WorldMsg::ConnectionReady { .. }
            | WorldMsg::LegalActions { .. }
    )
}

/// Whether a message is a typed line: the one kind `command_processed` counts,
/// and the one kind [`COMMAND_DEFER_DEPTH`] bounds per connection.
const fn is_typed_line(msg: &WorldMsg) -> bool {
    matches!(msg, WorldMsg::Inbound(Inbound::Cmd { .. }))
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
        | StaffOpKind::DiscordStatus { .. }
        | StaffOpKind::DiscordShow { .. }
        | StaffOpKind::Ps
        | StaffOpKind::Stats => IngressClass::StaffStatus,
        StaffOpKind::Shutdown => IngressClass::Shutdown,
        StaffOpKind::SetBan { .. }
        | StaffOpKind::ClearBan { .. }
        | StaffOpKind::LiftTempban { .. }
        | StaffOpKind::Dump
        | StaffOpKind::Boot { .. }
        | StaffOpKind::NewPassword { .. }
        | StaffOpKind::AddSshKey { .. }
        | StaffOpKind::DelSshKey { .. }
        | StaffOpKind::NewChar { .. }
        | StaffOpKind::DiscordLinkMint { .. }
        | StaffOpKind::DiscordUnlinkSelf { .. }
        | StaffOpKind::DiscordFollow { .. }
        | StaffOpKind::DiscordSet { .. }
        | StaffOpKind::DiscordUnlink { .. }
        | StaffOpKind::DiscordBan { .. }
        | StaffOpKind::DiscordUnban { .. } => IngressClass::StaffMutation,
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
    resolve_store_read_only_command(world, msg)
}

/// The one admission that needs the message body: a command line typed while
/// the store is read-only is allowed only if the read-only allowlist clears it.
///
/// Only [`Inbound::Cmd`] classifies as [`IngressClass::Command`], so anything
/// else arriving here means the classifier and this resolver have drifted
/// apart. That is a wiring breach rather than something the player did — but
/// the caller is the single writer, so it fails closed with the refusal the
/// caller was already prepared to hear instead of ending the world task.
fn resolve_store_read_only_command(world: &World, msg: &WorldMsg) -> Admission {
    let WorldMsg::Inbound(inbound) = msg else {
        tracing::error!(
            class = ?classify_world_msg(msg),
            "store-read-only admission reached a message that is not inbound; refusing"
        );
        return Admission::Refuse(RefusalReason::StoreUnavailable);
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
    direct_actions: Option<Arc<DirectActionIngress>>,
}

struct DirectActionIngress {
    store: Arc<dyn ObjectStore>,
    routes: OfferRouteTable,
    projections: ReceiptProjectionTable,
    epoch: u64,
    next_sequence: AtomicU64,
    /// The same registry the world loop and store writer publish into. This
    /// path has no gateway boundary to report through, so a submission
    /// deferred at the preflight bound would otherwise be invisible.
    metrics: Arc<Metrics>,
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

    /// Ask the world task to create a Player named `name` for `account`.
    /// Resolves only after the creation batch is acknowledged by the store.
    ///
    /// **16b cut F.4**: `account` is inserted into the live account oracle in
    /// the same turn the Player is created, so the character can claim a
    /// simulator slot (and hold a focus) the moment it exists. `None` leaves it
    /// unmapped and failing closed.
    pub async fn create_player(
        &self,
        name: &str,
        account: Option<mush_core::AccountId>,
    ) -> Result<ObjId, CreatePlayerErr> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::CreatePlayer {
                name: name.to_string(),
                account,
                reply,
            })
            .await
            .is_err()
        {
            return Err(CreatePlayerErr::WorldGone);
        }
        rx.await.unwrap_or(Err(CreatePlayerErr::WorldGone))
    }

    /// **Forget a character in the live account oracle** (16b cut F.4).
    ///
    /// The undo of [`Self::create_player`]'s insert, for the caller whose own
    /// account-document write then failed: the Player exists and is an orphan on
    /// disk, so it must be an orphan in RAM too. `Ok(false)` means the oracle
    /// did not hold it, which is the same end state.
    ///
    /// # Errors
    ///
    /// [`WorldReadError::Busy`] mid-commit and [`WorldReadError::Unavailable`]
    /// for a gone task. Either leaves the mapping standing until the next boot
    /// re-derives the map from the account documents, so callers say so.
    pub async fn unlink_player(&self, character: ObjId) -> Result<bool, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::UnlinkPlayer { character, reply })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
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

    /// Speak a relayed line (programme D4) as the character the bridge chose,
    /// after the oracle re-checks the `(account, actor)` pairing. The inner
    /// result is spoken-or-why-not; the outer error is the world being busy or
    /// gone.
    pub async fn relay_speak(
        &self,
        req: mush_ipc::RelaySpeak,
    ) -> Result<Result<(), mush_ipc::RelayError>, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::RelaySpeak { req, reply })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Dispatch a Discord component interaction the gateway forwarded
    /// (programme D3) to the module handler owning its `custom_id` prefix.
    /// The inner result is the handler's answer (or why there is none); the
    /// outer error is the world being busy or gone.
    pub async fn discord_interaction(
        &self,
        interaction: mush_ipc::DiscordInteraction,
    ) -> Result<Result<String, mush_ipc::DiscordInteractionError>, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::DiscordInteraction { interaction, reply })
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

    /// Re-check a privilege the world task authorized before it queued an op
    /// to an off-loop executor (WS02-A1). `Ok(false)` means the grant was
    /// revoked in the window and a second `Denied` row is in the audit;
    /// `Err` means the world could not answer, which callers must treat as a
    /// refusal — an authorization is never inferred from a failed read.
    pub async fn recheck_privilege(
        &self,
        actor: ObjId,
        privilege: Privileges,
        subject: Option<ObjId>,
        command: &'static str,
    ) -> Result<bool, WorldReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::RecheckPrivilege {
                actor,
                privilege,
                subject,
                command,
                reply,
            })
            .await
            .is_err()
        {
            return Err(WorldReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
    }

    /// Enumerate the current viewer-safe action offers for an authenticated
    /// connection. The world task owns both connection truth and provider
    /// projection, so no interface can manufacture either side of the query.
    pub async fn legal_actions(
        &self,
        conn: u64,
        actor: ObjId,
        subject: PerceivedRef,
    ) -> Result<Vec<ProjectedLegalAction>, LegalActionReadError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::LegalActions {
                conn,
                actor,
                subject,
                reply,
            })
            .await
            .is_err()
        {
            return Err(LegalActionReadError::Unavailable);
        }
        rx.await.unwrap_or(Err(LegalActionReadError::Unavailable))
    }

    /// Recover an authoritative action result for an authenticated live
    /// connection. The public reference never supplies the principal.
    pub async fn action_status(
        &self,
        conn: u64,
        actor: ObjId,
        status: &ActionStatusId,
    ) -> Result<ProjectedActionStatus, WorldReadError> {
        if !self.connection_ready(conn, actor).await? {
            return Ok(ProjectedActionStatus::Unknown);
        }
        let Some(direct) = &self.direct_actions else {
            return Err(WorldReadError::Unavailable);
        };
        let Some(store) = direct.store.authoritative_store() else {
            return Err(WorldReadError::Unavailable);
        };
        match ActionCoordinator::projected_status(
            store,
            &direct.projections,
            &PrincipalId::Object { id: actor },
            status,
        )
        .await
        {
            Ok(projected) => Ok(projected),
            Err(crate::action_coordinator::ActionStatusError::InvalidReference)
            | Err(crate::action_coordinator::ActionStatusError::Forbidden) => {
                Ok(ProjectedActionStatus::Unknown)
            }
            Err(error) => {
                tracing::error!(%actor, %status, error = %error, "action status projection failed");
                Ok(ProjectedActionStatus::Unavailable {
                    reference: status.clone(),
                })
            }
        }
    }

    /// Project a receipt whose durable binding has already authenticated the
    /// actor (for example, a split-boundary replay after its original
    /// connection disappeared).
    pub fn project_known_action_receipt(
        &self,
        actor: ObjId,
        status: ActionStatusId,
        receipt: &AuthoritativeReceipt,
    ) -> Result<mush_proto::ProjectedActionReceipt, crate::action_coordinator::ActionStatusError>
    {
        let direct = self.direct_actions.as_ref().ok_or(
            crate::action_coordinator::ActionStatusError::Integrity(
                "action projection table is unavailable".to_owned(),
            ),
        )?;
        ActionCoordinator::project_finalized_receipt(
            &direct.projections,
            &PrincipalId::Object { id: actor },
            status,
            receipt,
        )
    }

    /// All-in-one counterpart of the split gateway's replay-custody path.
    /// It performs the same mandatory preflight and does not report accepted
    /// until the exact intent is durably owned. Split mode never calls this;
    /// its socket link retains/replays the request and observes dispositions.
    pub async fn submit_legal_action(&self, action: GatewayAction) -> ActionSend {
        let Some(direct) = &self.direct_actions else {
            return ActionSend::Gone;
        };
        let Some(sequence) = direct
            .next_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .ok()
        else {
            tracing::error!("in-process legal-action request sequence exhausted");
            return ActionSend::Gone;
        };
        let request = GatewayRequest::new(
            direct.epoch,
            sequence,
            GatewayInput::Action(Box::new(action)),
        );
        let envelope = match ActionCoordinator::gateway_ingress(&request, &direct.routes) {
            Ok(envelope) => envelope,
            Err(detail) => {
                tracing::warn!(%detail, "invalid in-process legal-action ingress");
                return ActionSend::Rejected;
            }
        };
        let status = match ActionCoordinator::status_id(envelope.idempotency()) {
            Ok(status) => status,
            Err(error) => {
                tracing::error!(error = %error, "in-process action status identity failed");
                return ActionSend::Gone;
            }
        };
        let viewer = envelope.principal().clone();
        let Some(store) = direct.store.authoritative_store() else {
            tracing::error!("in-process store lacks authoritative request capability");
            return ActionSend::Gone;
        };
        // Bounded exactly like the split boundary's handler, under the shared
        // pair in `action_coordinator`: the loop rides out a store hiccup, it
        // is never the durable answer. What differs is the answer at the
        // bound. The split arm retains custody because its gateway still holds
        // a replay copy; this arm has no second custodian, so it answers
        // `Busy` — "bounded custody full before durable ownership". That is
        // honest here and only here, because `next_sequence` above minted a
        // fresh sequence for this call: no intent under this request's
        // identity can exist yet, so a caller's retry cannot double-apply one.
        let started = tokio::time::Instant::now();
        let mut delay = Duration::from_millis(100);
        let mut attempts = 0u32;
        let preflight = loop {
            attempts += 1;
            match ActionCoordinator::preflight(store, envelope.clone()).await {
                Ok(preflight) => break preflight,
                Err(error) if error.is_transient() => {
                    let spent = started.elapsed();
                    if attempts >= MAX_PREFLIGHT_ATTEMPTS
                        || spent.saturating_add(delay) > PREFLIGHT_RETRY_WINDOW
                    {
                        tracing::error!(
                            error = %error,
                            attempts,
                            spent_ms = spent.as_millis(),
                            "in-process coordinator preflight stayed transient to the bound"
                        );
                        direct.metrics.coordinator_preflight_deferred();
                        return ActionSend::Busy;
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(5));
                }
                Err(error) => {
                    tracing::error!(error = %error, "in-process coordinator preflight failed");
                    return ActionSend::Gone;
                }
            }
        };
        let permit = match preflight {
            crate::action_coordinator::ActionPreflight::Vacant(permit) => permit,
            crate::action_coordinator::ActionPreflight::Pending { .. } => {
                return ActionSend::Owned(status);
            }
            crate::action_coordinator::ActionPreflight::Finalized(receipt) => {
                return match ActionCoordinator::project_finalized_receipt(
                    &direct.projections,
                    &viewer,
                    status,
                    &receipt,
                ) {
                    Ok(receipt) => ActionSend::Finished(receipt),
                    Err(error) => {
                        tracing::error!(error = %error, "in-process finalized retry projection failed");
                        ActionSend::Gone
                    }
                };
            }
            crate::action_coordinator::ActionPreflight::Retired { .. }
            | crate::action_coordinator::ActionPreflight::Conflict => {
                return ActionSend::Rejected;
            }
            crate::action_coordinator::ActionPreflight::Quarantined(receipt) => {
                tracing::error!(
                    token = %receipt.token(),
                    evidence = %receipt.evidence_hash(),
                    "in-process coordinator request is quarantined"
                );
                return ActionSend::Gone;
            }
        };
        let GatewayInput::Action(input) = request.input else {
            // This function built `request` from `GatewayInput::Action` a few
            // lines up; a different variant here means the ingress types moved
            // under it. Answer the way every other internal failure in this
            // function answers — nothing was submitted — rather than unwinding
            // the caller's task.
            tracing::error!("in-process legal-action request lost its action variant");
            return ActionSend::Gone;
        };
        match self.connection_ready(input.conn, input.actor).await {
            Ok(true) => {}
            Ok(false) => return ActionSend::Rejected,
            Err(WorldReadError::Busy) => return ActionSend::Busy,
            Err(WorldReadError::Unavailable) => return ActionSend::Gone,
        }
        let submission = OfferSubmission {
            offer: input.offered_action,
            subject: input.subject,
            targets: input.targets,
            parameters: input.parameters,
        };
        let watch = match self.authoritative_action(permit, submission).await {
            Ok(watch) => watch,
            Err(AuthoritativeActionStartError::Busy) => return ActionSend::Busy,
            Err(AuthoritativeActionStartError::Unavailable) => return ActionSend::Gone,
            Err(AuthoritativeActionStartError::Rejected(_)) => return ActionSend::Rejected,
        };
        let owned = match watch.owned.await {
            Ok(Ok(owned)) => owned,
            Ok(Err(AuthoritativeCommitFailure::Refused(_))) => return ActionSend::Busy,
            Ok(Err(AuthoritativeCommitFailure::Integrity(_))) | Err(_) => {
                return ActionSend::Gone;
            }
        };
        match tokio::time::timeout(Duration::from_secs(2), watch.finished).await {
            Ok(Ok(Ok(finished))) if finished == owned => {
                match store
                    .authoritative_status(finished.token, finished.bundle_hash)
                    .await
                {
                    Ok(BundleStatus::FinalizedExact(receipt)) => {
                        match ActionCoordinator::project_finalized_receipt(
                            &direct.projections,
                            &PrincipalId::Object { id: input.actor },
                            status.clone(),
                            &receipt,
                        ) {
                            Ok(receipt) => ActionSend::Finished(receipt),
                            Err(error) => {
                                tracing::error!(error = %error, "in-process receipt projection failed");
                                ActionSend::Owned(status)
                            }
                        }
                    }
                    Ok(other) => {
                        tracing::error!(
                            ?other,
                            "finalized action did not have a finalized store receipt"
                        );
                        ActionSend::Owned(status)
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "finalized action receipt read failed");
                        ActionSend::Owned(status)
                    }
                }
            }
            Ok(Ok(Ok(finished))) => {
                tracing::error!(owned_token = %owned.token, finished_token = %finished.token, "in-process terminal identity mismatch");
                ActionSend::Gone
            }
            Ok(Ok(Err(error))) => {
                tracing::error!(error = %error.detail(), "owned in-process action ended in an integrity incident");
                ActionSend::Owned(status)
            }
            Ok(Err(_)) | Err(_) => ActionSend::Owned(status),
        }
    }

    /// Hand one preflighted semantic action to the world single-writer.
    ///
    /// The returned watch is not an acceptance acknowledgement. Its `owned`
    /// receiver is the only proof that the K3 writer crossed the exact durable
    /// intent boundary; callers must retain pre-intent custody until then.
    pub async fn authoritative_action(
        &self,
        permit: Box<ActionPermit>,
        submission: OfferSubmission,
    ) -> Result<AuthoritativeActionWatch, AuthoritativeActionStartError> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::AuthoritativeAction(Box::new(
                AuthoritativeActionRequest {
                    permit,
                    submission,
                    reply,
                },
            )))
            .await
            .is_err()
        {
            return Err(AuthoritativeActionStartError::Unavailable);
        }
        rx.await
            .unwrap_or(Err(AuthoritativeActionStartError::Unavailable))
    }

    /// Typed round-trip barrier: resolves once everything sent through this
    /// handle before the call has been processed and its output delivered.
    /// A line or a read still waiting in the world's defer queue behind a
    /// commit (16b cut B.5) counts as NOT yet processed: a barrier that arrives
    /// behind queued entries waits behind them and completes after the last is
    /// served or refused, and behind the receipt of any commit one of them
    /// starts.
    /// Bounded pending-custody overload — the retained-waiter cap, or a defer
    /// queue already holding its 64 entries — stays retryable
    /// [`WorldReadError::Busy`], never an early `Ok`, while an aborted receipt,
    /// explicit refusal, or vanished task is
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
///
/// Production builds it through [`ServerDeps::production`], the one constructor
/// both binaries call; [`ServerDeps::in_memory`] is the tests' and the compat
/// path's.
pub struct ServerDeps {
    /// The persistence backend.
    pub store: Arc<dyn ObjectStore>,
    /// The shared per-IP guard (its siteban mirror is updated here).
    pub guard: Arc<IpGuard>,
    /// The queue `register_server_commands` handlers push into.
    pub staff: Arc<StaffQueue>,
    /// The M7 LLM service handle. Present on every production world task
    /// (through [`ServerDeps::production`], the booted engine's own service);
    /// `EffectDispatcher` uses it to run a module's deferred
    /// [`Effect::ModuleAsync`] (worldgen `@gen`, an NPC turn, `@weather/generate`,
    /// a tacsim squad's deliberation) off the world loop and re-enter as
    /// [`WorldCommand::ModuleAsyncDone`]. `None` only for
    /// [`ServerDeps::in_memory`] and the rigs that boot no LLM-driven module —
    /// such an effect is then logged and, for worldgen, npc and weather,
    /// dropped; a tacsim deliberation re-enters an `unavailable` refusal (the
    /// squad stays on its machines' own minds); never a panic. The M8 NPC module
    /// shares this same handle (its off-loop turn is driven from `EffectDispatcher`);
    /// its on-loop ability execution goes through the module's grounding adapter,
    /// not this struct.
    pub llm: Option<Arc<LlmService>>,
    /// The shared metrics registry (PLAN "Observability"): the world loop
    /// records loop lag every iteration and publishes the world-owned gauges
    /// (object counts, store health, scheduler depth, NPC + LLM aggregates)
    /// each throttled sample; the store writer stamps write-queue depth and
    /// last-ack age. Read back by `mush-net-web`'s `/healthz` + `/metrics`.
    pub metrics: Arc<Metrics>,
    /// Required unified persistence quarantine sink. Production supplies a
    /// durable, boot-validated spool for ledger seals and complete K3 bundles;
    /// tests opt into the explicit in-memory implementation. There is no
    /// log-only preservation mode.
    pub quarantine_spool: crate::quarantine::QuarantineSpool,
    /// Durable-outbox delivery adapters the deployment supplies beyond the
    /// engine's own (programme D5: the on-call Discord DM adapter). Registered
    /// into the outbox registry at spawn; a duplicate is logged and skipped.
    pub outbox_adapters: Vec<Arc<dyn crate::durable_outbox::DurableOutboxAdapter>>,
}

impl ServerDeps {
    /// The production dependencies of the world task: what BOTH binaries hand
    /// `spawn_persist_with_gateway_production` (the split, `mush_server::serve`)
    /// and `spawn_persist_with_decision_workers` (the all-in-one, `mushd::main`).
    /// `staff` and `llm` are the booted engine's own — the queue the registry
    /// handlers push into and the service the modules were composed with — so
    /// neither can be an orphan; `metrics` is the ONE registry the caller's
    /// ingress stamps too (the `SocketSink` and the writer on the split, the SSH
    /// and web listeners on the all-in-one). 16b cut AC.1 (16c `B-82`): the split
    /// half used to build these from `in_memory()` and overwrite five fields,
    /// leaving `llm` `None`, so on dev1 every LLM-backed `Effect::ModuleAsync` met
    /// no service — worldgen, npc and weather dropped, a squad's deliberation
    /// refused and the squad left on its own — while the all-in-one ran them.
    #[must_use]
    pub fn production(
        booted: &crate::boot::BootedEngine,
        store: Arc<dyn ObjectStore>,
        guard: Arc<IpGuard>,
        metrics: Arc<Metrics>,
        quarantine_spool: crate::quarantine::QuarantineSpool,
        outbox_adapters: Vec<Arc<dyn crate::durable_outbox::DurableOutboxAdapter>>,
    ) -> ServerDeps {
        ServerDeps {
            store,
            guard,
            staff: Arc::clone(&booted.staff),
            llm: Some(Arc::clone(&booted.llm)),
            metrics,
            quarantine_spool,
            outbox_adapters,
        }
    }

    /// Throwaway deps for the tests and for `mushd::world_task::spawn`'s compat
    /// path: a fresh in-memory store, a fresh guard, a never-populated staff
    /// queue, NO LLM service, a throwaway metrics registry and an in-memory
    /// quarantine spool. Never production's: both binaries build theirs with
    /// [`ServerDeps::production`], and a production world task whose `llm` is
    /// `None` is the 16c `B-82` defect (every LLM-backed `Effect::ModuleAsync`
    /// met no service: worldgen, npc and weather dropped, a squad's deliberation
    /// refused and the squad left on its own).
    #[must_use]
    pub fn in_memory() -> ServerDeps {
        ServerDeps {
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        }
    }
}

/// How the world loop's own function ended.
///
/// Distinct from [`WorldExitStatus`], which answers whether the loop reached a
/// *clean shutdown*: this says whether it walked out of its state machine at
/// all, or was thrown out of it by a panic the dispatch boundary caught. Every
/// ordinary break — including the ones that are not clean exits — is
/// [`Clean`](WorldOutcome::Clean); only the firewall produces
/// [`Faulted`](WorldOutcome::Faulted).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldOutcome {
    /// The loop ended through one of its own breaks.
    Clean,
    /// A synchronous engine entry panicked; the loop froze persistence,
    /// discarded the faulted turn and stopped.
    Faulted(WorldFault),
}

/// One caught panic: where the loop was, and what the payload said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldFault {
    /// The guarded call site (a static label, not a source location).
    pub site: &'static str,
    /// The downcast panic payload, or a placeholder for a non-string one.
    pub detail: String,
}

/// Render a `catch_unwind` payload as text. Both shapes `panic!` produces are
/// covered; anything else is named rather than dropped.
fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
}

/// The world-task dispatch boundary: run one **synchronous** engine entry and
/// turn a panic into a value.
///
/// The world task owns the in-RAM world and must never panic (docs/PLAN.md
/// "Architecture"), but every command handler, tick, observer and hook in the
/// process runs inside it — a single `unwrap` in module code used to take the
/// whole game down while the store writer kept its queue. This is the same
/// shape the script host already uses for the interpreter
/// (`mush_script::runtime`): [`AssertUnwindSafe`](std::panic::AssertUnwindSafe)
/// because the world is genuinely observed after the catch, which is exactly
/// why the caller must freeze persistence rather than carry on.
///
/// It cannot cover a panic across an `.await` — the driver sites are futures,
/// not closures — so the task's `JoinError` remains the net for those.
fn guard<T>(site: &'static str, call: impl FnOnce() -> T) -> Result<T, WorldFault> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|payload| {
        let detail = panic_detail(payload.as_ref());
        tracing::error!(
            site,
            detail = %detail,
            "WORLD TASK PANIC caught at the dispatch boundary; the world is stopping frozen"
        );
        WorldFault { site, detail }
    })
}

/// Handles from [`spawn_persist_with_gateway`]: the sender everything talks through and
/// the join handles the shutdown sequence awaits **in order** (world first,
/// then the writer — dropping the world task's job sender is what lets the
/// writer drain and exit).
pub struct PersistHandles {
    /// Sender handle for the world task.
    pub handle: WorldHandle,
    /// The world task itself. Its value says whether the loop ended through
    /// its own state machine or through the panic firewall.
    pub world_join: JoinHandle<WorldOutcome>,
    /// The store-writer task (drains after the world task ends).
    pub writer_join: JoinHandle<()>,
    /// Recovery-driven durable external-delivery worker.
    pub outbox_join: JoinHandle<()>,
    /// 16b cut T.2 (T1): the shared table of running arena tasks, or `None` when
    /// no game layer composed one.
    ///
    /// It is on this struct because the two halves that need it do not run in
    /// the same place: matches are started and driven from **inside** the world
    /// loop (where the authority checks live), and joined by the process's
    /// shutdown tail, which is outside it. `arena_shutdown` is what both tails
    /// call.
    pub arenas: Option<tacsim::arena_bridge::ArenaTasks>,
    /// Process-local fail-closed snapshot barrier shared by the world,
    /// store writer, and shutdown coordinator.
    pub persistence_integrity: PersistenceIntegrityLatch,
    /// Whether the world task reached one of its clean exits. Read by the
    /// shutdown coordinator before the ordinary final BGSAVE; false until the
    /// world loop's own last statement sets it, so a panicked or abandoned
    /// world task can never be snapshotted over.
    pub world_exit: WorldExitStatus,
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
/// The durable-outbox registry every spawn path uses: the production adapters
/// (`production_durable_outbox` — E7 cut 8's tactics boundary outbox) plus the
/// deployment's own (`ServerDeps::outbox_adapters` — programme D5's on-call
/// Discord DM adapter among them). A duplicate `(owner, adapter)` is logged
/// and skipped.
fn outbox_registry(deps: &ServerDeps) -> DurableOutboxRegistry {
    let mut registry = production_durable_outbox();
    for adapter in &deps.outbox_adapters {
        if let Err(error) = registry.register(Arc::clone(adapter)) {
            tracing::error!(%error, "durable outbox adapter not registered");
        }
    }
    registry
}

pub fn spawn_persist_with_gateway(
    engine: Engine,
    deps: ServerDeps,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    let registry = Arc::new(outbox_registry(&deps));
    spawn_persist_with_gateway_and_outbox(engine, deps, registry, make_gateway)
}

/// The production durable-outbox registry.
///
/// E7 cut 8 (10b §3.8 OPEN 4): `durable_outbox.rs` had the trait and no
/// production implementation, and every spawn path wired an **empty** registry.
/// The tactical pulse's typed boundary facts (T2, D-E7-17) are the first thing
/// that has to cross it, so the tactics adapter is registered here, on the one
/// path every spawn shares.
///
/// A registration that will not build is logged and skipped rather than
/// panicking a boot: an unroutable outbox item is retained by the store and
/// retried when the adapter appears, which is strictly better than refusing to
/// start.
#[must_use]
pub fn production_durable_outbox() -> DurableOutboxRegistry {
    let mut registry = DurableOutboxRegistry::new();
    match crate::tactics_outbox::TacticsBoundaryOutbox::new() {
        Some(outbox) => {
            if let Err(error) = registry.register(Arc::new(outbox)) {
                tracing::error!(%error, "the tactics boundary outbox adapter was not registered");
            }
        }
        None => tracing::error!("the tactics boundary outbox adapter could not be constructed"),
    }
    registry
}

struct DecisionRuntime {
    dispatcher: Arc<DecisionDispatcher>,
    wakes: mpsc::Receiver<DecisionWakeDispatch>,
    completions: mpsc::Receiver<DecisionWorkerCompletion>,
}

/// Spawn with explicit owner-routed delayed-decision providers. This validates
/// every composed source before any worker or outbox task starts and installs
/// its no-I/O wake adapter into the durable delivery registry.
pub fn spawn_persist_with_gateway_and_decisions(
    engine: Engine,
    deps: ServerDeps,
    mut durable_outbox: DurableOutboxRegistry,
    providers: DecisionProviderRegistry,
    worker_limit: usize,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> Result<PersistHandles, String> {
    let sources = engine
        .registry
        .decision_sources()
        .ok_or_else(|| "decision-source registry is unavailable".to_owned())?;
    audit_durable_decision_sources(&engine)?;
    let (wake_tx, wakes) =
        decision_wake_mailbox(mush_core::MAX_DECISION_MAILBOX_IDS).map_err(str::to_owned)?;
    for (owner, adapter) in sources.wake_adapters() {
        if !providers.contains_owner(&owner) {
            return Err(format!(
                "decision source {owner} has no registered async provider"
            ));
        }
        durable_outbox
            .register(Arc::new(DecisionWakeOutboxAdapter::new(
                owner,
                adapter,
                wake_tx.clone(),
            )))
            .map_err(|error| error.to_string())?;
    }
    let (completion_tx, completions) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = DecisionDispatcher::new(Arc::new(providers), worker_limit, completion_tx)
        .map_err(|error| error.to_string())?;
    Ok(spawn_persist_with_gateway_mode(
        engine,
        deps,
        Arc::new(durable_outbox),
        Some(DecisionRuntime {
            dispatcher: Arc::new(dispatcher),
            wakes,
            completions,
        }),
        None,
        make_gateway,
    ))
}

/// H2a: the production spawn. A composition with no decision source spawns
/// exactly as [`spawn_persist_with_gateway`] does; one with composed sources
/// registers one [`crate::decision_llm::LlmDecisionProvider`] per source owner
/// — today `npc`, on [`mush_llm::LlmFeature::NpcDecision`] — over `llm` (an
/// absent service or route answers `Unavailable`, so every decision reaches
/// the owner's fallback: D-H-5) and spawns through
/// [`spawn_persist_with_gateway_and_decisions`], which audits every durable
/// decision record before the world task exists (K7.4).
///
/// # Errors
/// A composed source owner with no production provider; a `decision_workers`
/// outside `1..=MAX_DECISION_PROVIDER_WORKERS`; a malformed durable decision
/// record (the owner is frozen at boot).
pub fn spawn_persist_with_gateway_production(
    engine: Engine,
    deps: ServerDeps,
    llm: Option<Arc<LlmService>>,
    decision_workers: usize,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> Result<PersistHandles, String> {
    let composed = engine
        .registry
        .decision_sources()
        .is_some_and(|sources| !sources.is_empty());
    if !composed {
        return Ok(spawn_persist_with_gateway(engine, deps, make_gateway));
    }
    if !engine.has_decision_keys() {
        // A composition with sources but no key ring (a test rig built
        // without `build_engine`) cannot audit or sign a decision: the
        // sources stay inert and the plain runtime starts. Production always
        // sets its keys before this point.
        tracing::warn!(
            "decision sources are composed but no decision key ring is set; the decision runtime is not started"
        );
        let registry = Arc::new(outbox_registry(&deps));
        return Ok(spawn_persist_with_gateway_mode(
            engine,
            deps,
            registry,
            None,
            None,
            make_gateway,
        ));
    }
    let providers = production_decision_providers(&engine, llm)?;
    let registry = outbox_registry(&deps);
    spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        registry,
        providers,
        decision_workers,
        make_gateway,
    )
}

/// One production provider per composed decision source owner (D-H-4: the
/// adapter is owner-parameterised, so a later owner registers a second
/// instance, not a second implementation).
fn production_decision_providers(
    engine: &Engine,
    llm: Option<Arc<LlmService>>,
) -> Result<DecisionProviderRegistry, String> {
    let mut providers = DecisionProviderRegistry::new();
    let owners = engine
        .registry
        .decision_sources()
        .map(mush_core::DecisionSourceRegistry::wake_adapters)
        .unwrap_or_default();
    for (owner, _) in owners {
        let feature = match owner.as_str() {
            "npc" => mush_llm::LlmFeature::NpcDecision,
            other => {
                return Err(format!(
                    "decision source {other} has no production provider"
                ));
            }
        };
        match &llm {
            Some(llm) if llm.caps_for(feature).is_some() => {}
            Some(_) => tracing::warn!(
                %owner,
                route = feature.route_key(),
                "no [llm.routes.{}] route: every {owner} decision will fall back to its idle action",
                feature.route_key()
            ),
            None => tracing::warn!(
                %owner,
                "no LLM service: every {owner} decision will fall back to its idle action"
            ),
        }
        providers
            .register(Arc::new(crate::decision_llm::LlmDecisionProvider::new(
                owner,
                llm.clone(),
                feature,
            )))
            .map_err(|error| error.to_string())?;
    }
    Ok(providers)
}

/// Exhaustively validate all durable decision bundles and aggregate
/// admission ceilings before the world task—and therefore writable ingress—
/// exists. Pages stay bounded while the compact accounting index enforces the
/// global/controller/actor/storage ceilings across page boundaries.
fn audit_durable_decision_sources(engine: &Engine) -> Result<(), String> {
    let mut cursor = None;
    let mut admission = mush_core::DecisionAdmissionIndex::default();
    loop {
        let page = engine
            .discover_decision_dispatches(cursor.as_ref(), mush_core::MAX_DECISION_RECOVERY_PAGE)
            .map_err(|error| error.to_string())?;
        admission
            .merge_existing(page.admission_index().clone())
            .map_err(|error| error.to_string())?;
        cursor = page.next_cursor().cloned();
        if cursor.is_none() {
            return Ok(());
        }
    }
}

/// Spawn with an explicit frozen durable-delivery registry. Work for a missing
/// adapter remains pending and is surfaced as an operational error.
pub fn spawn_persist_with_gateway_and_outbox(
    engine: Engine,
    deps: ServerDeps,
    durable_outbox: Arc<DurableOutboxRegistry>,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    assert!(
        engine
            .registry
            .decision_sources()
            .is_none_or(mush_core::DecisionSourceRegistry::is_empty),
        "composed decision sources require spawn_persist_with_gateway_and_decisions"
    );
    spawn_persist_with_gateway_mode(engine, deps, durable_outbox, None, None, make_gateway)
}

/// Shared spawn implementation. Unit tests can start the real loop in a hard
/// mode; delayed-decision production wiring supplies the optional runtime.
fn spawn_persist_with_gateway_mode(
    engine: Engine,
    deps: ServerDeps,
    durable_outbox: Arc<DurableOutboxRegistry>,
    decision_runtime: Option<DecisionRuntime>,
    initial_mode: Option<WorldMode>,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // 16b cut T.2 (T1, T4): the game layer's shared arena-task table, created by
    // the tacsim module in `register` and read back out of its own state slot
    // here. An immutable read and a clone of an `Arc` — the host composes the
    // game, it does not write the game's state — so this is not a cross-module
    // `module_state_mut` reach and needs no `state-slot-owners.toml` row. A
    // composition with no game layer leaves it `None` and the arena egress arm
    // disarmed.
    let arenas = engine
        .world
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone());
    let routes = engine
        .registry
        .action_providers()
        .map_or_else(OfferRouteTable::default, |providers| {
            providers.offer_route_table()
        });
    let projections = engine
        .registry
        .action_providers()
        .map_or_else(ReceiptProjectionTable::empty, |providers| {
            providers.receipt_projection_table()
        });
    let action_epoch = crate::action_coordinator::fresh_action_epoch();
    let direct_actions = Arc::new(DirectActionIngress {
        store: Arc::clone(&deps.store),
        routes,
        projections,
        epoch: action_epoch,
        next_sequence: AtomicU64::new(1),
        metrics: Arc::clone(&deps.metrics),
    });
    let handle = WorldHandle {
        tx: tx.clone(),
        shutdown: Some(shutdown_tx),
        direct_actions: Some(direct_actions),
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
    // Set only by the world loop's own final statement, and only for its two
    // clean breaks; the shutdown coordinator gates the final BGSAVE on it.
    let world_exit = WorldExitStatus::new();
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
    // The delivery worker is independent of the world/write FIFO: finalized
    // items survive crashes in the store and are safe to rediscover. Its
    // lifetime sender is owned by the world task, so every world exit stops
    // the worker without keeping the inbound channel alive.
    let (outbox_lifetime, outbox_stop) = watch::channel(false);
    let outbox_join = tokio::spawn(run_authoritative_outbox_worker(
        Arc::clone(&deps.store),
        durable_outbox,
        DurableOutboxPolicy::default(),
        outbox_stop,
    ));
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
        action_epoch,
        persistence_integrity.clone(),
        world_exit.clone(),
        decision_runtime,
        initial_mode,
        outbox_lifetime,
        arenas.clone(),
    ));
    PersistHandles {
        handle,
        world_join,
        writer_join,
        outbox_join,
        persistence_integrity,
        world_exit,
        arenas,
    }
}

/// **Stop and join every arena task** — the one call both shutdown tails make
/// (16b cut T.2, T1).
///
/// Every stop signal is set first, so no task is still trying to publish a result
/// into an egress nobody is draining, and then each is joined under its own
/// bound. `None` (no game layer) is a no-op. Nothing else in the shutdown order
/// changes: it runs after the world task has stopped and before the store
/// writer's drain, because an arena task holds no store handle and therefore
/// nothing the writer is waiting for.
pub async fn arena_shutdown(arenas: Option<tacsim::arena_bridge::ArenaTasks>) {
    if let Some(arenas) = arenas {
        arenas.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
    }
}

/// One `select!` wake: a message, a writer control signal, a due timer, or
/// channel closure.
enum Wake {
    Msg(WorldMsg),
    Signal(WriterSignal),
    CommitReceipt(PendingReceiptEvent),
    Timer,
    /// E7c2 B1: one command-originated authoritative submission is queued and
    /// every precondition for staging it holds.
    CommandSubmits,
    DurableSchedules,
    DecisionNotification(DecisionWakeDispatch),
    DecisionResult(Box<DecisionWorkerCompletion>),
    DurableDecisions,
    /// 16b cut T.2 (T4): a running match published something. The payload is the
    /// first message off the egress; the arm drains the rest without blocking and
    /// re-enters them all through the game's registered async-completion handler,
    /// which is the tree's one "run module code with a `Ctx` from the loop"
    /// mechanism (the same `WorldCommand::ModuleAsyncDone` worldgen and the NPC
    /// layer come back on) rather than a second one.
    ArenaEgress(Box<tacsim::arena_task::ArenaOut>),
    /// 16b cut B.5 (`B-13`): no commit is pending and the defer queue is not
    /// empty; the next entry is taken off its front. A queued message becomes a
    /// [`Wake::Msg`] before the dispatch; this variant survives only when the
    /// queue turned out empty.
    Deferred,
    /// 16b cut B.5: the queued barrier the drain just took off the front.
    DeferredBarrier(oneshot::Sender<Result<(), WorldReadError>>),
    /// 16b cut B.5: the oldest queued message has waited
    /// [`COMMAND_DEFER_MS`] while a commit still pends.
    DeferExpiry,
    Shutdown,
    ShutdownDeadline,
    Closed,
}

struct DurableDecisionDriver {
    cursor: Option<mush_core::DecisionSourceCursor>,
    recovery_audit: mush_core::DecisionAdmissionIndex,
    recovery_complete: bool,
    retry: Option<mush_core::DecisionDispatchCandidate>,
    reconciliation: Option<mush_core::DecisionReconciliationCandidate>,
    fallback_trigger: Option<mush_core::DecisionFallbackTriggerCandidate>,
    fallback: Option<mush_core::DecisionFallbackCandidate>,
    compaction: Option<mush_core::DecisionCompactionCandidate>,
    failure_owner: Option<mush_core::OwnerId>,
    idempotency_epoch: u64,
    dispatcher: Arc<DecisionDispatcher>,
}

impl DurableDecisionDriver {
    fn new(idempotency_epoch: u64, dispatcher: Arc<DecisionDispatcher>) -> Self {
        Self {
            cursor: None,
            recovery_audit: mush_core::DecisionAdmissionIndex::default(),
            recovery_complete: false,
            retry: None,
            reconciliation: None,
            fallback_trigger: None,
            fallback: None,
            compaction: None,
            failure_owner: None,
            idempotency_epoch,
            dispatcher,
        }
    }

    async fn drive_one(
        &mut self,
        engine: &mut Engine,
        store: &dyn AuthoritativeStore,
        wake: Option<&DecisionWakeDispatch>,
    ) -> Result<DurableDecisionDrive, String> {
        self.failure_owner = None;
        if engine.has_legacy_persistence_work() {
            return Ok(DurableDecisionDrive::DeferredByLegacyPredecessor);
        }
        if !self.recovery_complete {
            let page = match engine.discover_decision_dispatches(
                self.cursor.as_ref(),
                mush_core::MAX_DECISION_RECOVERY_PAGE,
            ) {
                Ok(page) => page,
                Err(error) => {
                    self.failure_owner = error.owner().cloned();
                    return Err(error.to_string());
                }
            };
            self.recovery_audit
                .merge_existing(page.admission_index().clone())
                .map_err(|error| error.to_string())?;
            self.cursor = page.next_cursor().cloned();
            if self.cursor.is_some() {
                return Ok(DurableDecisionDrive::RecoveryAuditProgress);
            }
            self.recovery_complete = true;
            self.cursor = None;
        }
        if self.retry.is_none()
            && self.reconciliation.is_none()
            && self.fallback_trigger.is_none()
            && self.fallback.is_none()
            && self.compaction.is_none()
        {
            if let Some(wake) = wake {
                self.retry = match engine.decision_dispatch_for_wake(wake.owner(), &wake.wake()) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        self.failure_owner = Some(wake.owner().clone());
                        return Err(error.to_string());
                    }
                };
            } else {
                let page = match engine.discover_decision_dispatches(self.cursor.as_ref(), 1) {
                    Ok(page) => page,
                    Err(error) => {
                        self.failure_owner = error.owner().cloned();
                        return Err(error.to_string());
                    }
                };
                self.cursor = page.next_cursor().cloned();
                self.retry = page.candidates().first().cloned();
                self.reconciliation = page.reconciliations().first().cloned();
                self.fallback_trigger = page.fallback_triggers().first().cloned();
                self.fallback = page.fallbacks().first().cloned();
                self.compaction = page.compactions().first().cloned();
            }
        }
        if let Some(candidate) = self.compaction.clone() {
            self.failure_owner = Some(candidate.owner().clone());
            if let Some(binding) = candidate.action_binding() {
                let status = store
                    .bound_authoritative_idempotency_status(candidate.action_idempotency())
                    .await
                    .map_err(|error| error.to_string())?;
                match status {
                    BoundIdempotencyStatus::FinalizedExact(receipt) => {
                        if receipt.idempotency() != candidate.action_idempotency()
                            || receipt.request() != binding.request
                            || receipt.request_hash() != binding.request_hash
                        {
                            return Err(
                                "terminal decision attribution disagrees with finalized action receipt"
                                    .to_owned(),
                            );
                        }
                    }
                    BoundIdempotencyStatus::PendingExact { .. } => {
                        return Ok(DurableDecisionDrive::Transient(
                            "terminal decision compaction waits for its selected action receipt"
                                .to_owned(),
                        ));
                    }
                    BoundIdempotencyStatus::Vacant => {
                        return Err(
                            "terminal decision compaction has no authoritative selected action"
                                .to_owned(),
                        );
                    }
                    BoundIdempotencyStatus::Retired { .. } => {
                        return Err(
                            "terminal decision selected-action attribution was retired".to_owned()
                        );
                    }
                    BoundIdempotencyStatus::QuarantinedExact(_) => {
                        return Err("terminal decision selected action is quarantined".to_owned());
                    }
                }
            }
            match stage_decision_compaction(engine, store, &candidate, self.idempotency_epoch)
                .await?
            {
                Some(staged) => {
                    self.compaction = None;
                    return Ok(DurableDecisionDrive::StagedLifecycle(staged));
                }
                None => {
                    return Ok(DurableDecisionDrive::Transient(
                        "decision compaction action already has durable custody".to_owned(),
                    ));
                }
            }
        }
        if let Some(candidate) = self.reconciliation.clone() {
            self.failure_owner = Some(candidate.fence().controller().owner.clone());
            let status = store
                .bound_authoritative_idempotency_status(candidate.action_idempotency())
                .await
                .map_err(|error| error.to_string())?;
            match status {
                BoundIdempotencyStatus::FinalizedExact(receipt) => {
                    if receipt.idempotency() != candidate.action_idempotency()
                        || receipt.request() != candidate.binding().request
                        || receipt.request_hash() != candidate.binding().request_hash
                    {
                        return Err(
                            "resolving decision binding disagrees with finalized action receipt"
                                .to_owned(),
                        );
                    }
                    match stage_decision_terminal(
                        engine,
                        store,
                        candidate.fence(),
                        candidate.binding(),
                        candidate.outcome(),
                        self.idempotency_epoch,
                    )
                    .await?
                    {
                        Some(staged) => {
                            self.reconciliation = None;
                            return Ok(DurableDecisionDrive::StagedLifecycle(staged));
                        }
                        None => {
                            return Ok(DurableDecisionDrive::Transient(
                                "resolving decision terminal action already has durable custody"
                                    .to_owned(),
                            ));
                        }
                    }
                }
                BoundIdempotencyStatus::PendingExact { .. } => {
                    return Ok(DurableDecisionDrive::Transient(
                        "resolving decision waits for its selected action receipt".to_owned(),
                    ));
                }
                BoundIdempotencyStatus::Vacant => {
                    return Err("resolving decision binding has no authoritative action".to_owned());
                }
                BoundIdempotencyStatus::Retired { .. } => {
                    return Err("resolving decision action binding was retired".to_owned());
                }
                BoundIdempotencyStatus::QuarantinedExact(_) => {
                    return Err("resolving decision action is quarantined".to_owned());
                }
            }
        }
        if let Some(candidate) = self.fallback_trigger.clone() {
            self.failure_owner = Some(candidate.owner().clone());
            match stage_decision_fallback_trigger(engine, store, &candidate, self.idempotency_epoch)
                .await?
            {
                Some(staged) => {
                    self.fallback_trigger = None;
                    return Ok(DurableDecisionDrive::StagedLifecycle(staged));
                }
                None => {
                    return Ok(DurableDecisionDrive::Transient(
                        "fallback-custody action already has durable custody".to_owned(),
                    ));
                }
            }
        }
        if let Some(candidate) = self.fallback.clone() {
            self.failure_owner = Some(candidate.owner().clone());
            let (response, ingress) =
                ActionCoordinator::decision_fallback_ingress(engine, &candidate)
                    .map_err(|error| error.to_string())?;
            match ActionCoordinator::preflight(store, ingress)
                .await
                .map_err(|error| error.to_string())?
            {
                ActionPreflight::Vacant(permit) => {
                    let selected = ActionCoordinator::prepare_and_stage_decision_fallback(
                        permit, engine, &candidate, &response,
                    )
                    .map_err(|error| error.to_string())?;
                    self.fallback = None;
                    return Ok(DurableDecisionDrive::StagedFallback(selected));
                }
                ActionPreflight::Finalized(receipt) => {
                    let binding = mush_core::DecisionActionBinding {
                        request: receipt.request(),
                        request_hash: receipt.request_hash(),
                    };
                    match stage_decision_resolving(
                        engine,
                        store,
                        candidate.fence(),
                        &binding,
                        mush_core::DecisionTerminalOutcome::Fallback,
                        self.idempotency_epoch,
                    )
                    .await?
                    {
                        Some(staged) => {
                            self.fallback = None;
                            return Ok(DurableDecisionDrive::StagedLifecycleWithTerminal {
                                staged,
                                fence: candidate.fence().clone(),
                                binding,
                                outcome: mush_core::DecisionTerminalOutcome::Fallback,
                            });
                        }
                        None => {
                            return Ok(DurableDecisionDrive::Transient(
                                "fallback resolving action already has durable custody".to_owned(),
                            ));
                        }
                    }
                }
                ActionPreflight::Pending { .. } => {
                    return Ok(DurableDecisionDrive::Transient(
                        "fallback selected action waits for its exact receipt".to_owned(),
                    ));
                }
                ActionPreflight::Retired { .. } => {
                    return Err("fallback action idempotency binding was retired".to_owned());
                }
                ActionPreflight::Quarantined(_) => {
                    return Err("fallback selected action is quarantined".to_owned());
                }
                ActionPreflight::Conflict => {
                    return Err("fallback lost to a conflicting semantic action".to_owned());
                }
            }
        }
        let Some(candidate) = self.retry.as_ref() else {
            return Ok(DurableDecisionDrive::Idle);
        };
        self.failure_owner = Some(candidate.owner().clone());
        let reservation = match self.dispatcher.try_reserve(candidate.controller()) {
            Ok(reservation) => reservation,
            Err(crate::decision_dispatch::DecisionDispatchReserveError::Busy) => {
                return Ok(DurableDecisionDrive::Transient(
                    "bounded decision workers are busy".to_owned(),
                ));
            }
            Err(crate::decision_dispatch::DecisionDispatchReserveError::Unavailable { .. }) => {
                let trigger = engine
                    .decision_fallback_trigger_for_dispatch(candidate)
                    .map_err(|error| error.to_string())?;
                self.retry = None;
                return match stage_decision_fallback_trigger(
                    engine,
                    store,
                    &trigger,
                    self.idempotency_epoch,
                )
                .await?
                {
                    Some(staged) => Ok(DurableDecisionDrive::StagedLifecycle(staged)),
                    None => Ok(DurableDecisionDrive::Transient(
                        "provider-unavailable fallback transfer already has durable custody"
                            .to_owned(),
                    )),
                };
            }
        };

        for _ in 0..3 {
            let ingress = ActionCoordinator::decision_lease_ingress(
                engine,
                candidate,
                self.idempotency_epoch,
            )
            .map_err(|error| error.to_string())?;
            let preflight = ActionCoordinator::preflight(store, ingress)
                .await
                .map_err(|error| error.to_string())?;
            match preflight {
                ActionPreflight::Retired {
                    minimum_accepted_epoch,
                } => {
                    self.idempotency_epoch = self.idempotency_epoch.max(minimum_accepted_epoch);
                }
                ActionPreflight::Vacant(permit) => {
                    let staged = match ActionCoordinator::prepare_and_stage_decision_lease(
                        permit, engine, candidate,
                    ) {
                        Ok(staged) => staged,
                        Err(error) if decision_lease_stale(&error) => {
                            self.retry = None;
                            return Ok(DurableDecisionDrive::DiscardedStale(error.to_string()));
                        }
                        Err(error) => return Err(error.to_string()),
                    };
                    self.retry = None;
                    return Ok(DurableDecisionDrive::Staged(Box::new(
                        StagedDecisionDispatch {
                            staged,
                            reservation,
                        },
                    )));
                }
                ActionPreflight::Pending { .. } => {
                    return Err(
                        "decision lease rediscovered an unresolved intent after writable boot"
                            .to_owned(),
                    );
                }
                ActionPreflight::Finalized(_) => {
                    return Err(
                        "decision remained dispatchable after its lease action finalized"
                            .to_owned(),
                    );
                }
                ActionPreflight::Quarantined(_) => {
                    return Err("decision lease action is quarantined".to_owned());
                }
                ActionPreflight::Conflict => {
                    return Err("decision lease idempotency identity conflicts".to_owned());
                }
            }
        }
        Ok(DurableDecisionDrive::Transient(
            "decision lease retirement watermark changed repeatedly".to_owned(),
        ))
    }

    fn take_failure_owner(&mut self) -> Option<mush_core::OwnerId> {
        self.failure_owner.take()
    }
}

struct StagedDecisionDispatch {
    staged: StagedDecisionLease,
    reservation: DecisionDispatchReservation,
}

enum DurableDecisionDrive {
    Idle,
    DeferredByLegacyPredecessor,
    RecoveryAuditProgress,
    DiscardedStale(String),
    Transient(String),
    Staged(Box<StagedDecisionDispatch>),
    StagedLifecycle(StagedAuthoritativeAction),
    StagedFallback(StagedDecisionResponse),
    StagedLifecycleWithTerminal {
        staged: StagedAuthoritativeAction,
        fence: mush_core::DecisionCompletionFence,
        binding: mush_core::DecisionActionBinding,
        outcome: mush_core::DecisionTerminalOutcome,
    },
}

fn decision_lease_stale(error: &crate::action_coordinator::ActionCoordinatorError) -> bool {
    matches!(
        error,
        crate::action_coordinator::ActionCoordinatorError::DecisionLeasePlan(source)
            if matches!(
                source.as_ref(),
                mush_core::DecisionSourcePlanError::StaleCandidate
                    | mush_core::DecisionSourcePlanError::MissingDecision
                    | mush_core::DecisionSourcePlanError::MissingPrompt
                    | mush_core::DecisionSourcePlanError::Decision(
                        mush_core::DecisionError::StaleRevision
                            | mush_core::DecisionError::StaleAttempt
                            | mush_core::DecisionError::StaleLease
                            | mush_core::DecisionError::InvalidTransition
                            | mush_core::DecisionError::DeadlineReached
                    )
            )
    )
}

/// Stage the bounded owner-local lifecycle follow-up after the selected
/// ordinary action's exact receipt has installed. Returning `None` means the
/// identical lifecycle key already has durable custody or a final receipt.
async fn stage_decision_resolving(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    fence: &mush_core::DecisionCompletionFence,
    binding: &mush_core::DecisionActionBinding,
    outcome: mush_core::DecisionTerminalOutcome,
    mut idempotency_epoch: u64,
) -> Result<Option<StagedAuthoritativeAction>, String> {
    for _ in 0..3 {
        let ingress = ActionCoordinator::decision_resolving_ingress(
            engine,
            fence,
            binding,
            outcome,
            idempotency_epoch,
        )
        .map_err(|error| error.to_string())?;
        match ActionCoordinator::preflight(store, ingress)
            .await
            .map_err(|error| error.to_string())?
        {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => idempotency_epoch = idempotency_epoch.max(minimum_accepted_epoch),
            ActionPreflight::Vacant(permit) => {
                return ActionCoordinator::prepare_and_stage_decision_resolving(
                    permit, engine, fence, binding, outcome,
                )
                .map(Some)
                .map_err(|error| error.to_string());
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => return Ok(None),
            ActionPreflight::Quarantined(receipt) => {
                return Err(format!(
                    "decision resolving action {} is quarantined",
                    receipt.token()
                ));
            }
            ActionPreflight::Conflict => {
                return Err("decision resolving idempotency identity conflicts".to_owned());
            }
        }
    }
    Err("decision resolving retirement watermark changed repeatedly".to_owned())
}

/// Stage an exact owner-local terminal transition after the resolving binding
/// is durable. Recovery may safely join the same deterministic lifecycle key.
async fn stage_decision_terminal(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    fence: &mush_core::DecisionCompletionFence,
    binding: &mush_core::DecisionActionBinding,
    outcome: mush_core::DecisionTerminalOutcome,
    mut idempotency_epoch: u64,
) -> Result<Option<StagedAuthoritativeAction>, String> {
    for _ in 0..3 {
        let ingress = ActionCoordinator::decision_terminal_ingress(
            engine,
            fence,
            binding,
            outcome,
            idempotency_epoch,
        )
        .map_err(|error| error.to_string())?;
        match ActionCoordinator::preflight(store, ingress)
            .await
            .map_err(|error| error.to_string())?
        {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => idempotency_epoch = idempotency_epoch.max(minimum_accepted_epoch),
            ActionPreflight::Vacant(permit) => {
                return ActionCoordinator::prepare_and_stage_decision_terminal(
                    permit, engine, fence, binding, outcome,
                )
                .map(Some)
                .map_err(|error| error.to_string());
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => return Ok(None),
            ActionPreflight::Quarantined(receipt) => {
                return Err(format!(
                    "decision terminal action {} is quarantined",
                    receipt.token()
                ));
            }
            ActionPreflight::Conflict => {
                return Err("decision terminal idempotency identity conflicts".to_owned());
            }
        }
    }
    Err("decision terminal retirement watermark changed repeatedly".to_owned())
}

/// Stage the exact terminal-to-receipt replacement. The kernel re-reads the
/// terminal, prompt, deadline, and composition identity during preparation;
/// selected-action durability was established by the caller immediately
/// before entering this helper.
async fn stage_decision_compaction(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    candidate: &mush_core::DecisionCompactionCandidate,
    mut idempotency_epoch: u64,
) -> Result<Option<StagedAuthoritativeAction>, String> {
    for _ in 0..3 {
        let ingress =
            ActionCoordinator::decision_compaction_ingress(engine, candidate, idempotency_epoch)
                .map_err(|error| error.to_string())?;
        match ActionCoordinator::preflight(store, ingress)
            .await
            .map_err(|error| error.to_string())?
        {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => idempotency_epoch = idempotency_epoch.max(minimum_accepted_epoch),
            ActionPreflight::Vacant(permit) => {
                return ActionCoordinator::prepare_and_stage_decision_compaction(
                    permit, engine, candidate,
                )
                .map(Some)
                .map_err(|error| error.to_string());
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => return Ok(None),
            ActionPreflight::Quarantined(receipt) => {
                return Err(format!(
                    "decision compaction action {} is quarantined",
                    receipt.token()
                ));
            }
            ActionPreflight::Conflict => {
                return Err("decision compaction idempotency identity conflicts".to_owned());
            }
        }
    }
    Err("decision compaction retirement watermark changed repeatedly".to_owned())
}

async fn stage_decision_attempt(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    fence: &mush_core::DecisionCompletionFence,
    kind: mush_core::DecisionAttemptFailureKind,
    mut idempotency_epoch: u64,
) -> Result<Option<StagedDecisionAttempt>, String> {
    for _ in 0..3 {
        let ingress = ActionCoordinator::decision_attempt_ingress(
            engine,
            fence,
            kind,
            mush_core::DEFAULT_DECISION_ATTEMPTS,
            idempotency_epoch,
        )
        .map_err(|error| error.to_string())?;
        match ActionCoordinator::preflight(store, ingress)
            .await
            .map_err(|error| error.to_string())?
        {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => idempotency_epoch = idempotency_epoch.max(minimum_accepted_epoch),
            ActionPreflight::Vacant(permit) => {
                return ActionCoordinator::prepare_and_stage_decision_attempt(
                    permit,
                    engine,
                    fence,
                    kind,
                    mush_core::DEFAULT_DECISION_ATTEMPTS,
                )
                .map(Some)
                .map_err(|error| error.to_string());
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => return Ok(None),
            ActionPreflight::Quarantined(receipt) => {
                return Err(format!(
                    "decision attempt action {} is quarantined",
                    receipt.token()
                ));
            }
            ActionPreflight::Conflict => {
                return Err("decision attempt idempotency identity conflicts".to_owned());
            }
        }
    }
    Err("decision attempt retirement watermark changed repeatedly".to_owned())
}

async fn stage_decision_fallback_trigger(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    candidate: &mush_core::DecisionFallbackTriggerCandidate,
    mut idempotency_epoch: u64,
) -> Result<Option<StagedAuthoritativeAction>, String> {
    for _ in 0..3 {
        let ingress = ActionCoordinator::decision_fallback_trigger_ingress(
            engine,
            candidate,
            idempotency_epoch,
        )
        .map_err(|error| error.to_string())?;
        match ActionCoordinator::preflight(store, ingress)
            .await
            .map_err(|error| error.to_string())?
        {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => idempotency_epoch = idempotency_epoch.max(minimum_accepted_epoch),
            ActionPreflight::Vacant(permit) => {
                return ActionCoordinator::prepare_and_stage_decision_fallback_trigger(
                    permit, engine, candidate,
                )
                .map(|staged| Some(staged.staged))
                .map_err(|error| error.to_string());
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => return Ok(None),
            ActionPreflight::Quarantined(receipt) => {
                return Err(format!(
                    "decision fallback trigger action {} is quarantined",
                    receipt.token()
                ));
            }
            ActionPreflight::Conflict => {
                return Err("decision fallback trigger idempotency identity conflicts".to_owned());
            }
        }
    }
    Err("decision fallback trigger retirement watermark changed repeatedly".to_owned())
}

mod submission_disposition;

// ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----

/// The five steps every authoritative offer execution takes inside the world
/// single-writer: recover the actor from the permit, build its legal-action
/// query, project **that actor's own** offers, resolve the submitted opaque
/// offer against them, and hand the resolved offer to
/// [`ActionCoordinator::prepare_and_stage_resolved`].
///
/// Both ingresses call exactly this: the gateway's
/// [`WorldMsg::AuthoritativeAction`] arm and the command adapter's
/// [`CommandSubmitDriver`]. Forking a second spelling is the failure mode B1
/// exists to prevent, and a test pins that both call this one function.
fn resolve_and_stage_offer(
    engine: &mut Engine,
    permit: Box<ActionPermit>,
    submission: OfferSubmission,
) -> Result<(StagedAuthoritativeAction, ObjId), String> {
    let principal = permit.ingress().principal().clone();
    let PrincipalId::Object { id: actor } = principal else {
        return Err("coordinator principal is not an object actor".to_owned());
    };
    let query = legal_action_query(
        engine,
        PrincipalId::Object { id: actor },
        actor,
        submission.subject.clone(),
    )
    .map_err(|error| error.to_string())?;
    let offers = engine
        .project_legal_actions(&query)
        .map_err(|error| error.to_string())?;
    let resolved = engine
        .resolve_legal_action(
            &offers,
            OfferExecutionContext {
                request: permit.ingress().request(),
                principal: PrincipalId::Object { id: actor },
                actor,
                idempotency: permit.ingress().idempotency().clone(),
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            submission,
        )
        .map_err(|error| error.to_string())?;
    ActionCoordinator::prepare_and_stage_resolved(permit, engine, resolved)
        .map(|staged| (staged, actor))
        .map_err(|error| error.to_string())
}

/// Turn a command handler's **selector** (an action definition plus the subject
/// it expects) into the actor's own opaque offer.
///
/// This is the whole reason `Ctx::submit_authoritative` cannot be an offer: the
/// handler never sees an `ActionOfferId`, and the id used here is the one this
/// actor's projection just minted. A subject the handler did not expect is a
/// refusal, never a silent re-target.
fn command_offer_submission(
    engine: &Engine,
    submit: &mush_core::AuthoritativeSubmit,
) -> Result<OfferSubmission, String> {
    let query = legal_action_query(
        engine,
        PrincipalId::Object { id: submit.actor },
        submit.actor,
        submit.subject.clone(),
    )
    .map_err(|error| error.to_string())?;
    let offers = engine
        .project_legal_actions(&query)
        .map_err(|error| error.to_string())?;
    let projected = offers
        .projected_for_action(&submit.action)
        .ok_or_else(|| "that action is not currently available".to_owned())?;
    if projected.subject() != &submit.subject {
        return Err("that action's subject is not the one the command named".to_owned());
    }
    Ok(OfferSubmission {
        offer: projected.offer(),
        subject: projected.subject().clone(),
        targets: submit.targets.clone(),
        parameters: submit.parameters.clone(),
    })
}

/// Drives the one command-originated authoritative submission an
/// [`Engine::step`] may have queued.
///
/// Shaped on [`DurableScheduleDriver`] deliberately: **at most one per step**,
/// deferred while legacy persistence work is pending (a prepared action may
/// H2a (D-H-6): turn one queued decision capture into the owner's
/// capture-action submission. The kernel builds the complete plan (offers,
/// witnesses, signed prompt, admission, deadline); the plan is armed in the
/// owner's registered sink; a synthesized [`mush_core::AuthoritativeSubmit`] for the
/// owner's capture action is queued in the same one-per-backend slot every
/// command submit uses, and `CommandSubmitDriver::drive_one` carries it
/// through `resolve_and_stage_offer` on this same iteration. Every refusal
/// leaves no durable intent and disarms the sink.
fn drive_decision_capture(
    engine: &mut Engine,
    capture: mush_core::QueuedDecisionCapture,
    idempotency_epoch: u64,
    out: &mut Outbox,
) {
    let actor = capture.draft.actor;
    let owner = capture.owner.clone();
    let Some((capture_action, sink)) = engine.registry.decision_capture_sink(owner.as_str()) else {
        tracing::warn!(%owner, %actor, "decision capture refused: the owner registered no capture sink");
        out.best_effort_error(
            actor,
            "That decision could not be captured. Nothing changed.",
        );
        return;
    };
    let plan = match engine.build_decision_capture(&owner, &capture.draft, idempotency_epoch) {
        Ok(plan) => plan,
        Err(error) => {
            tracing::info!(%owner, %actor, error = %error, "decision capture refused before any durable intent");
            out.best_effort_error(
                actor,
                "That decision could not be captured. Nothing changed.",
            );
            return;
        }
    };
    if sink.arm(plan).is_some() {
        tracing::warn!(%owner, %actor, "a previously armed decision capture plan was never staged; replaced");
    }
    let submit = Box::new(mush_core::AuthoritativeSubmit {
        actor,
        action: capture_action,
        subject: capture.draft.subject.clone(),
        targets: Vec::new(),
        parameters: serde_json::json!({}),
        key: capture.draft.key.clone(),
    });
    if engine.queue_authoritative_submit(submit).is_err() {
        // One unresolved authoritative intent per backend (D-E7-39): the
        // slot is taken, so nothing is staged and the sink is cleared.
        let _ = sink.take();
        tracing::info!(%owner, %actor, "decision capture deferred: an authoritative submission already holds the slot");
        out.best_effort_error(
            actor,
            "An earlier action is still committing; the decision was not captured.",
        );
    }
}

/// never ride alongside it), and epoch retirement recovered by raising the
/// watermark and retrying a bounded number of times.
struct CommandSubmitDriver {
    idempotency_epoch: u64,
    runtime_installed: bool,
    retry_after: Option<tokio::time::Instant>,
    // Impossible queue-restoration failure is an integrity stop, not evidence
    // of absence. Preserve the exact handoff even while the loop freezes.
    stranded: Option<mush_core::submission_observation::DrivenSubmission>,
}

/// What one command-ingress drive did. Deferred arms retain the same queued
/// submission; only the separate observation can prove exact intent absence.
enum CommandSubmitDrive {
    Idle,
    DeferredByLegacyPredecessor,
    /// Same observed attempt retained before staging, waiting for its timer.
    DeferredByPreflight,
    /// Presentation refusal. This also includes indeterminate/integrity cases;
    /// only the separate private observation can prove no durable intent.
    Refused {
        actor: ObjId,
        detail: String,
        /// Whether this refusal is a **durable-integrity** event rather than an
        /// ordinary "that action is not currently available" (E7c2 fold R8).
        ///
        /// `ActionPreflight::Quarantined` (*the request was consumed by a
        /// never-finalized intent whose complete recovery material is in
        /// quarantine*), `ActionPreflight::Conflict` (*the scoped key is bound
        /// to different canonical ingress*) and retirement exhaustion used to
        /// collapse into the same arm and the same `tracing::info!` as a typo,
        /// which left a quarantined authoritative intent on the command lane
        /// indistinguishable in the log from a mistyped verb. The actor's line
        /// is unchanged either way — that is the disclosure rule.
        integrity: bool,
    },
    /// The same request already owns a durable, not-yet-finalized intent.
    AlreadyOwned {
        actor: ObjId,
    },
    /// The same request already committed: the actor's confirmation is
    /// projected from the durable receipt, and no provider or RNG runs again.
    AlreadyFinished {
        actor: ObjId,
        receipt: Box<AuthoritativeReceipt>,
        status: mush_proto::ActionStatusId,
    },
    Staged {
        staged: StagedAuthoritativeAction,
        actor: ObjId,
        disposition: Option<submission_disposition::StagedAttempt>,
    },
}

impl CommandSubmitDriver {
    const fn new(idempotency_epoch: u64) -> Self {
        Self {
            idempotency_epoch,
            runtime_installed: false,
            retry_after: None,
            stranded: None,
        }
    }

    async fn drive_one(
        &mut self,
        engine: &mut Engine,
        store: &dyn AuthoritativeStore,
    ) -> Result<CommandSubmitDrive, String> {
        if self.stranded.is_some() {
            return Err("observed command handoff could not be restored".into());
        }
        if !engine.has_authoritative_submit() {
            self.retry_after = None;
            return Ok(CommandSubmitDrive::Idle);
        }
        if self
            .retry_after
            .is_some_and(|deadline| tokio::time::Instant::now() < deadline)
        {
            return Ok(CommandSubmitDrive::DeferredByPreflight);
        }
        self.retry_after = None;
        // Same rule as `DurableScheduleDriver::drive_one`: legacy work owns the
        // next FIFO commit, and `flush_prepared_action` rejects a prepared
        // action that reaches submission beside it. The submission stays queued.
        if engine.has_legacy_persistence_work() {
            return Ok(CommandSubmitDrive::DeferredByLegacyPredecessor);
        }
        if !self.runtime_installed {
            engine
                .set_submission_runtime(Arc::new(submission_disposition::Runtime))
                .map_err(str::to_owned)?;
            self.runtime_installed = true;
        }
        let Some(driven) = engine.take_observed_authoritative_submit() else {
            return Ok(CommandSubmitDrive::Idle);
        };
        // Keep the opaque handoff intact until preflight gives a definitive
        // branch. A transient lookup may only restore this same attempt.
        let submit = driven.request();
        let actor = submit.actor;
        for _ in 0..3 {
            let envelope = match ActionCoordinator::command_ingress(submit, self.idempotency_epoch)
            {
                Ok(envelope) => envelope,
                Err(detail) => {
                    return Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail,
                        integrity: false,
                    });
                }
            };
            let status = match ActionCoordinator::status_id(envelope.idempotency()) {
                Ok(status) => status,
                Err(error) => {
                    return Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail: error.to_string(),
                        integrity: false,
                    });
                }
            };
            let preflight = match ActionCoordinator::preflight(store, envelope).await {
                Ok(preflight) => preflight,
                // No provider or writer has seen this attempt. Restore the
                // same opaque observation before relinquishing the Engine;
                // missing acknowledgement is never interpreted as absence.
                Err(error) if error.is_transient() => {
                    if driven.is_observed() {
                        if let Err(driven) = engine.requeue_observed_authoritative_submit(driven) {
                            self.stranded = Some(driven);
                            return Err("observed command handoff could not be restored".into());
                        }
                        self.retry_after =
                            Some(tokio::time::Instant::now() + COMMAND_PREFLIGHT_RETRY_DELAY);
                        return Ok(CommandSubmitDrive::DeferredByPreflight);
                    }
                    return Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail: error.to_string(),
                        integrity: false,
                    });
                }
                Err(error) => return Err(error.to_string()),
            };
            match preflight {
                ActionPreflight::Retired {
                    minimum_accepted_epoch,
                } => {
                    self.idempotency_epoch = self.idempotency_epoch.max(minimum_accepted_epoch);
                }
                ActionPreflight::Vacant(permit) => {
                    let (submit, driver) = driven.into_parts();
                    let observation = submission_disposition::Attempt::from_driver(driver)
                        .map(|attempt| attempt.vacant(&permit))
                        .transpose()?;
                    let submission = match command_offer_submission(engine, &submit) {
                        Ok(submission) => submission,
                        Err(detail) => {
                            if let Some(observation) = observation {
                                observation.refuse_unstaged(engine);
                            }
                            return Ok(CommandSubmitDrive::Refused {
                                actor,
                                detail,
                                integrity: false,
                            });
                        }
                    };
                    return Ok(match resolve_and_stage_offer(engine, permit, submission) {
                        Ok((staged, actor)) => {
                            let disposition = observation.map(|proof| proof.staged(&staged));
                            CommandSubmitDrive::Staged {
                                staged,
                                actor,
                                disposition,
                            }
                        }
                        Err(detail) => {
                            if let Some(observation) = observation {
                                observation.refuse_unstaged(engine);
                            }
                            CommandSubmitDrive::Refused {
                                actor,
                                detail,
                                integrity: false,
                            }
                        }
                    });
                }
                ActionPreflight::Pending { .. } => {
                    return Ok(CommandSubmitDrive::AlreadyOwned { actor });
                }
                ActionPreflight::Finalized(receipt) => {
                    return Ok(CommandSubmitDrive::AlreadyFinished {
                        actor,
                        receipt,
                        status,
                    });
                }
                ActionPreflight::Quarantined(receipt) => {
                    return Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail: format!(
                            "that request is quarantined and will not be replanned (spool {})",
                            receipt.token()
                        ),
                        integrity: true,
                    });
                }
                ActionPreflight::Conflict => {
                    return Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail: "that request identity conflicts with different ingress".to_owned(),
                        integrity: true,
                    });
                }
            }
        }
        Ok(CommandSubmitDrive::Refused {
            actor,
            detail: "the idempotency retirement watermark changed repeatedly".to_owned(),
            integrity: true,
        })
    }
}

// Retained preflight work occupies the existing queue budget. A timer rather
// than a ready future prevents a store outage from spinning the world loop.
const COMMAND_PREFLIGHT_RETRY_DELAY: Duration = Duration::from_millis(250);

async fn command_submit_ready(retry_after: Option<tokio::time::Instant>) {
    if let Some(deadline) = retry_after {
        tokio::time::sleep_until(deadline).await;
    }
}

struct DurableScheduleDriver {
    cursor: Option<mush_core::ScheduleSourceCursor>,
    retry: Option<mush_core::ScheduledActionCandidate>,
    cause: mush_core::ScheduleDriveCause,
    idempotency_epoch: u64,
}

impl DurableScheduleDriver {
    const fn new(idempotency_epoch: u64) -> Self {
        Self {
            cursor: None,
            retry: None,
            cause: mush_core::ScheduleDriveCause::Restart,
            idempotency_epoch,
        }
    }

    async fn drive_one(
        &mut self,
        engine: &mut Engine,
        store: &dyn AuthoritativeStore,
    ) -> Result<DurableScheduleDrive, String> {
        // The world loop deliberately gives legacy work the next FIFO commit
        // rather than allowing an internal schedule to collide with it.  The
        // current wake will fall through to the ordinary flush; the retained
        // candidate/cursor is untouched and will be retried on the next poll.
        if engine.has_legacy_persistence_work() {
            return Ok(DurableScheduleDrive::DeferredByLegacyPredecessor);
        }
        if self.retry.is_none() {
            let page = match engine.discover_scheduled_actions(self.cause, self.cursor.as_ref(), 1)
            {
                Ok(page) => page,
                Err(error) if schedule_discovery_retryable(&error) => {
                    return Ok(DurableScheduleDrive::Transient(error.to_string()));
                }
                Err(error) => return Err(error.to_string()),
            };
            self.cursor = page.next_cursor().cloned();
            if page.records_scanned() == 0 && self.cursor.is_none() {
                self.cause = mush_core::ScheduleDriveCause::Live;
            }
            self.retry = page.candidates().first().cloned();
        }
        let Some(candidate) = self.retry.as_ref() else {
            return Ok(DurableScheduleDrive::Idle);
        };

        // Epoch retirement is recoverable for internal deterministic work:
        // no binding can exist below the durable watermark, so rebuild the
        // same semantic key at the returned minimum and preflight again.
        for _ in 0..3 {
            let ingress = ActionCoordinator::scheduled_ingress(
                candidate.action(),
                candidate.advance(),
                self.idempotency_epoch,
            )
            .map_err(|error| error.to_string())?;
            let preflight = match ActionCoordinator::preflight(store, ingress).await {
                Ok(preflight) => preflight,
                Err(error) if error.is_transient() => {
                    return Ok(DurableScheduleDrive::Transient(error.to_string()));
                }
                Err(error) => return Err(error.to_string()),
            };
            match preflight {
                ActionPreflight::Retired {
                    minimum_accepted_epoch,
                } => {
                    self.idempotency_epoch = self.idempotency_epoch.max(minimum_accepted_epoch);
                }
                ActionPreflight::Vacant(permit) => {
                    let staged = match ActionCoordinator::prepare_and_stage_scheduled(
                        permit, engine, candidate,
                    ) {
                        Ok(staged) => staged,
                        Err(error) if scheduled_prepare_stale(&error) => {
                            self.retry = None;
                            return Ok(DurableScheduleDrive::DiscardedStale(error.to_string()));
                        }
                        Err(error) if scheduled_prepare_retryable(&error) => {
                            return Ok(DurableScheduleDrive::Transient(error.to_string()));
                        }
                        Err(error) => return Err(error.to_string()),
                    };
                    self.retry = None;
                    return Ok(DurableScheduleDrive::Staged(staged));
                }
                ActionPreflight::Pending { .. } => {
                    return Err(
                        "durable schedule rediscovered an unresolved intent after writable boot"
                            .to_owned(),
                    );
                }
                ActionPreflight::Finalized(_) => {
                    return Err(
                        "durable schedule pre-state remained live after its finalized action"
                            .to_owned(),
                    );
                }
                ActionPreflight::Quarantined(_) => {
                    return Err("durable schedule action is quarantined".to_owned());
                }
                ActionPreflight::Conflict => {
                    return Err("durable schedule idempotency identity conflicts".to_owned());
                }
            }
        }
        Ok(DurableScheduleDrive::Transient(
            "durable schedule retirement watermark changed repeatedly".to_owned(),
        ))
    }
}

enum DurableScheduleDrive {
    Idle,
    DeferredByLegacyPredecessor,
    DiscardedStale(String),
    Transient(String),
    Staged(StagedAuthoritativeAction),
}

/// Whether a failed schedule discovery is a transient the next poll may clear.
///
/// **E7 cut 8 independent verify fold T2/D4(a).** The scan itself now pauses a
/// single entry whose clock cannot answer
/// (`mush_core::schedule_scan_clock_is_skippable`), so this is the backstop for
/// the one shape it cannot skip per entry: a registered-provider clock that is
/// simply unavailable right now. It accepts the same skippable set, so a clock
/// stall on ANY lane is a `Transient` the driver retries rather than an `Err`
/// the caller turns into `apply_integrity_freeze`.
fn schedule_discovery_retryable(error: &mush_core::ScheduleSourceScanError) -> bool {
    let mush_core::ScheduleSourceScanError::Clock { source, .. } = error else {
        return false;
    };
    if mush_core::schedule_scan_clock_is_skippable(source.as_ref()) {
        return true;
    }
    matches!(
        source.as_ref(),
        mush_core::ClockReadError::Provider {
            source: mush_core::ClockProviderFailure::Unavailable,
            ..
        }
    )
}

fn scheduled_prepare_retryable(error: &crate::action_coordinator::ActionCoordinatorError) -> bool {
    match error {
        crate::action_coordinator::ActionCoordinatorError::SchedulePlan(source) => matches!(
            source.as_ref(),
            mush_core::ScheduleActionPlanError::Adapter { source, .. } if source.retryable()
        ),
        crate::action_coordinator::ActionCoordinatorError::Provider(
            mush_core::ActionProviderPrepareError::Provider { source, .. },
        ) => source.retryable(),
        _ => false,
    }
}

fn scheduled_prepare_stale(error: &crate::action_coordinator::ActionCoordinatorError) -> bool {
    match error {
        crate::action_coordinator::ActionCoordinatorError::SchedulePlan(source) => matches!(
            source.as_ref(),
            mush_core::ScheduleActionPlanError::Adapter { source, .. }
                if source.code() == mush_core::ActionErrorCode::Stale
        ),
        crate::action_coordinator::ActionCoordinatorError::Provider(
            mush_core::ActionProviderPrepareError::Provider { source, .. },
        ) => source.code() == mush_core::ActionErrorCode::Stale,
        crate::action_coordinator::ActionCoordinatorError::Provider(
            mush_core::ActionProviderPrepareError::Schedule(source),
        ) => matches!(
            source.as_ref(),
            mush_core::ScheduleAdvanceAdmissionError::MissingSchedule { .. }
                | mush_core::ScheduleAdvanceAdmissionError::StalePlan { .. }
        ),
        _ => false,
    }
}

enum PendingReceiptEvent {
    Owned(Result<IntentReceipt, AuthoritativeCommitFailure>),
    Final(Result<(), AuthoritativeCommitFailure>),
}

type ClassifiedSend = (DeliveryClass, SendTarget, Arc<OutputEvent>);

struct CreateCompletion {
    id: ObjId,
    reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
}

struct ActionCompletion {
    staged: StagedAuthoritativeAction,
    disposition: Option<submission_disposition::StagedAttempt>,
    owned: Option<oneshot::Sender<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>>,
    finished:
        Option<oneshot::Sender<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>>,
    ownership_valid: bool,
    post_commit_decision: Option<DecisionPostCommit>,
    post_commit_decision_response: Option<DecisionResponsePostCommit>,
    post_commit_decision_terminal: Option<DecisionTerminalPostCommit>,
}

struct DecisionPostCommit {
    reservation: DecisionDispatchReservation,
    worker: mush_core::DecisionWorkerLease,
}

struct DecisionResponsePostCommit {
    fence: mush_core::DecisionCompletionFence,
    binding: mush_core::DecisionActionBinding,
    outcome: mush_core::DecisionTerminalOutcome,
}

struct DecisionTerminalPostCommit {
    fence: mush_core::DecisionCompletionFence,
    binding: mush_core::DecisionActionBinding,
    outcome: mush_core::DecisionTerminalOutcome,
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
    action: Option<ActionCompletion>,
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
        ownership: Option<oneshot::Receiver<Result<IntentReceipt, AuthoritativeCommitFailure>>>,
        receiver: oneshot::Receiver<Result<AuthoritativeReceipt, AuthoritativeCommitFailure>>,
    },
}

impl PendingReceipt {
    const fn authoritative_identity(
        &self,
    ) -> Option<(mush_core::CommitToken, mush_core::CanonicalHash)> {
        match self {
            Self::Authoritative {
                expected_token,
                expected_hash,
                ..
            } => Some((*expected_token, *expected_hash)),
            Self::Legacy { .. } => None,
        }
    }

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
                ..
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

    async fn receive_ownership(&mut self) -> Result<IntentReceipt, AuthoritativeCommitFailure> {
        match self {
            Self::Legacy { .. } => std::future::pending().await,
            Self::Authoritative { ownership, .. } => {
                let Some(receiver) = ownership.take() else {
                    return std::future::pending().await;
                };
                receiver.await.unwrap_or_else(|_| {
                    Err(AuthoritativeCommitFailure::Integrity(
                        "store writer dropped the authoritative ownership receipt".to_owned(),
                    ))
                })
            }
        }
    }

    const fn expects_ownership(&self) -> bool {
        matches!(
            self,
            Self::Authoritative {
                ownership: Some(_),
                ..
            }
        )
    }

    async fn receive_next(&mut self) -> PendingReceiptEvent {
        if self.expects_ownership() {
            PendingReceiptEvent::Owned(self.receive_ownership().await)
        } else {
            PendingReceiptEvent::Final(self.receive().await)
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
    decision_response: Option<DecisionResponsePostCommit>,
    decision_terminal: Option<DecisionTerminalPostCommit>,
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

/// Publish the custody gauges. `queued_barriers` is the barrier waiters held in
/// the [`DeferQueue`] behind deferred lines (16b cut B.5): they wait on the same
/// commit a retained barrier waits on, one step further back, so the gauge
/// counts both and charges both the same fixed bytes.
fn publish_pending_custody(
    metrics: &Metrics,
    pending: Option<&PendingCommit>,
    queued_barriers: usize,
) {
    let custody = pending.map_or_else(PendingCustody::default, PendingCommit::custody);
    let as_u64 = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
    metrics.set_commit_pending_custody(
        as_u64(custody.barriers.saturating_add(queued_barriers)),
        as_u64(custody.reattach_records),
        as_u64(
            custody
                .operational_bytes
                .saturating_add(queued_barriers.saturating_mul(PENDING_BARRIER_CHARGE_BYTES)),
        ),
        as_u64(custody.sends),
        as_u64(custody.effects),
    );
}

/// Answer a barrier that has nothing queued ahead of it: behind the pending
/// commit's exact receipt when there is one (processing is not delivery), at
/// once otherwise. A retained waiter past [`MAX_PENDING_BARRIERS`] is refused
/// `Busy` by [`PendingCommit::retain_barrier`].
fn settle_barrier(
    reply: oneshot::Sender<Result<(), WorldReadError>>,
    pending: Option<&mut PendingCommit>,
    metrics: &Metrics,
) {
    if let Some(pending) = pending {
        // Processing is not delivery: keep the barrier behind the prior exact
        // receipt and held output.
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
}

/// The [`DeferQueue`]'s clock: tokio's, the one its timer arm sleeps on. In
/// production it reads the same as `std::time::Instant`; under a paused test
/// runtime the stamp, the deadline and the expiry all move with the virtual
/// clock together, so an elapsed deadline always finds its entry aged (a
/// `std` stamp against a virtual sleep re-armed an elapsed deadline and spun
/// until real time caught up).
type DeferInstant = tokio::time::Instant;

/// **One entry of the [`DeferQueue`]**: what waits, and since when (16b cut
/// B.5). `since` is the instant the entry first joined the queue, and it
/// survives a return to the front ([`DeferQueue::push_front_kept`]), so the
/// [`COMMAND_DEFER_MS`] bound is always measured from the player's keystroke.
struct Deferred {
    since: DeferInstant,
    entry: DeferEntry,
}

/// What a [`Deferred`] holds.
enum DeferEntry {
    /// A typed line, a query or a reply-carrying world read
    /// ([`waits_in_defer_queue`]), refused `CommitPending` on arrival.
    Msg(WorldMsg),
    /// A [`WorldMsg::Barrier`] that arrived behind queued entries. It is
    /// bookkeeping, so it never expires; it completes when it reaches the front
    /// with no commit pending, which is when everything sent before it has been
    /// served (or refused).
    Barrier(oneshot::Sender<Result<(), WorldReadError>>),
}

/// **The bounded defer queue** (16b cut B.5, `B-13`; doc 09 "single-flight").
///
/// While one exact commit is in flight the world cannot serve a line or a read
/// (RAM holds the tentative post-image), and it used to refuse every one at
/// once — four web lines sent in one burst lost three, and every test rig in the
/// tree carried a retry loop a player does not have. This holds them instead:
/// at most [`COMMAND_DEFER_DEPTH`] typed lines a connection and
/// [`COMMAND_DEFER_CAP`] entries in all, each for at most [`COMMAND_DEFER_MS`]
/// from its arrival. The loop drains it front to back once no commit is pending,
/// one entry a turn and before anything new is taken, so a connection's lines
/// stay in the order it typed them.
///
/// Pure: no clock of its own (every call takes `now`), no I/O, no world.
#[derive(Default)]
struct DeferQueue {
    entries: VecDeque<Deferred>,
}

impl DeferQueue {
    /// Queue a message refused `CommitPending`, or hand it back when its
    /// connection already has [`COMMAND_DEFER_DEPTH`] typed lines waiting (a
    /// typed line only) or the queue holds [`COMMAND_DEFER_CAP`] entries.
    #[allow(clippy::result_large_err)] // Refusal returns the exact inbound message without allocating on a full queue.
    fn push_msg(&mut self, msg: WorldMsg, now: DeferInstant) -> Result<(), WorldMsg> {
        if self.entries.len() >= COMMAND_DEFER_CAP {
            return Err(msg);
        }
        if let WorldMsg::Inbound(Inbound::Cmd { conn, .. }) = &msg {
            let depth = self
                .entries
                .iter()
                .filter(|held| {
                    matches!(
                        &held.entry,
                        DeferEntry::Msg(WorldMsg::Inbound(Inbound::Cmd { conn: queued, .. }))
                            if queued == conn
                    )
                })
                .count();
            if depth >= COMMAND_DEFER_DEPTH {
                return Err(msg);
            }
        }
        self.entries.push_back(Deferred {
            since: now,
            entry: DeferEntry::Msg(msg),
        });
        Ok(())
    }

    /// Queue a barrier behind every entry already waiting, or hand its reply
    /// back when the queue holds [`COMMAND_DEFER_CAP`] entries. It is never
    /// answered `Ok` here: the queue is only non-empty behind a pending commit
    /// (the loop polls `rx` with a non-empty queue only then), so a barrier
    /// answered at the cap would overtake both the queued entries and that
    /// commit's receipt. The caller refuses a handed-back reply with the
    /// retryable [`WorldReadError::Busy`] — the same overload answer
    /// [`PendingCommit::retain_barrier`] gives past [`MAX_PENDING_BARRIERS`].
    fn push_barrier(
        &mut self,
        reply: oneshot::Sender<Result<(), WorldReadError>>,
        now: DeferInstant,
    ) -> Result<(), oneshot::Sender<Result<(), WorldReadError>>> {
        if self.entries.len() >= COMMAND_DEFER_CAP {
            return Err(reply);
        }
        self.entries.push_back(Deferred {
            since: now,
            entry: DeferEntry::Barrier(reply),
        });
        Ok(())
    }

    /// Put a just-popped entry back at the FRONT with its original `since`: a
    /// drained line that met a commit again (an earlier drained line started
    /// one) keeps its place and its deadline.
    fn push_front_kept(&mut self, deferred: Deferred) {
        self.entries.push_front(deferred);
    }

    /// Take every message that has waited [`COMMAND_DEFER_MS`] or longer, front
    /// to back. A barrier never expires and keeps its place among the rest.
    fn expire(&mut self, now: DeferInstant) -> Vec<WorldMsg> {
        let mut expired = Vec::new();
        let mut kept = VecDeque::with_capacity(self.entries.len());
        for held in self.entries.drain(..) {
            match held.entry {
                DeferEntry::Msg(msg)
                    if now.saturating_duration_since(held.since) >= COMMAND_DEFER =>
                {
                    expired.push(msg);
                }
                entry => kept.push_back(Deferred {
                    since: held.since,
                    entry,
                }),
            }
        }
        self.entries = kept;
        expired
    }

    /// When the next message ages out: the earliest `since + COMMAND_DEFER_MS`
    /// among the queued messages, or `None` when only barriers (or nothing)
    /// wait.
    fn oldest_deadline(&self) -> Option<DeferInstant> {
        self.entries
            .iter()
            .filter(|held| matches!(held.entry, DeferEntry::Msg(_)))
            .map(|held| held.since.checked_add(COMMAND_DEFER).unwrap_or(held.since))
            .min()
    }

    fn pop_front(&mut self) -> Option<Deferred> {
        self.entries.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Everything, front to back, leaving the queue empty (shutdown and the
    /// loop's exit).
    fn drain_all(&mut self) -> Vec<Deferred> {
        self.entries.drain(..).collect()
    }

    /// How many barrier waiters are queued (the custody gauge).
    fn barriers(&self) -> usize {
        self.entries
            .iter()
            .filter(|held| matches!(held.entry, DeferEntry::Barrier(_)))
            .count()
    }
}

#[allow(clippy::too_many_lines)] // the world loop is one cohesive state machine
#[allow(clippy::too_many_arguments)] // the world loop threads its whole context
async fn run(
    mut engine: Engine,
    mut rx: mpsc::Receiver<WorldMsg>,
    tx: mpsc::WeakSender<WorldMsg>,
    writer: StoreWriter,
    deps: ServerDeps,
    staff_ops: mpsc::Sender<StaffOp>,
    gateway: Gateway,
    high_water: Arc<AtomicU64>,
    mut signals: WriterSignalReceiver,
    mut shutdown: watch::Receiver<bool>,
    action_epoch: u64,
    persistence_integrity: PersistenceIntegrityLatch,
    world_exit: WorldExitStatus,
    decision_runtime: Option<DecisionRuntime>,
    initial_mode: Option<WorldMode>,
    _outbox_lifetime: watch::Sender<bool>,
    arenas: Option<tacsim::arena_bridge::ArenaTasks>,
) -> WorldOutcome {
    tracing::info!("world task running");
    // 16b cut T.2 (T4): the one receiver of every running match's egress. Taken
    // once, here, because the loop is its only legitimate owner — a second taker
    // would silently split the stream.
    let mut arena_egress = arenas
        .as_ref()
        .and_then(tacsim::arena_bridge::ArenaTasks::take_egress);
    let boot_unix = unix_now_s();
    // The LLM service handle (built from [llm] config) that drives a module's
    // deferred async work off-loop; cloned out of `deps` so the loop keeps
    // borrowing `deps` (staff queue) freely each iteration.
    let llm = deps.llm.clone();
    let metrics = Arc::clone(&deps.metrics);
    let mut durable_schedules = DurableScheduleDriver::new(action_epoch);
    // E7c2 B1: the command-ingress adapter, sharing the same process epoch as
    // every other in-process action identity.
    let mut command_submits = CommandSubmitDriver::new(action_epoch);
    let mut durable_schedule_tick = tokio::time::interval(DURABLE_SCHEDULE_POLL_INTERVAL);
    durable_schedule_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut durable_schedule_transient_alarmed = false;
    let (mut durable_decisions, mut decision_wakes, mut decision_completions) =
        if let Some(runtime) = decision_runtime {
            (
                Some(DurableDecisionDriver::new(
                    action_epoch,
                    Arc::clone(&runtime.dispatcher),
                )),
                Some(runtime.wakes),
                Some(runtime.completions),
            )
        } else {
            (None, None, None)
        };
    let mut durable_decision_tick = tokio::time::interval(Duration::from_millis(250));
    durable_decision_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut durable_decision_transient_alarmed = false;
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
    // not served: RAM already contains the tentative post-image, so it is not
    // an authoritative query snapshot yet. They wait in the bounded defer
    // queue (16b cut B.5) and are refused with a typed Busy response only when
    // it is full or they outlive its 250 ms bound.
    let mut pending_commit: Option<PendingCommit> = None;
    // Loss witnesses already accounted for by the supervision check at the head
    // of the loop. Boot may hydrate a store that has none, so this starts where
    // the store starts rather than at zero.
    let mut lost_installations_seen = engine.action_state().lost_installations();
    // If a must-retain async completion or connection-lifecycle message is
    // already dequeued while another exact receipt is outstanding, retain that
    // ONE message locally and stop polling `rx`. This preserves FIFO without
    // inventing a second unbounded input queue; the bounded channel supplies
    // backpressure until the commit resolves. Nothing else occupies this slot:
    // CreatePlayer/Dump and typed service work is refused immediately, and a
    // typed line, a query or a reply-carrying world read waits in `defer`
    // below. Operational traffic behind a retained Apply/lifecycle message is
    // still head-of-line blocked: the separately bounded semantic/lifecycle
    // lane remains K2 work.
    let mut deferred_world_msg: Option<WorldMsg> = None;
    // **16b cut B.5 (`B-13`)**: the bounded defer queue. A line that meets a
    // commit waits here — at most four a connection, 64 in all, 250 ms from its
    // arrival — instead of being refused at once, and `rx` stays polled while
    // the commit is in flight, so nothing queued here blocks the receipt,
    // lifecycle or bookkeeping traffic behind it. Once the commit resolves the
    // queue drains, one entry a turn, BEFORE the lifecycle slot above and
    // before anything new is taken from `rx`: every queued entry arrived before
    // whatever fills that slot (`rx` stops once it is full), and a connection's
    // lines stay in the order it typed them.
    let mut defer = DeferQueue::default();
    // Store-backed staff operations drained beside a local operation remain
    // ordered behind any commit that local operation starts.
    let mut deferred_staff_ops: Vec<StaffOp> = Vec::new();
    // Whether the writer's control mailbox is still open (disabled once the
    // writer is gone and its final latched facts have been drained, so the
    // closed wake channel does not busy-loop the select).
    let mut signals_open = true;
    // Store health and exact commit receipts travel on separate bounded lanes.
    // A recovered health hint may therefore arrive first; retain it until the
    // corresponding pending commit is finalized so the public "writable
    // again" announcement is also a true admission guarantee.
    let mut recovery_waits_for_commit = false;
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
    // Set at the two clean breaks ONLY, and consumed by the very last statement
    // of this function. Everything else that can end the loop — the
    // channel-closed exit, the bounded shutdown grace, an unacknowledged,
    // refused or rejected shutdown write — leaves it false, and a panic never
    // reaches the statement at all. The shutdown coordinator refuses the final
    // BGSAVE unless it is set.
    let mut clean_exit = false;
    // Set by the panic firewall alone, and read by the very last statement of
    // this function. A faulted break never sets `clean_exit`, so a caught panic
    // can no more publish a clean exit than an uncaught one could.
    let mut fault: Option<WorldFault> = None;
    // Every SYNCHRONOUS engine entry below runs through this. On a caught panic
    // the loop records the fault, freezes persistence, discards the faulted
    // turn's residue, and breaks — it never re-enters the engine afterwards and
    // never releases the half-written output of the turn that faulted.
    macro_rules! guarded {
        ($site:expr, $call:expr) => {
            match guard($site, || $call) {
                Ok(value) => value,
                Err(caught) => {
                    fault = Some(recover_from_world_fault(
                        caught,
                        &mut engine,
                        &mut out,
                        &mut fx,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        &metrics,
                    ));
                    break;
                }
            }
        };
    }
    loop {
        // Supervision, before anything is admitted: a `PreparedInstallation`
        // that died without an install or an abort took the RAM half of an
        // ack-gated commit with it, and its durable half may already be in the
        // store. There is no answer to give the player and no state to answer
        // from, so the freeze is latched here and `admission_mode` below reads
        // it — every later line is refused rather than served from RAM that may
        // no longer match Redis.
        freeze_on_lost_installation(
            &engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &mut lost_installations_seen,
        );
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
        let durable_schedule_armed = engine
            .registry
            .schedule_sources()
            .is_some_and(|sources| !sources.is_empty())
            // A retained seal is an older durability predecessor. Let its
            // re-emit wake drain before admitting an exact-token action.
            && retained.is_empty()
            && admission_for(admission_mode, IngressClass::Timer) == Admission::Allow;
        let durable_decision_armed = durable_decisions.is_some()
            && retained.is_empty()
            && admission_for(admission_mode, IngressClass::Timer) == Admission::Allow;
        // E7c2 B1: arm the command-ingress adapter only when a prepared action
        // could legally be staged right now — no queued legacy persistence
        // work, no unresolved ledger predecessor, no pending commit.
        // A transient preflight retains the same queue entry and arms a timer;
        // other deferred work stays behind conditions the foot-of-loop clears.
        let command_submit_armed = (engine.has_authoritative_submit()
            || engine.has_decision_capture())
            && pending_commit.is_none()
            && retained.is_empty()
            && !engine.has_legacy_persistence_work()
            && admission_for(admission_mode, IngressClass::Timer) == Admission::Allow;
        // 16b cut T.2 (T4): acknowledge every match-ledger chunk the writer has
        // committed, on the E0 durable-commit high-water and nothing new. Read
        // once per iteration, before the select blocks, so an arena waiting on an
        // acknowledgement is never held behind an idle world.
        if let Some(arenas) = arenas.as_ref() {
            arenas.ack_ledger_upto(high_water.load(Ordering::Relaxed));
        }
        // The arena egress is ordinary module-write work, so it is admitted on the
        // same terms as a timer — never while a commit is pending, never while a
        // retained seal is unresolved, and never in a mode that cannot write.
        let arena_egress_armed = arena_egress.is_some()
            && pending_commit.is_none()
            && retained.is_empty()
            && !engine.has_legacy_persistence_work()
            && admission_for(admission_mode, IngressClass::Timer) == Admission::Allow;
        // 16b cut B.5 (`B-13`): the defer queue's two arms. The timer refuses
        // an entry that has waited its 250 ms while the commit still pends —
        // ONLY here, never on the turn that could serve it, so a line the
        // resolved commit can serve is served. The drain is ready work (the
        // `command_submit_armed` idiom): going through the select keeps the
        // signals, the shutdown watch and every other ready arm polled between
        // drained entries.
        let defer_deadline = defer.oldest_deadline();
        let defer_timer_armed = pending_commit.is_some() && defer_deadline.is_some();
        let defer_ready = pending_commit.is_none() && !defer.is_empty();
        // The lifecycle slot is taken only once the queue is empty: every
        // queued entry arrived before it (a `ConnDown` from `QUIT` runs after
        // the lines the player typed before it).
        let ready_deferred = if pending_commit.is_none() && defer.is_empty() {
            deferred_world_msg.take()
        } else {
            None
        };
        let wake = if let Some(msg) = ready_deferred {
            Wake::Msg(msg)
        } else {
            tokio::select! {
            // While a commit is in flight `rx` stays polled, so reads,
            // lifecycle traffic and bookkeeping are never head-of-line blocked
            // behind the queue; once it resolves the queue drains before
            // anything new is taken.
            m = rx.recv(), if inbound_open
                && deferred_world_msg.is_none()
                && (defer.is_empty() || pending_commit.is_some()) => {
                m.map_or(Wake::Closed, Wake::Msg)
            },
            () = async {
                match defer_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            }, if defer_timer_armed => Wake::DeferExpiry,
            () = std::future::ready(()), if defer_ready => Wake::Deferred,
            // Writer control facts ride their own bounded/coalescing mailbox
            // (see `WriterSignal`). `None` means the writer is gone and every
            // final latched fact was drained; disable this arm (avoiding a
            // busy-loop) rather than closing the world — the world's lifetime
            // is driven by its inbound channel, not this one.
            s = signals.recv(), if signals_open => match s {
                Some(sig) => Wake::Signal(sig),
                None => { signals_open = false; Wake::Timer }
            },
            receipt = async {
                match pending_commit.as_mut() {
                    Some(pending) => pending.receipt.receive_next().await,
                    None => std::future::pending().await,
                }
            } => Wake::CommitReceipt(receipt),
            () = sleep_until_unix_ms(next_due), if sched_armed => Wake::Timer,
            // New command submissions are immediately ready. Only a retained
            // transient preflight waits, without blocking other loop arms.
            () = command_submit_ready(command_submits.retry_after), if command_submit_armed => Wake::CommandSubmits,
            _ = durable_schedule_tick.tick(), if durable_schedule_armed => Wake::DurableSchedules,
            wake = async {
                match decision_wakes.as_mut() {
                    Some(wakes) => wakes.recv().await,
                    None => std::future::pending().await,
                }
            }, if durable_decision_armed => match wake {
                Some(wake) => Wake::DecisionNotification(wake),
                None => Wake::DurableDecisions,
            },
            completion = async {
                match decision_completions.as_mut() {
                    Some(completions) => completions.recv().await,
                    None => std::future::pending().await,
                }
            }, if durable_decisions.is_some() => match completion {
                Some(completion) => Wake::DecisionResult(Box::new(completion)),
                None => Wake::DurableDecisions,
            },
            _ = durable_decision_tick.tick(), if durable_decision_armed => Wake::DurableDecisions,
            // 16b cut T.2 (T4). Armed only while a game layer composed an arena
            // table and the world may write: an arena chunk is a module write, so
            // draining into a frozen world would build a batch nobody can commit
            // and then acknowledge it to the arena.
            egress = async {
                match arena_egress.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if arena_egress_armed => match egress {
                Some(out) => Wake::ArenaEgress(Box::new(out)),
                // Every sender is gone, which can only happen once every match's
                // task has ended. Disarm rather than spin.
                None => { arena_egress = None; Wake::Timer }
            },
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
        // 16b cut B.5: a drained line is handled EXACTLY as a fresh one —
        // admission and all — so it becomes a `Wake::Msg`, remembering when it
        // first queued in case it meets a commit again and must go back to the
        // front with its deadline. A drained barrier has its own arm.
        let mut drained_since: Option<DeferInstant> = None;
        let wake = match wake {
            Wake::Deferred => match defer.pop_front() {
                Some(Deferred {
                    since,
                    entry: DeferEntry::Msg(msg),
                }) => {
                    drained_since = Some(since);
                    Wake::Msg(msg)
                }
                Some(Deferred {
                    entry: DeferEntry::Barrier(reply),
                    ..
                }) => Wake::DeferredBarrier(reply),
                None => Wake::Deferred,
            },
            wake => wake,
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
        let mut action_completion: Option<ActionCompletion> = None;
        let mut failure_actor: Option<ObjId> = None;
        let outcome = match wake {
            Wake::Msg(msg) => {
                // 16b cut B.5: `command_processed` counts a typed line where it
                // is admitted or finally refused, never where it is deferred,
                // so a line that waits is counted once.
                match admit_world_msg(admission_mode, &engine.world, &msg) {
                    Admission::Refuse(reason) => {
                        if reason == RefusalReason::CommitPending && must_defer_behind_commit(&msg)
                        {
                            debug_assert!(deferred_world_msg.is_none());
                            deferred_world_msg = Some(msg);
                        } else if reason == RefusalReason::CommitPending
                            && waits_in_defer_queue(&msg)
                        {
                            match drained_since {
                                // A drained entry that met a commit again goes
                                // back to the front, deadline and all.
                                Some(since) => defer.push_front_kept(Deferred {
                                    since,
                                    entry: DeferEntry::Msg(msg),
                                }),
                                None => match defer.push_msg(msg, DeferInstant::now()) {
                                    Ok(()) => metrics.command_deferred(),
                                    Err(msg) => {
                                        if is_typed_line(&msg) {
                                            metrics.command_processed();
                                        }
                                        let brackets = output_brackets_of(&engine.world, &msg);
                                        refuse_world_msg(msg, reason, &gateway, brackets);
                                    }
                                },
                            }
                        } else {
                            if is_typed_line(&msg) {
                                metrics.command_processed();
                            }
                            let brackets = output_brackets_of(&engine.world, &msg);
                            refuse_world_msg(msg, reason, &gateway, brackets);
                        }
                        StepOutcome::Continue
                    }
                    Admission::CheckStoreReadOnlyCommand => {
                        // `admit_world_msg` resolves this class itself and
                        // never returns it. Seeing it means the resolver was
                        // bypassed, so treat the message as unadmitted rather
                        // than stepping it against an unresolved admission.
                        tracing::error!(
                            "unresolved store-read-only admission reached the world loop; refusing"
                        );
                        if is_typed_line(&msg) {
                            metrics.command_processed();
                        }
                        let brackets = output_brackets_of(&engine.world, &msg);
                        refuse_world_msg(msg, RefusalReason::StoreUnavailable, &gateway, brackets);
                        StepOutcome::Continue
                    }
                    Admission::Allow => match msg {
                        WorldMsg::Inbound(msg) => {
                            if let Inbound::Cmd { actor, .. } = &msg {
                                metrics.command_processed();
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
                                guarded!("engine.step", {
                                    // WS01-S8's armed world fault (feature +
                                    // environment gated, inert in every shipped
                                    // build). Inside the guard, so the proof
                                    // exercises the firewall's `Faulted` path
                                    // rather than an uncaught task panic.
                                    crate::process_fault::panic_world_turn_if_armed(&msg);
                                    engine.step(msg, &mut out, &mut fx)
                                })
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
                                guarded!(
                                    "apply_reattach_snapshot",
                                    apply_reattach_snapshot(&mut engine, &recs, &mut out)
                                );
                                tracing::info!(
                                    count = recs.len(),
                                    "re-attached held connections after restart"
                                );
                            }
                            StepOutcome::Continue
                        }
                        WorldMsg::CreatePlayer {
                            name,
                            account,
                            reply,
                        } => {
                            match create_player(&mut engine, &name) {
                                Ok(id) => {
                                    // **16b cut F.4**: the account oracle learns
                                    // of the character HERE, in the turn that
                                    // made it and before the reply leaves, so a
                                    // player who registers after boot can claim
                                    // a simulator slot without a restart (the
                                    // recorded K6 point). The map is RAM over
                                    // the store's account documents, which the
                                    // creating caller writes next; a boot
                                    // re-derives it from those documents.
                                    link_created_character(&engine, id, &name, account);
                                    // Tell module observers a Player now exists
                                    // (comsys auto-joins the default channel here).
                                    // Emitted BEFORE the flush so whatever the
                                    // observers persist rides the same durable
                                    // creation batch.
                                    guarded!(
                                        "emit_world_event/ObjCreated",
                                        engine.emit_world_event(
                                            &WorldEvent::ObjCreated {
                                                obj: id,
                                                kind: ObjKind::Player,
                                                creator: mush_core::GOD,
                                            },
                                            &mut out,
                                            &mut fx,
                                        )
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
                        WorldMsg::UnlinkPlayer { character, reply } => {
                            // **16b cut F.4**: the compensating half. The caller
                            // could not write the account document the next boot
                            // would re-derive this mapping from, so RAM has to
                            // give it back or the two disagree in the permissive
                            // direction until the next restart.
                            let forgotten = engine.unlink_account_character(character);
                            if forgotten {
                                tracing::warn!(
                                    %character,
                                    "the account oracle forgot a character whose account document \
                                     could not be written"
                                );
                            }
                            let _ = reply.send(Ok(forgotten));
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
                        WorldMsg::RelaySpeak { req, reply } => {
                            // The bridge's `(account, actor)` claim is checked
                            // against the boot-hydrated oracle (fail closed: no
                            // oracle, no match); comsys runs its own gates.
                            let result = if engine.account_of(req.actor) != Some(req.account) {
                                Err(mush_ipc::RelayError::AccountMismatch)
                            } else if req.text.chars().count() > MAX_RELAY_LINE_CHARS {
                                Err(mush_ipc::RelayError::TooLong)
                            } else {
                                engine
                                    .relay_speak(
                                        req.actor,
                                        &req.channel,
                                        req.kind,
                                        &req.text,
                                        &mut out,
                                    )
                                    .map_err(|why| mush_ipc::RelayError::Refused(why.to_string()))
                            };
                            tracing::debug!(
                                actor = %req.actor,
                                channel = %req.channel,
                                ok = result.is_ok(),
                                "relay speak dispatched"
                            );
                            let _ = reply.send(Ok(result));
                            StepOutcome::Continue
                        }
                        WorldMsg::DiscordInteraction { interaction, reply } => {
                            let core = mush_core::DiscordInteraction {
                                account: interaction.account.clone(),
                                actor: interaction.actor,
                                custom_id: interaction.custom_id.clone(),
                                values: interaction.values.clone(),
                            };
                            let dispatched = guarded!(
                                "discord_interaction",
                                engine.discord_interaction(&core, &mut out)
                            );
                            let result = match dispatched {
                                None => Err(mush_ipc::DiscordInteractionError::UnknownPrefix),
                                Some(Ok(text)) => Ok(text),
                                Some(Err(why)) => {
                                    Err(mush_ipc::DiscordInteractionError::Refused(why))
                                }
                            };
                            tracing::debug!(
                                custom_id = %interaction.custom_id,
                                handled = result.is_ok(),
                                "discord interaction dispatched"
                            );
                            let _ = reply.send(Ok(result));
                            StepOutcome::Continue
                        }
                        WorldMsg::ConnectionReady { conn, actor, reply } => {
                            let _ = reply.send(Ok(engine.actor_of(conn) == Some(actor)));
                            StepOutcome::Continue
                        }
                        WorldMsg::RecheckPrivilege {
                            actor,
                            privilege,
                            subject,
                            command,
                            reply,
                        } => {
                            // The world fields are borrowed disjointly so the
                            // audit append rides this step's deferred queue.
                            let held = crate::staff::recheck_privilege(
                                &mut engine.world,
                                &engine.registry,
                                &mut out,
                                actor,
                                privilege,
                                subject,
                                command,
                            );
                            let _ = reply.send(Ok(held));
                            StepOutcome::Continue
                        }
                        WorldMsg::LegalActions {
                            conn,
                            actor,
                            subject,
                            reply,
                        } => {
                            let result = if engine.actor_of(conn) != Some(actor) {
                                Err(LegalActionReadError::InvalidSubject)
                            } else {
                                legal_action_query(
                                    &engine,
                                    PrincipalId::Object { id: actor },
                                    actor,
                                    subject,
                                )
                                .and_then(|query| {
                                    engine
                                        .project_legal_actions(&query)
                                        .map(|offers| offers.projected())
                                        .map_err(|error| {
                                            tracing::error!(
                                                %actor,
                                                error = %error,
                                                "legal-action projection failed"
                                            );
                                            LegalActionReadError::Unavailable
                                        })
                                })
                            };
                            let _ = reply.send(result);
                            StepOutcome::Continue
                        }
                        WorldMsg::AuthoritativeAction(request) => {
                            let AuthoritativeActionRequest {
                                permit,
                                submission,
                                reply,
                            } = *request;
                            // E7c2 B1: the five steps live in ONE function, so
                            // the command-ingress adapter cannot fork a second
                            // spelling of them.
                            let staged = resolve_and_stage_offer(&mut engine, permit, submission);
                            match staged {
                                Ok((staged, actor)) => {
                                    let (owned, owned_rx) = oneshot::channel();
                                    let (finished, finished_rx) = oneshot::channel();
                                    let _ = reply.send(Ok(AuthoritativeActionWatch {
                                        owned: owned_rx,
                                        finished: finished_rx,
                                    }));
                                    failure_actor = Some(actor);
                                    action_completion = Some(ActionCompletion {
                                        disposition: None,
                                        staged,
                                        owned: Some(owned),
                                        finished: Some(finished),
                                        ownership_valid: false,
                                        post_commit_decision: None,
                                        post_commit_decision_response: None,
                                        post_commit_decision_terminal: None,
                                    });
                                }
                                Err(detail) => {
                                    let _ = reply
                                        .send(Err(AuthoritativeActionStartError::Rejected(detail)));
                                }
                            }
                            StepOutcome::Continue
                        }
                        WorldMsg::Barrier { reply } => {
                            if !defer.is_empty() {
                                // 16b cut B.5: lines queued ahead of it have
                                // not been processed yet, so the barrier waits
                                // behind them and completes after them. A full
                                // queue refuses it `Busy` (retryable), never
                                // `Ok` ahead of the queued lines and the
                                // pending receipt.
                                if let Err(reply) = defer.push_barrier(reply, DeferInstant::now()) {
                                    let _ = reply.send(Err(WorldReadError::Busy));
                                    metrics.commit_pending_barrier_refused();
                                    tracing::warn!(
                                        cap = COMMAND_DEFER_CAP,
                                        "defer queue full; refusing barrier waiter"
                                    );
                                }
                            } else {
                                settle_barrier(reply, pending_commit.as_mut(), &metrics);
                            }
                            StepOutcome::Continue
                        }
                    },
                }
            }
            // 16b cut B.5: a queued barrier reached the front. Everything queued
            // before it has been served or refused, so it is answered as a fresh
            // barrier with an empty queue would be — never queued again behind
            // entries that arrived after it.
            Wake::DeferredBarrier(reply) => {
                settle_barrier(reply, pending_commit.as_mut(), &metrics);
                StepOutcome::Continue
            }
            // 16b cut B.5: the drain arm fired over an empty queue. Nothing to do.
            Wake::Deferred => StepOutcome::Continue,
            // 16b cut B.5: the 250 ms bound, while the commit still pends. A
            // typed line is refused in a player's words; everything else hears
            // exactly the `CommitPending` refusal it always did.
            Wake::DeferExpiry => {
                for msg in defer.expire(DeferInstant::now()) {
                    if is_typed_line(&msg) {
                        metrics.command_processed();
                    }
                    let brackets = output_brackets_of(&engine.world, &msg);
                    refuse_world_msg(msg, RefusalReason::CommitTimedOut, &gateway, brackets);
                    metrics.command_defer_timed_out();
                }
                StepOutcome::Continue
            }
            Wake::Signal(signal) => {
                debug_assert_eq!(
                    classify_writer_signal(&signal),
                    IngressClass::WriterRecovery
                );
                match signal {
                    WriterSignal::StoreStatus { healthy, detail } => {
                        if healthy && pending_commit.is_some() {
                            recovery_waits_for_commit = true;
                            tracing::debug!(
                                "deferring store recovery transition until exact commit finalization"
                            );
                        } else {
                            recovery_waits_for_commit = false;
                            apply_store_status(
                                &engine.world,
                                &gateway,
                                &mut health,
                                healthy,
                                &detail,
                            );
                        }
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
            Wake::CommitReceipt(PendingReceiptEvent::Owned(ownership)) => {
                match pending_commit.as_mut() {
                    Some(pending) => complete_pending_ownership(
                        pending,
                        ownership,
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                    ),
                    None => freeze_orphan_commit_receipt(
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        "ownership",
                    ),
                }
                StepOutcome::Continue
            }
            Wake::CommitReceipt(PendingReceiptEvent::Final(ack)) => {
                // A `let ... else` cannot stand in here: this arm has to
                // produce a `StepOutcome`, and a let-else block must
                // diverge. The freeze arm therefore yields `Continue` in
                // its own right — the loop survives the breach.
                match pending_commit.take() {
                    None => {
                        freeze_orphan_commit_receipt(
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            "commit",
                        );
                        StepOutcome::Continue
                    }
                    Some(pending) => {
                        let exit_after_ack = pending.exit_after_ack;
                        let mut completed = guarded!(
                            "complete_pending_commit",
                            complete_pending_commit(
                                pending,
                                ack,
                                &mut engine,
                                &gateway,
                                &mut health,
                                &persistence_integrity,
                                &tx,
                                llm.as_ref(),
                                &deps.store,
                            )
                        );
                        let decision_response = completed.decision_response.take();
                        let decision_terminal = completed.decision_terminal.take();
                        guarded!(
                            "finish_pending_operational",
                            finish_pending_operational(&mut engine, &gateway, completed)
                        );
                        if let Some(response) = decision_response {
                            let resolving = if let Some(store) = deps.store.authoritative_store() {
                                stage_decision_resolving(
                                    &mut engine,
                                    store,
                                    &response.fence,
                                    &response.binding,
                                    response.outcome,
                                    action_epoch,
                                )
                                .await
                            } else {
                                Err("authoritative store capability disappeared".to_owned())
                            };
                            match resolving {
                                Ok(Some(staged)) => {
                                    action_completion = Some(ActionCompletion {
                                        disposition: None,
                                        staged,
                                        owned: None,
                                        finished: None,
                                        ownership_valid: false,
                                        post_commit_decision: None,
                                        post_commit_decision_response: None,
                                        post_commit_decision_terminal: Some(
                                            DecisionTerminalPostCommit {
                                                fence: response.fence,
                                                binding: response.binding,
                                                outcome: response.outcome,
                                            },
                                        ),
                                    });
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::error!(
                                        owner = %response.fence.controller().owner,
                                        decision = ?response.fence.decision(),
                                        error = %error,
                                        "failed to bind committed selected action to pending decision"
                                    );
                                }
                            }
                        }
                        if let Some(terminal) = decision_terminal {
                            let staged = if let Some(store) = deps.store.authoritative_store() {
                                stage_decision_terminal(
                                    &mut engine,
                                    store,
                                    &terminal.fence,
                                    &terminal.binding,
                                    terminal.outcome,
                                    action_epoch,
                                )
                                .await
                            } else {
                                Err("authoritative store capability disappeared".to_owned())
                            };
                            match staged {
                                Ok(Some(staged)) => {
                                    action_completion = Some(ActionCompletion {
                                        disposition: None,
                                        staged,
                                        owned: None,
                                        finished: None,
                                        ownership_valid: false,
                                        post_commit_decision: None,
                                        post_commit_decision_response: None,
                                        post_commit_decision_terminal: None,
                                    });
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::error!(
                                        owner = %terminal.fence.controller().owner,
                                        decision = ?terminal.fence.decision(),
                                        error = %error,
                                        "failed to terminalize resolved decision"
                                    );
                                }
                            }
                        }
                        if std::mem::take(&mut recovery_waits_for_commit) {
                            apply_store_status(&engine.world, &gateway, &mut health, true, "");
                        }
                        if exit_after_ack {
                            StepOutcome::Shutdown
                        } else {
                            StepOutcome::Continue
                        }
                    }
                }
            }
            Wake::Timer => {
                // A poison freeze is a hard mutation barrier for scheduled work:
                // do not fire due timers (scheduled sim, NPC cadence, @wait) — the
                // ledger is holed and mutating further would compound it. A Timer
                // wake here is then only the re-emit nudge, which drives the
                // foot-of-loop persistence flush (re-emit stays live while frozen).
                if admission_for(admission_mode, IngressClass::Timer) == Admission::Allow {
                    guarded!(
                        "fire_due",
                        engine.fire_due(unix_now_ms(), &mut out, &mut fx)
                    );
                }
                StepOutcome::Continue
            }
            // ---- 16b cut T.2 (T4): the arena egress ----
            //
            // One arena message woke the loop; everything else already queued is
            // drained here, without blocking, so a pulse's chunks and frames are
            // handled together and the loop pays one step for the batch. The
            // messages are **parked**, not encoded: they are moved into the game's
            // own shared table and re-entered through its registered
            // `AsyncCompletion` with a token payload, which is the one mechanism
            // in the tree for running module code with a `Ctx` from the loop.
            Wake::ArenaEgress(first) => {
                let mut staged = 0_usize;
                if let Some(arenas) = arenas.as_ref() {
                    if arenas.stage(*first) {
                        staged += 1;
                    }
                    if let Some(rx) = arena_egress.as_mut() {
                        while staged < tacsim::arena_bridge::MAX_EGRESS_PER_DRAIN {
                            match rx.try_recv() {
                                Ok(message) => {
                                    if !arenas.stage(message) {
                                        break;
                                    }
                                    staged += 1;
                                }
                                Err(_) => break,
                            }
                        }
                    }
                }
                if staged == 0 {
                    StepOutcome::Continue
                } else {
                    guarded!(
                        "engine.step/arena-egress",
                        engine.step(
                            Inbound::Apply(WorldCommand::ModuleAsyncDone {
                                module: SmolStr::new_static(tacsim::arena_bridge::MODULE),
                                actor: mush_core::GOD,
                                correlation: 0,
                                payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
                            }),
                            &mut out,
                            &mut fx,
                        )
                    )
                }
            }
            // ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
            Wake::CommandSubmits => {
                let Some(store) = deps.store.authoritative_store() else {
                    // E7c2 fold R11: a queued submission parked by a freeze was
                    // mute — acceptance and loss were indistinguishable at the
                    // terminal. Tell whoever is waiting before the world stops.
                    let waiting: Vec<ObjId> = engine.queued_authoritative_actors().collect();
                    for actor in waiting {
                        out.best_effort_error(
                            actor,
                            "Your queued action is held while the world recovers; nothing was done.",
                        );
                    }
                    apply_integrity_freeze(
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        "command ingress adapter has no authoritative store capability",
                    );
                    continue;
                };
                // H2a (D-H-6): a module's decision capture becomes the owner's
                // capture-action submission — built by the kernel, armed in the
                // owner's sink, then driven through the ONE five-step function
                // below like any command submit.
                if !engine.has_legacy_persistence_work()
                    && let Some(capture) = engine.take_decision_capture()
                {
                    drive_decision_capture(
                        &mut engine,
                        capture,
                        command_submits.idempotency_epoch,
                        &mut out,
                    );
                }
                match command_submits.drive_one(&mut engine, store).await {
                    Ok(
                        CommandSubmitDrive::Idle
                        | CommandSubmitDrive::DeferredByLegacyPredecessor
                        | CommandSubmitDrive::DeferredByPreflight,
                    ) => {}
                    Ok(CommandSubmitDrive::Refused {
                        actor,
                        detail,
                        integrity,
                    }) => {
                        // E7c2 fold R8: a quarantined or conflicting durable
                        // identity is an operator event, not a typo. The actor's
                        // line is the same generic one either way.
                        if integrity {
                            tracing::error!(
                                actor = %actor,
                                detail = %detail,
                                "command authoritative submission hit a durable-integrity refusal"
                            );
                        } else {
                            tracing::info!(
                                actor = %actor,
                                detail = %detail,
                                "command authoritative submission refused before any durable intent"
                            );
                        }
                        // Nothing was staged, so this is transient runtime
                        // truth: it is delivered whatever the step's unrelated
                        // writes do, and it never claims a mutation happened.
                        out.best_effort_error(actor, "That did not go through. Nothing changed.");
                    }
                    Ok(CommandSubmitDrive::AlreadyOwned { actor }) => {
                        out.best_effort_error(
                            actor,
                            "That exact request is already committing; nothing was done twice.",
                        );
                    }
                    Ok(CommandSubmitDrive::AlreadyFinished {
                        actor,
                        receipt,
                        status,
                    }) => {
                        // A HIT. The confirmation is projected from the DURABLE
                        // receipt — no provider, no RNG and no second commit.
                        let projections = engine
                            .registry
                            .action_providers()
                            .map(mush_core::ActionProviderRegistry::receipt_projection_table);
                        let line = projections
                            .and_then(|projections| {
                                ActionCoordinator::project_finalized_receipt(
                                    &projections,
                                    &PrincipalId::Object { id: actor },
                                    status,
                                    &receipt,
                                )
                                .ok()
                            })
                            .and_then(|projected| {
                                projected.result().map(|result| result.summary().to_owned())
                            });
                        match line {
                            Some(summary) => out.info(actor, format!("{summary} (already done)")),
                            None => out.info(actor, "That was already done."),
                        }
                    }
                    Ok(CommandSubmitDrive::Staged {
                        staged,
                        actor,
                        disposition,
                    }) => {
                        // The confirmation is COMMIT-BOUND: `resolve_classified_sends`
                        // collects it below and the delivery boundary withholds
                        // it until this exact bundle's durable receipt resolves,
                        // suppressing it entirely on abort (§4 checklist item 1).
                        //
                        // E7c2 fold B12: it is the PROVIDER's own sentence, the
                        // same projection a HIT renders — never
                        // `deterministic_commit_token`'s internal binding digest
                        // rendered raw into player output.
                        failure_actor = Some(actor);
                        out.info(
                            actor,
                            staged.summary.clone().unwrap_or_else(|| "Done.".to_owned()),
                        );
                        action_completion = Some(ActionCompletion {
                            disposition,
                            staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: None,
                            post_commit_decision_response: None,
                            post_commit_decision_terminal: None,
                        });
                    }
                    Err(detail) => {
                        apply_integrity_freeze(
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &format!("command ingress adapter failed: {detail}"),
                        );
                    }
                }
                StepOutcome::Continue
            }
            Wake::DurableSchedules => {
                let Some(store) = deps.store.authoritative_store() else {
                    apply_integrity_freeze(
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        "durable schedule driver has no authoritative store capability",
                    );
                    continue;
                };
                match durable_schedules.drive_one(&mut engine, store).await {
                    Ok(
                        DurableScheduleDrive::Idle
                        | DurableScheduleDrive::DeferredByLegacyPredecessor,
                    ) => {
                        durable_schedule_transient_alarmed = false;
                    }
                    Ok(DurableScheduleDrive::DiscardedStale(detail)) => {
                        durable_schedule_transient_alarmed = false;
                        tracing::debug!(
                            error = %detail,
                            "discarded stale durable schedule candidate"
                        );
                    }
                    Ok(DurableScheduleDrive::Transient(detail)) => {
                        if !durable_schedule_transient_alarmed {
                            tracing::warn!(
                                error = %detail,
                                "durable schedule drive is temporarily unavailable"
                            );
                            durable_schedule_transient_alarmed = true;
                        }
                    }
                    Ok(DurableScheduleDrive::Staged(staged)) => {
                        durable_schedule_transient_alarmed = false;
                        action_completion = Some(ActionCompletion {
                            disposition: None,
                            staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: None,
                            post_commit_decision_response: None,
                            post_commit_decision_terminal: None,
                        });
                    }
                    Err(detail) => {
                        apply_integrity_freeze(
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &format!("durable schedule drive failed: {detail}"),
                        );
                    }
                }
                StepOutcome::Continue
            }
            wake @ (Wake::DecisionNotification(_) | Wake::DurableDecisions) => {
                let Some(store) = deps.store.authoritative_store() else {
                    apply_integrity_freeze(
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        "delayed-decision driver has no authoritative store capability",
                    );
                    continue;
                };
                let wake = match &wake {
                    Wake::DecisionNotification(wake) => Some(wake),
                    Wake::DurableDecisions => None,
                    // The arm pattern above admits exactly those two wakes;
                    // anything else means the pattern and this match drifted.
                    // Drop the wake instead of the world.
                    _ => {
                        tracing::error!(
                            "delayed-decision driver saw a wake it does not own; ignoring"
                        );
                        continue;
                    }
                };
                let Some(driver) = durable_decisions.as_mut() else {
                    continue;
                };
                match driver.drive_one(&mut engine, store, wake).await {
                    Ok(
                        DurableDecisionDrive::Idle
                        | DurableDecisionDrive::DeferredByLegacyPredecessor
                        | DurableDecisionDrive::RecoveryAuditProgress,
                    ) => {
                        durable_decision_transient_alarmed = false;
                    }
                    Ok(DurableDecisionDrive::DiscardedStale(detail)) => {
                        durable_decision_transient_alarmed = false;
                        tracing::debug!(
                            error = %detail,
                            "discarded stale delayed-decision candidate"
                        );
                    }
                    Ok(DurableDecisionDrive::Transient(detail)) => {
                        if !durable_decision_transient_alarmed {
                            tracing::warn!(
                                error = %detail,
                                "delayed-decision dispatch is temporarily unavailable"
                            );
                            durable_decision_transient_alarmed = true;
                        }
                    }
                    Ok(DurableDecisionDrive::Staged(staged_dispatch)) => {
                        let StagedDecisionDispatch {
                            staged,
                            reservation,
                        } = *staged_dispatch;
                        durable_decision_transient_alarmed = false;
                        let StagedDecisionLease { staged, worker } = staged;
                        action_completion = Some(ActionCompletion {
                            disposition: None,
                            staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: Some(DecisionPostCommit {
                                reservation,
                                worker,
                            }),
                            post_commit_decision_response: None,
                            post_commit_decision_terminal: None,
                        });
                    }
                    Ok(DurableDecisionDrive::StagedLifecycle(staged)) => {
                        durable_decision_transient_alarmed = false;
                        action_completion = Some(ActionCompletion {
                            disposition: None,
                            staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: None,
                            post_commit_decision_response: None,
                            post_commit_decision_terminal: None,
                        });
                    }
                    Ok(DurableDecisionDrive::StagedFallback(selected)) => {
                        action_completion = Some(ActionCompletion {
                            disposition: None,
                            staged: selected.staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: None,
                            post_commit_decision_response: Some(DecisionResponsePostCommit {
                                fence: selected.fence,
                                binding: selected.binding,
                                outcome: mush_core::DecisionTerminalOutcome::Fallback,
                            }),
                            post_commit_decision_terminal: None,
                        });
                    }
                    Ok(DurableDecisionDrive::StagedLifecycleWithTerminal {
                        staged,
                        fence,
                        binding,
                        outcome,
                    }) => {
                        action_completion = Some(ActionCompletion {
                            disposition: None,
                            staged,
                            owned: None,
                            finished: None,
                            ownership_valid: false,
                            post_commit_decision: None,
                            post_commit_decision_response: None,
                            post_commit_decision_terminal: Some(DecisionTerminalPostCommit {
                                fence,
                                binding,
                                outcome,
                            }),
                        });
                    }
                    Err(detail) => {
                        if let Some(owner) = driver.take_failure_owner() {
                            let incident = incident_id("decision-recovery", &detail);
                            let reason = mush_core::RecoveryReason::IntegrityViolation {
                                code: mush_core::CanonicalId::new("decision_recovery")
                                    .expect("static decision recovery code"),
                            };
                            match engine
                                .registry
                                .freeze_mechanics_owner(&owner, incident, reason)
                            {
                                Ok(_) => tracing::error!(
                                    owner = %owner,
                                    incident = ?incident,
                                    error = %detail,
                                    "delayed-decision owner frozen after recovery integrity failure"
                                ),
                                Err(error) => apply_integrity_freeze(
                                    &engine.world,
                                    &gateway,
                                    &mut health,
                                    &persistence_integrity,
                                    &format!(
                                        "delayed-decision owner freeze failed after {detail}: {error}"
                                    ),
                                ),
                            }
                        } else {
                            apply_integrity_freeze(
                                &engine.world,
                                &gateway,
                                &mut health,
                                &persistence_integrity,
                                &format!("delayed-decision drive failed: {detail}"),
                            );
                        }
                    }
                }
                StepOutcome::Continue
            }
            Wake::DecisionResult(completion) => {
                let Some(store) = deps.store.authoritative_store() else {
                    apply_integrity_freeze(
                        &engine.world,
                        &gateway,
                        &mut health,
                        &persistence_integrity,
                        "decision response ingress has no authoritative store capability",
                    );
                    continue;
                };
                let fence = completion.fence();
                match completion.into_result() {
                    Err(error) => {
                        let kind = match error.kind() {
                            crate::decision_dispatch::DecisionProviderFailureKind::Unavailable => {
                                mush_core::DecisionAttemptFailureKind::Unavailable
                            }
                            crate::decision_dispatch::DecisionProviderFailureKind::Malformed => {
                                mush_core::DecisionAttemptFailureKind::Malformed
                            }
                            crate::decision_dispatch::DecisionProviderFailureKind::Permanent => {
                                mush_core::DecisionAttemptFailureKind::Permanent
                            }
                        };
                        match stage_decision_attempt(&mut engine, store, &fence, kind, action_epoch)
                            .await
                        {
                            Ok(Some(attempt)) => {
                                tracing::warn!(
                                    owner = %fence.controller().owner,
                                    decision = ?fence.decision(),
                                    revision = fence.revision(),
                                    attempt = fence.attempt(),
                                    kind = ?error.kind(),
                                    error = %error.detail(),
                                    fallback = attempt.fallback_fence.is_some(),
                                    "decision provider failure entered durable lifecycle"
                                );
                                action_completion = Some(ActionCompletion {
                                    disposition: None,
                                    staged: attempt.staged,
                                    owned: None,
                                    finished: None,
                                    ownership_valid: false,
                                    post_commit_decision: None,
                                    post_commit_decision_response: None,
                                    post_commit_decision_terminal: None,
                                });
                            }
                            Ok(None) => {
                                tracing::info!(
                                    decision = ?fence.decision(),
                                    attempt = fence.attempt(),
                                    "decision provider failure joined existing attempt lifecycle"
                                );
                            }
                            Err(detail) => {
                                tracing::error!(
                                    decision = ?fence.decision(),
                                    attempt = fence.attempt(),
                                    error = %detail,
                                    "decision provider failure could not enter durable lifecycle"
                                );
                            }
                        }
                    }
                    Ok(response) => {
                        let ingress = match ActionCoordinator::decision_response_ingress(
                            &engine, &fence, &response,
                        ) {
                            Ok(ingress) => ingress,
                            Err(error) => {
                                tracing::warn!(
                                    owner = %fence.controller().owner,
                                    decision = ?fence.decision(),
                                    revision = fence.revision(),
                                    attempt = fence.attempt(),
                                    error = %error,
                                    "refused stale or malformed decision response"
                                );
                                continue;
                            }
                        };
                        let preflight = match ActionCoordinator::preflight(store, ingress).await {
                            Ok(preflight) => preflight,
                            Err(error) => {
                                tracing::warn!(
                                    owner = %fence.controller().owner,
                                    decision = ?fence.decision(),
                                    error = %error,
                                    "decision response idempotency preflight is unavailable"
                                );
                                continue;
                            }
                        };
                        match preflight {
                            ActionPreflight::Vacant(permit) => {
                                match ActionCoordinator::prepare_and_stage_decision_response(
                                    permit,
                                    &mut engine,
                                    fence,
                                    &response,
                                ) {
                                    Ok(StagedDecisionResponse {
                                        staged,
                                        fence,
                                        binding,
                                    }) => {
                                        action_completion = Some(ActionCompletion {
                                            disposition: None,
                                            staged,
                                            owned: None,
                                            finished: None,
                                            ownership_valid: false,
                                            post_commit_decision: None,
                                            post_commit_decision_response: Some(
                                                DecisionResponsePostCommit {
                                                    fence,
                                                    binding,
                                                    outcome:
                                                        mush_core::DecisionTerminalOutcome::Executed,
                                                },
                                            ),
                                            post_commit_decision_terminal: None,
                                        });
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            error = %error,
                                            "decision response failed current offer revalidation"
                                        );
                                    }
                                }
                            }
                            ActionPreflight::Pending { token, bundle_hash } => {
                                tracing::info!(
                                    %token,
                                    %bundle_hash,
                                    decision = ?fence.decision(),
                                    "decision response joined an existing selected-action intent"
                                );
                            }
                            ActionPreflight::Finalized(receipt) => {
                                let binding = mush_core::DecisionActionBinding {
                                    request: receipt.request(),
                                    request_hash: receipt.request_hash(),
                                };
                                match stage_decision_resolving(
                                    &mut engine,
                                    store,
                                    &fence,
                                    &binding,
                                    mush_core::DecisionTerminalOutcome::Executed,
                                    action_epoch,
                                )
                                .await
                                {
                                    Ok(Some(staged)) => {
                                        tracing::info!(
                                            token = %receipt.token(),
                                            decision = ?fence.decision(),
                                            "reconciled an already finalized selected action"
                                        );
                                        action_completion = Some(ActionCompletion {
                                            disposition: None,
                                            staged,
                                            owned: None,
                                            finished: None,
                                            ownership_valid: false,
                                            post_commit_decision: None,
                                            post_commit_decision_response: None,
                                            post_commit_decision_terminal: Some(
                                                DecisionTerminalPostCommit {
                                                    fence,
                                                    binding,
                                                    outcome:
                                                        mush_core::DecisionTerminalOutcome::Executed,
                                                },
                                            ),
                                        });
                                    }
                                    Ok(None) => {
                                        tracing::info!(
                                            token = %receipt.token(),
                                            decision = ?fence.decision(),
                                            "joined an existing selected-action lifecycle intent"
                                        );
                                    }
                                    Err(error) => {
                                        tracing::error!(
                                            token = %receipt.token(),
                                            decision = ?fence.decision(),
                                            error = %error,
                                            "failed to reconcile finalized selected action"
                                        );
                                    }
                                }
                            }
                            ActionPreflight::Retired {
                                minimum_accepted_epoch,
                            } => {
                                tracing::warn!(
                                    decision = ?fence.decision(),
                                    minimum_accepted_epoch,
                                    "decision response action idempotency epoch is retired"
                                );
                            }
                            ActionPreflight::Quarantined(receipt) => {
                                tracing::error!(
                                    token = %receipt.token(),
                                    decision = ?fence.decision(),
                                    "decision response selected action is quarantined"
                                );
                            }
                            ActionPreflight::Conflict => {
                                tracing::info!(
                                    decision = ?fence.decision(),
                                    "decision response lost the selected-action idempotency race"
                                );
                            }
                        }
                    }
                }
                StepOutcome::Continue
            }
            Wake::Shutdown => {
                shutdown_requested = true;
                // 16b cut B.5: nothing queued is served past a shutdown. Every
                // queued message hears the refusal it would have heard before
                // the queue existed, and every queued barrier is answered the
                // way a retained one is: behind the pending commit's receipt
                // (or the shutdown grace), at once when none is pending.
                for deferred in defer.drain_all() {
                    match deferred.entry {
                        DeferEntry::Msg(msg) => {
                            if is_typed_line(&msg) {
                                metrics.command_processed();
                            }
                            let brackets = output_brackets_of(&engine.world, &msg);
                            refuse_world_msg(msg, RefusalReason::CommitPending, &gateway, brackets);
                        }
                        DeferEntry::Barrier(reply) => {
                            settle_barrier(reply, pending_commit.as_mut(), &metrics);
                        }
                    }
                }
                if pending_commit.is_some() {
                    // Supersede the one already-dequeued ordinary message and
                    // stop polling the bounded queue. The shutdown request did
                    // not need queue capacity, and the prior exact commit gets
                    // a bounded grace period to finish normally.
                    if let Some(blocked) = deferred_world_msg.take() {
                        let brackets = output_brackets_of(&engine.world, &blocked);
                        refuse_world_msg(blocked, RefusalReason::CommitPending, &gateway, brackets);
                    }
                    deferred_world_msg =
                        Some(WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)));
                    shutdown_deadline = Some(Instant::now() + WORLD_SHUTDOWN_GRACE);
                    StepOutcome::Continue
                } else {
                    guarded!(
                        "engine.step/shutdown",
                        engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx)
                    )
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
                    let completed = guarded!(
                        "complete_pending_commit/shutdown-deadline",
                        complete_pending_commit(
                            pending,
                            Err(AuthoritativeCommitFailure::Integrity(detail.to_owned())),
                            &mut engine,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &tx,
                            llm.as_ref(),
                            &deps.store,
                        )
                    );
                    guarded!(
                        "finish_pending_operational/shutdown-deadline",
                        finish_pending_operational(&mut engine, &gateway, completed)
                    );
                }
                metrics.set_persistence_state(
                    health.mode.store_read_only(),
                    health.mode.integrity_frozen(),
                );
                publish_pending_custody(&metrics, None, 0);
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
                    shutdown_deadline = Some(Instant::now() + WORLD_SHUTDOWN_GRACE);
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
        let mut classified_sends = resolve_classified_sends(
            &engine.registry,
            &engine.world,
            &mut out,
            TypedStateLens::Live(engine.action_state()),
        );
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
                    if action_completion.is_some() {
                        fail_action_completion(
                            action_completion.take(),
                            AuthoritativeCommitFailure::Integrity(
                                "authoritative action produced no durable write".to_owned(),
                            ),
                        );
                        abort_without_receipt(
                            classified_sends,
                            classified_effects,
                            failure_actor,
                            create_completion.take(),
                            "authoritative action produced no durable write",
                            &engine.world,
                            &gateway,
                            &mut health,
                            &persistence_integrity,
                            &tx,
                            llm.as_ref(),
                            &deps.store,
                        );
                    } else if create_completion.is_some() {
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
                            &deps.store,
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
                            &deps.store,
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
                        action: action_completion.take(),
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
                    fail_action_completion(
                        action_completion.take(),
                        AuthoritativeCommitFailure::Integrity(
                            "durable writer accepted an authoritative action without returning its receipt"
                                .to_owned(),
                        ),
                    );
                    if let Some(installation) = prepared_installation {
                        release_prepared_installation(
                            &mut engine,
                            installation,
                            "step-flush/queued-without-receipt",
                        );
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
                        &deps.store,
                    );
                }
                FlushOutcome::Refused(detail) => {
                    fail_action_completion(
                        action_completion.take(),
                        AuthoritativeCommitFailure::Refused(detail.clone()),
                    );
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
                        &deps.store,
                    );
                }
                FlushOutcome::Rejected(detail) => {
                    fail_action_completion(
                        action_completion.take(),
                        AuthoritativeCommitFailure::Integrity(detail.clone()),
                    );
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
                        &deps.store,
                    );
                }
                FlushOutcome::WriterGone => {
                    fail_action_completion(
                        action_completion.take(),
                        AuthoritativeCommitFailure::Integrity(
                            "store writer disappeared before accepting the authoritative batch"
                                .to_owned(),
                        ),
                    );
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
                        &deps.store,
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
            guarded!(
                "run_local_staff_ops",
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
                )
            );
            let mut sends = resolve_classified_sends(
                &engine.registry,
                &engine.world,
                &mut out,
                TypedStateLens::Live(engine.action_state()),
            );
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
                        &deps.store,
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
                        action: None,
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
                        release_prepared_installation(
                            &mut engine,
                            installation,
                            "timer-flush/queued-without-receipt",
                        );
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
                        &deps.store,
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
                    &deps.store,
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
                    &deps.store,
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
                    &deps.store,
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
        publish_pending_custody(&metrics, pending_commit.as_ref(), defer.barriers());
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
            clean_exit = true;
            break;
        }
        if staff_shutdown {
            // In-game `@shutdown`: step the same ControlMsg the console path
            // sends, so the announce-to-all and the StepOutcome are
            // identical; main's shutdown sequence (writer drain → store
            // drain → BGSAVE) runs when this task's join resolves.
            let outcome = guarded!(
                "engine.step/staff-shutdown",
                engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx)
            );
            debug_assert_eq!(outcome, StepOutcome::Shutdown);
            let mut sends = resolve_classified_sends(
                &engine.registry,
                &engine.world,
                &mut out,
                TypedStateLens::Live(engine.action_state()),
            );
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
                        &deps.store,
                    );
                    tracing::info!("world task shut down cleanly");
                    clean_exit = true;
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
                        action: None,
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
                        release_prepared_installation(
                            &mut engine,
                            installation,
                            "shutdown-flush/queued-without-receipt",
                        );
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
                        &deps.store,
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
                        &deps.store,
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
                        &deps.store,
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
                        &deps.store,
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
    // 16b cut B.5: whatever still waits in the defer queue when the loop ends
    // (the shutdown grace ran out, the inbound channel closed behind a pending
    // commit, a fault) is answered rather than dropped: a message hears the
    // retryable `CommitPending` refusal, a barrier `Unavailable`, which is what
    // a vanished world task means to its caller.
    for deferred in defer.drain_all() {
        match deferred.entry {
            DeferEntry::Msg(msg) => {
                if is_typed_line(&msg) {
                    metrics.command_processed();
                }
                let brackets = output_brackets_of(&engine.world, &msg);
                refuse_world_msg(msg, RefusalReason::CommitPending, &gateway, brackets);
            }
            DeferEntry::Barrier(reply) => {
                let _ = reply.send(Err(WorldReadError::Unavailable));
            }
        }
    }
    // Dropping `writer` here closes the job queue; the writer task drains
    // whatever is still pending and exits (awaited by the shutdown
    // sequence via `PersistHandles::writer_join`).
    //
    // The clean-exit publication is the LAST statement of the world loop: any
    // panic or abandonment before this line leaves the status false, and the
    // shutdown coordinator then skips the ordinary final BGSAVE rather than
    // snapshotting whatever the store happened to have caught.
    if clean_exit {
        world_exit.mark_clean();
    }
    match fault {
        Some(fault) => WorldOutcome::Faulted(fault),
        None => WorldOutcome::Clean,
    }
}

/// The `OUTPUTPREFIX`/`OUTPUTSUFFIX` row of the connection a typed line came
/// from (16b cut AP.4), cloned for [`refuse_world_msg`]; `None` for anything
/// that is not a typed line, and for a connection that set neither.
fn output_brackets_of(world: &World, msg: &WorldMsg) -> Option<OutputBrackets> {
    match msg {
        WorldMsg::Inbound(Inbound::Cmd { conn, .. }) => world.output_brackets(*conn).cloned(),
        _ => None,
    }
}

/// Complete a refused message without ever handing it to the engine. Requests
/// with typed replies receive a conservative answer instead of hanging; work
/// without a reply is dropped with a bounded diagnostic.
///
/// `brackets` is the refused typed line's connection row (16b cut AP.4,
/// [`output_brackets_of`]): a world-task refusal answers the line as the
/// engine would have, so a connection that set `OUTPUTPREFIX`/`OUTPUTSUFFIX`
/// reads the prefix, the refusal and the suffix — to that connection, in ONE
/// transmit so nothing lands between. Without a row the refusal goes to the
/// actor as it always did.
fn refuse_world_msg(
    msg: WorldMsg,
    reason: RefusalReason,
    gateway: &Gateway,
    brackets: Option<OutputBrackets>,
) {
    // 16b cut B.5: only a typed line hears `CommitTimedOut`'s own sentence.
    // Everything else the defer queue times out — a query (a hydration request,
    // not a typed line) and every reply-carrying read — is refused exactly as
    // `CommitPending` refuses it, because its callers already speak `Busy`.
    let reason = if reason == RefusalReason::CommitTimedOut && !is_typed_line(&msg) {
        RefusalReason::CommitPending
    } else {
        reason
    };
    match msg {
        WorldMsg::Inbound(Inbound::Cmd { conn, actor, .. }) => match brackets {
            Some(brackets) => {
                let line = |tag: &str, text: &str| {
                    (
                        SendTarget::Conn(conn),
                        Arc::new(OutputEvent::Sys(RichText::tag(tag, text))),
                    )
                };
                let mut sends = Vec::with_capacity(3);
                if let Some(prefix) = &brackets.prefix {
                    sends.push(line(OUTPUT_PREFIX_TAG, prefix));
                }
                // Worded and tagged exactly as `send_sys` words it.
                sends.push(line("system", reason.text()));
                if let Some(suffix) = &brackets.suffix {
                    sends.push(line(OUTPUT_SUFFIX_TAG, suffix));
                }
                gateway.transmit(sends);
            }
            None => send_sys(gateway, actor, reason.text()),
        },
        WorldMsg::Inbound(Inbound::ConnUp { actor, .. }) => {
            send_sys(gateway, actor, reason.text());
        }
        WorldMsg::Inbound(Inbound::Invoke { conn, id, .. }) => {
            gateway.transmit(vec![(
                SendTarget::Conn(conn),
                Arc::new(OutputEvent::InvokeResult {
                    id,
                    result: mush_proto::InvokeResult::error("unavailable", reason.text()),
                }),
            )]);
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
        // A presence hint is best-effort by contract: dropping it costs
        // nothing (the clients expire their pills on a timer).
        WorldMsg::Inbound(Inbound::Composing { .. }) => {}
        // A request answer is advisory/best-effort (interface plan I10):
        // dropping it means the server applies the deadline default. No reply.
        WorldMsg::Inbound(Inbound::Respond { .. }) => {}
        WorldMsg::Inbound(Inbound::Apply(_)) => {
            tracing::warn!(?reason, "async apply suppressed by world mode");
        }
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Dump)) => {
            tracing::warn!(?reason, "dump control suppressed by world mode");
        }
        // Shutdown and a transport-only reattach are admitted in every world
        // mode, so neither should ever be routed here. If one is, the refusal
        // is the safe half — refusing a shutdown only means the operator has
        // to ask again — and losing the loop would be the unsafe half.
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)) => {
            tracing::error!(?reason, "shutdown refused by world mode; dropped");
        }
        WorldMsg::ReAttach { .. } => {
            tracing::error!(?reason, "reattach refused by world mode; dropped");
        }
        WorldMsg::CreatePlayer { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                CreatePlayerErr::Busy
            } else {
                CreatePlayerErr::StoreUnavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::NameFree { reply, .. } | WorldMsg::UnlinkPlayer { reply, .. } => {
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
        WorldMsg::DiscordInteraction { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::RelaySpeak { reply, .. } => {
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
        WorldMsg::RecheckPrivilege { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                WorldReadError::Busy
            } else {
                WorldReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::LegalActions { reply, .. } => {
            let error = if reason == RefusalReason::CommitPending {
                LegalActionReadError::Busy
            } else {
                LegalActionReadError::Unavailable
            };
            let _ = reply.send(Err(error));
        }
        WorldMsg::AuthoritativeAction(request) => {
            let error = if reason == RefusalReason::CommitPending {
                AuthoritativeActionStartError::Busy
            } else {
                AuthoritativeActionStartError::Unavailable
            };
            let _ = request.reply.send(Err(error));
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

/// The typed heads `refuse_store_read_only` lets through while the store is
/// read-only: the conservative non-mutating allowlist, matched against the
/// line's first word (switches stripped, lowercased). Public so a contract
/// test can read the real list: 16b cut AD.1 asserts no word here can be a
/// player's macro name, since the gate trusts a typed head and a macro of
/// that name would otherwise expand past it.
pub const STORE_READ_ONLY_ALLOWED: &[&str] = &[
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
    // 16b cut AP.4: a connection's reply brackets are one RAM row on the
    // World; setting one needs no store.
    "outputprefix",
    "outputsuffix",
];

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
    if STORE_READ_ONLY_ALLOWED.contains(&verb.as_str()) {
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

/// Hand a prepared installation back to the engine on a path that already
/// carries its own refusal.
///
/// [`Engine::abort_prepared_action`] refuses only when the installation does
/// not match the store it is being applied to — a wiring bug, not gameplay —
/// and it checks before it mutates, so live action state is untouched either
/// way. Every caller of this helper is already returning a
/// [`FlushOutcome::Rejected`]/[`FlushOutcome::WriterGone`] or an
/// integrity failure that the world loop turns into a freeze, so the refusal
/// is logged with its site and the caller's own disposition stands. It is
/// never asserted: this is the single writer.
fn release_prepared_installation(
    engine: &mut Engine,
    installation: PreparedInstallation,
    site: &'static str,
) {
    if let Err(error) = engine.abort_prepared_action(installation) {
        tracing::error!(
            site,
            %error,
            "prepared action-state installation could not be released"
        );
    }
}

/// Fail stop on a prepared installation that died without an install or an
/// abort.
///
/// [`release_prepared_installation`] covers every path this file knows how to
/// end. What it cannot cover is a `PreparedInstallation` that is simply
/// dropped — a `PendingCommit` that goes out of scope, a panic caught between
/// taking the installation out and answering for it. By then
/// `PreparedTxn::into_parts` has already handed the module ops to the writer,
/// so the durable half of the commit may be acknowledged while the typed RAM
/// post-images are gone: RAM and Redis have diverged, and the write-through
/// rule that says they may not (docs/PLAN.md) has no local repair.
///
/// The store witnesses that itself — it poisons its reservation rather than
/// freeing it, and counts the loss — and this is the world loop reading that
/// count once per iteration, before admission. A loss is deliberately louder
/// than the wedge it replaces: divergence with no witness has no recovery, a
/// freeze has both. Returns whether this call was the one that saw a new loss.
fn freeze_on_lost_installation(
    engine: &Engine,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    seen: &mut u64,
) -> bool {
    let lost = engine.action_state().lost_installations();
    if lost == *seen {
        return false;
    }
    *seen = lost;
    apply_integrity_freeze(
        &engine.world,
        gateway,
        health,
        persistence_integrity,
        &format!(
            "{lost} prepared action-state installation(s) were dropped without an install or an \
             abort; acknowledged writes may not be live in RAM"
        ),
    );
    true
}

/// Latch the non-auto-lifting freeze on any deterministic persistence breach:
/// enter [`WorldMode::IntegrityFrozen`] so a later transient recovery cannot
/// lift it, and alert Wizards. Idempotent — the writer may send an initial
/// failure plus per-seal resolution details; only the first transition alerts.
/// The panic firewall's recovery path: everything the loop does between
/// catching a panic and breaking out with a [`WorldFault`].
///
/// A panic raised *inside* a `catch_unwind` handler aborts the process, so this
/// may call nothing that can panic. Two rules follow from that. The staged
/// action is dropped through [`Engine::discard_step_residue`] — an infallible
/// abort that runs no registry, guard, validator or observer code — rather than
/// through `Engine::take_staged_action`, which revalidates guards through the
/// registry and can fail (or panic) on its own. And the discard itself runs
/// inside a second [`guard`]: if even that explodes, the barrier is latched
/// directly and no further engine call is made.
///
/// The turn's presentation buffers are replaced wholesale. A faulted turn is
/// not a turn: half of a handler ran, the outbox holds whatever it had said so
/// far, and none of it may reach a player as though the turn had happened.
#[allow(clippy::too_many_arguments)] // the recovery path needs the loop's whole context
fn recover_from_world_fault(
    fault: WorldFault,
    engine: &mut Engine,
    out: &mut Outbox,
    fx: &mut Effects,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    metrics: &Metrics,
) -> WorldFault {
    metrics.world_panic();
    let detail = format!("world task panicked at {}: {}", fault.site, fault.detail);
    if guard("discard_step_residue", || engine.discard_step_residue()).is_err() {
        // The cleanup re-entered a panic. Fail closed without touching the
        // engine again: latch the process-wide barrier and set the loop's own
        // mode by hand, skipping the staff broadcast (it reads the world).
        persistence_integrity.latch();
        if !health.mode.integrity_frozen() {
            health.mode = WorldMode::IntegrityFrozen {
                incident: incident_id("world-task-panic-cleanup", &detail),
            };
        }
        tracing::error!(
            site = fault.site,
            "world-task fault cleanup panicked; freezing without further engine calls"
        );
        return fault;
    }
    *out = Outbox::new();
    *fx = Effects::new();
    apply_integrity_freeze(
        &engine.world,
        gateway,
        health,
        persistence_integrity,
        &detail,
    );
    fault
}

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

/// A commit receipt (ownership or final acknowledgement) resolved while the
/// loop holds no pending commit.
///
/// The receipt is proof that the writer accepted a batch; the pending commit is
/// the only place the output, effects and typed RAM images that batch belongs
/// to are held. Without it the acknowledgement cannot be released and cannot be
/// re-derived — RAM and the store have diverged in a way this task cannot
/// reconcile. That is the same class of breach as a deterministic store
/// rejection, so it takes the same answer: freeze, tell staff, and keep the
/// loop alive so the freeze is observable and the shutdown path still runs.
/// Asserting instead would end the single writer while the store writer kept
/// its queue and the players kept their connections.
fn freeze_orphan_commit_receipt(
    world: &World,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
    which: &str,
) {
    apply_integrity_freeze(
        world,
        gateway,
        health,
        persistence_integrity,
        &format!("{which} receipt resolved with no pending commit to release it against"),
    );
}

/// Every connected Wizard-or-better — the one staff audience the server fans an
/// alert out to, whether from the world loop (`broadcast_wizards`) or from inside
/// a step (the economy's DL-05 observer, `econ_alerts`).
pub(crate) fn connected_wizards(world: &World) -> Vec<ObjId> {
    world
        .connected()
        .filter(|actor| world.get(*actor).is_some_and(|o| o.role() >= Role::Wizard))
        .collect()
}

/// Send a Sys line to every connected Wizard-or-better, each as a
/// [`ToGateway::Deliver`](mush_ipc::ToGateway) through the seam.
fn broadcast_wizards(world: &World, gateway: &Gateway, text: &str) {
    let ev = Arc::new(OutputEvent::Sys(RichText::tag("system", text)));
    for actor in connected_wizards(world) {
        gateway.deliver(SendTarget::Char(u64::from(actor.0)), Arc::clone(&ev));
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
    writer: &StoreWriter,
    serial: &mut u64,
    health: &mut StoreHealth,
    metrics: &Metrics,
    retained: &mut RetainedSeals,
    high_water: &AtomicU64,
    quarantine_spool: &crate::quarantine::QuarantineSpool,
    persistence_integrity: &PersistenceIntegrityLatch,
) -> FlushOutcome {
    if engine.has_staged_action() {
        flush_prepared_action(engine, writer, health, metrics, retained, high_water).await
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
/// images for the matching acknowledgement. There is one lane: the caller
/// transfers a complete exact-token bundle to the FIFO writer and installs
/// only after its matching durable receipt. This path deliberately takes no
/// writer serial — a K3 commit is named by its own token, and the legacy
/// serial belongs to `flush_dirty` alone.
async fn flush_prepared_action(
    engine: &mut Engine,
    writer: &StoreWriter,
    health: &mut StoreHealth,
    _metrics: &Metrics,
    retained: &mut RetainedSeals,
    high_water: &AtomicU64,
) -> FlushOutcome {
    let staged: StagedAction = match engine.take_staged_action() {
        Ok(Some(staged)) => staged,
        // `flush_engine_step` selects this lane only when a staged action is
        // in hand. Nothing staged means the selection and the engine disagree:
        // refuse the flush, which leaves the step unpersisted and un-released
        // rather than claiming a durable write nobody built.
        Ok(None) => {
            tracing::error!("prepared flush selected with no staged action; refusing");
            return FlushOutcome::Refused(
                "prepared flush selected with no staged action".to_owned(),
            );
        }
        Err(error) => return FlushOutcome::Refused(error.to_string()),
    };
    let (bundle, installation) = staged.into_parts();

    // Staging checks this invariant once, but later code in the same engine
    // step must not be able to add legacy dirtiness and silently move it to a
    // subsequent commit. Recheck at the last boundary before writer custody.
    if engine.has_legacy_persistence_work() {
        release_prepared_installation(
            engine,
            installation,
            "flush_prepared_action/legacy-persistence-work",
        );
        return FlushOutcome::Rejected(
            "prepared action reached submission alongside legacy persistence work".to_owned(),
        );
    }

    // An older unacknowledged ledger seal is a durability predecessor. It may
    // not be folded into, or overtaken by, this state-family transaction.
    retained.ack(high_water.load(Ordering::Relaxed));
    if !retained.is_empty() {
        release_prepared_installation(
            engine,
            installation,
            "flush_prepared_action/retained-ledger-predecessor",
        );
        return FlushOutcome::Rejected(
            "prepared action reached submission with an unresolved legacy ledger predecessor"
                .to_owned(),
        );
    }
    let expected_token = bundle.token();
    let expected_hash = bundle.bundle_hash();
    let (ownership_notify, ownership) = oneshot::channel();
    let (notify, receiver) = oneshot::channel();
    if writer
        .send(StoreJob::Authoritative {
            bundle,
            ownership: ownership_notify,
            notify,
        })
        .await
        .is_err()
    {
        release_prepared_installation(
            engine,
            installation,
            "flush_prepared_action/writer-gone-authoritative",
        );
        tracing::error!(
            %expected_token,
            "store writer gone before accepting authoritative action"
        );
        return FlushOutcome::WriterGone;
    }
    let receipt = PendingReceipt::Authoritative {
        expected_token,
        expected_hash,
        ownership: Some(ownership),
        receiver,
    };
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
    writer: &StoreWriter,
    serial: &mut u64,
    durable: bool,
    health: &mut StoreHealth,
    _metrics: &Metrics,
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
        persist::preserve_unqueued_batch(quarantine_spool, &batch, &detail).await;
        return FlushOutcome::Rejected(detail);
    }
    // Bind acknowledgements to the actual nonempty batch, regardless of which
    // wake produced it. Private proving/bench precharges can originate from a
    // command or maintenance tick with no live arena traffic at all. A rejected
    // or uncommitted batch cannot advance the durable high-water.
    if let Some(registry) = engine
        .world
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
    {
        registry.tasks().stamp_ledger_writes(*serial);
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
            // The batch this call built is inside the returned job; another
            // variant means it is not in hand and cannot be preserved to the
            // quarantine spool. Nothing here can recover it, so the incident
            // id is the whole record of the loss — and the caller still hears
            // `WriterGone`, so the write is never confirmed as durable.
            tracing::error!(
                serial = *serial,
                incident = incident_id(
                    "store-writer-rejected-foreign-job",
                    "flush_dirty send returned a job that is not a write",
                )
                .0,
                "store writer returned a non-write job; the authoritative batch is unrecoverable"
            );
            return FlushOutcome::WriterGone;
        };
        tracing::error!(
            serial = *serial,
            "store writer gone; batch dropped (teardown)"
        );
        persist::preserve_unqueued_batch(
            quarantine_spool,
            &batch,
            "store writer queue closed before accepting authoritative batch",
        )
        .await;
        return FlushOutcome::WriterGone;
    }
    // StoreWriter acquired the matching lane permit and published queue depth
    // before returning; it holds both until the writer finishes this job.
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
                // Staff ops are typed, so they never classify as a command
                // line and never need the read-only allowlist. An unresolved
                // admission is suppressed: not running the op is the fail-safe
                // half of a staff mutation.
                tracing::error!(
                    actor = %op.actor,
                    "typed staff operation hit command-text admission; suppressed"
                );
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
            // Internal deadlines and registered continuations are ephemeral;
            // their private request arguments are not shown in @ps.
            ScheduledKind::RequestTimeout { .. }
            | ScheduledKind::CommandContinuation(_)
            | ScheduledKind::ConnectionCompletion(_) => {}
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
/// The NPC indicator (H2a): `active` is every NPC with a live runtime;
/// `thinking` is every dialogue turn in flight **plus** every non-terminal
/// durable decision (either kind of thinking); the census splits the
/// decisions by state for `mush_npc_decisions{state}`.
fn npc_counts(engine: &Engine) -> (i64, i64, mush_core::DecisionStatusCensus) {
    let (active, in_flight) = match engine.world.module_state::<mush_npc::NpcHostState>() {
        Some(h) => (
            h.runtimes.len() as i64,
            h.runtimes.values().filter(|rt| rt.in_flight).count() as i64,
        ),
        None => (0, 0),
    };
    let census = decision_census(engine);
    let thinking =
        in_flight.saturating_add(i64::try_from(census.nonterminal()).unwrap_or(i64::MAX));
    (active, thinking, census)
}

/// J1 (doc 12 D-12-1, D-12-3): the economy journal's size, its growth since
/// boot, and what the last cold replay cost — the numbers the checkpoint
/// decision keys on. Zero throughout when the economy module is not composed.
#[derive(Clone, Copy, Debug, Default)]
struct EconomyJournalSample {
    entries: i64,
    per_hour: i64,
    replay_us_per_entry: i64,
    replay_ms: i64,
    fetch_ms: i64,
}

fn economy_journal(engine: &Engine) -> EconomyJournalSample {
    let Some(store) = engine.world.module_state::<mush_economy::EconomyStore>() else {
        return EconomyJournalSample::default();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let uptime = now.saturating_sub(store.last_boot()).max(1);
    let posts = i64::try_from(store.posts_since_boot()).unwrap_or(i64::MAX);
    let per_hour = posts.saturating_mul(3_600) / uptime;
    let replay = store.boot_replay();
    let replay_ms = replay.map_or(0, |r| i64::try_from(r.elapsed_ms).unwrap_or(i64::MAX));
    let fetch_ms = replay.map_or(0, |r| i64::try_from(r.fetch_ms).unwrap_or(i64::MAX));
    let replay_us_per_entry = replay.filter(|r| r.entries > 0).map_or(0, |r| {
        let entries = u64::try_from(r.entries).unwrap_or(u64::MAX);
        i64::try_from(r.elapsed_ms.saturating_mul(1_000) / entries).unwrap_or(i64::MAX)
    });
    EconomyJournalSample {
        entries: i64::try_from(store.journal_len()).unwrap_or(i64::MAX),
        per_hour,
        replay_us_per_entry,
        replay_ms,
        fetch_ms,
    }
}

/// Every composed decision source's pending-state census, page by page. A
/// scan fault (a malformed record) reports zero rather than freezing a
/// metrics sample; the boot audit is what refuses such a record.
fn decision_census(engine: &Engine) -> mush_core::DecisionStatusCensus {
    let mut census = mush_core::DecisionStatusCensus::default();
    if !engine
        .registry
        .decision_sources()
        .is_some_and(|sources| !sources.is_empty())
    {
        return census;
    }
    let mut cursor = None;
    loop {
        let Ok(page) = engine
            .discover_decision_dispatches(cursor.as_ref(), mush_core::MAX_DECISION_RECOVERY_PAGE)
        else {
            return mush_core::DecisionStatusCensus::default();
        };
        census.merge(page.status_census());
        cursor = page.next_cursor().cloned();
        if cursor.is_none() {
            return census;
        }
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
    let (npc_active, npc_thinking, decisions) = npc_counts(engine);
    let economy = economy_journal(engine);
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
        npc_decisions_ready: i64::try_from(decisions.ready).unwrap_or(i64::MAX),
        npc_decisions_leased: i64::try_from(decisions.leased).unwrap_or(i64::MAX),
        npc_decisions_resolving: i64::try_from(decisions.resolving + decisions.fallback_ready)
            .unwrap_or(i64::MAX),
        economy_journal_entries: economy.entries,
        economy_journal_per_hour: economy.per_hour,
        economy_replay_us_per_entry: economy.replay_us_per_entry,
        economy_replay_ms: economy.replay_ms,
        economy_fetch_ms: economy.fetch_ms,
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
    /// H2a: durable NPC decisions by state — ready, leased, resolving.
    pub npc_decisions: [i64; 3],
    /// Approximate p99 loop lag (ms) from the histogram.
    pub lag_p99: u64,
    /// The authoritative lane (H3): whether the single pending-intent slot is
    /// held right now.
    pub auth_slot_held: bool,
    /// Most recent authoritative slot hold, ms.
    pub auth_hold_last_ms: u64,
    /// Approximate p99 authoritative slot hold, ms.
    pub auth_hold_p99_ms: u64,
    /// Longest authoritative slot hold since boot, ms.
    pub auth_hold_max_ms: u64,
    /// Most recent authoritative bundle's canonical encoded size, bytes.
    pub auth_last_bundle_bytes: u64,
    /// Most recent authoritative bundle's replay-write count.
    pub auth_last_replay_writes: u64,
    /// Settled authoritative commits since boot: finalized, refused,
    /// integrity, quarantined.
    pub auth_commits: [u64; 4],
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
    /// J1 (doc 12 D-12-1): the economy journal — entries held, posted per hour
    /// since boot, the last replay's µs per entry, the last replay ms, the
    /// last fetch ms.
    pub economy_journal: [i64; 5],
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
        npc_decisions: [
            snap.npc_decisions_ready,
            snap.npc_decisions_leased,
            snap.npc_decisions_resolving,
        ],
        lag_p99: snap.loop_lag_p99_ms(),
        auth_slot_held: snap.auth_slot_held,
        auth_hold_last_ms: snap.auth_hold_last_ms,
        auth_hold_p99_ms: snap.authoritative_hold_p99_ms(),
        auth_hold_max_ms: snap.auth_hold_max_ms,
        auth_last_bundle_bytes: snap.auth_bytes_last,
        auth_last_replay_writes: snap.auth_writes_last,
        auth_commits: [
            snap.auth_commits_finalized_total,
            snap.auth_commits_refused_total,
            snap.auth_commits_integrity_total,
            snap.auth_commits_quarantined_total,
        ],
        ipc_frames_shed: snap.ipc_frames_shed_total,
        ipc_write_timeouts: snap.ipc_write_timeouts_total,
        ipc_to_gw_overflow: snap.ipc_to_gw_overflow_total,
        ipc_hw_to_server: snap.ipc_queue_high_water_to_server,
        ipc_hw_to_gw: snap.ipc_queue_high_water_to_gw,
        llm: llm_line,
        economy_journal: [
            snap.economy_journal_entries,
            snap.economy_journal_per_hour,
            snap.economy_replay_us_per_entry,
            snap.economy_replay_ms,
            snap.economy_fetch_ms,
        ],
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
    // H3: the authoritative lane — the one pending-intent slot every
    // authoritative commit serializes on. Aggregates only.
    text.push_str(&format!(
        "\nAuthoritative lane: slot {}; hold last {}ms, p99 {}ms, max {}ms; last bundle {} bytes / {} writes; commits {} finalized, {} refused, {} integrity, {} quarantined",
        if extra.auth_slot_held { "HELD" } else { "free" },
        extra.auth_hold_last_ms,
        extra.auth_hold_p99_ms,
        extra.auth_hold_max_ms,
        extra.auth_last_bundle_bytes,
        extra.auth_last_replay_writes,
        extra.auth_commits[0],
        extra.auth_commits[1],
        extra.auth_commits[2],
        extra.auth_commits[3],
    ));
    // J1 (doc 12 D-12-1): the economy journal, what its replay cost, and the
    // projection the checkpoint decision keys on — the entry line derived from
    // the measured per-entry cost, and how far off it is at the current rate.
    let [entries, per_hour, us_per_entry, replay_ms, fetch_ms] = extra.economy_journal;
    let line_secs = i64::try_from(mush_economy::CHECKPOINT_REVIEW_SECS).unwrap_or(i64::MAX);
    let line_entries = if us_per_entry > 0 {
        line_secs.saturating_mul(1_000_000) / us_per_entry
    } else {
        0
    };
    let projection = if us_per_entry == 0 {
        "no replay measured".to_owned()
    } else if line_entries <= entries {
        "past the line".to_owned()
    } else if per_hour > 0 {
        format!(
            "~{} days away at this rate",
            (line_entries - entries) / per_hour.saturating_mul(24)
        )
    } else {
        "no posts since boot".to_owned()
    };
    text.push_str(&format!(
        "\nEconomy journal: {entries} entries, {per_hour}/h since boot; last replay {replay_ms}ms ({us_per_entry}us/entry; fetch {fetch_ms}ms); the {line_secs}s line is ~{line_entries} entries: {projection}"
    ));
    text.push_str(&format!(
        "\nNPC agents: {} active, {} thinking",
        extra.npc_active, extra.npc_thinking
    ));
    text.push_str(&format!(
        "\nNPC decisions: {} ready, {} leased, {} resolving",
        extra.npc_decisions[0], extra.npc_decisions[1], extra.npc_decisions[2]
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
    writer: &StoreWriter,
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
            StaffOpKind::ClearBan { ref ip } | StaffOpKind::LiftTempban { ref ip, .. } => {
                tracing::info!(%actor, %ip, operation = ?op.kind, "@siteban pardon requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(
                        gateway,
                        actor,
                        "@siteban pardon failed: staff executor gone.",
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
            // The Discord identity family (D1): the operational trace carries
            // the actor and the target character only — never a code, never
            // an identity (the privileges audit is the record of the act).
            StaffOpKind::DiscordStatus { .. }
            | StaffOpKind::DiscordLinkMint { .. }
            | StaffOpKind::DiscordUnlinkSelf { .. }
            | StaffOpKind::DiscordSet { .. } => {
                tracing::info!(%actor, kind = ?op.kind, "@discord self-service op requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
                }
            }
            // D3: the follow verbs are the gateway bridge's to answer; the
            // control crosses the seam fire-and-forget and the bridge tells the
            // character what happened on their own connections.
            StaffOpKind::DiscordFollow { actor: subject, on } => {
                tracing::info!(%actor, on, "@discord/follow requested");
                gateway.discord_control(
                    u64::from(subject.0),
                    if on {
                        DiscordControlOp::Follow
                    } else {
                        DiscordControlOp::Unfollow
                    },
                );
                send_sys(
                    gateway,
                    actor,
                    if on {
                        "@discord/follow: asked the Discord bot to follow you here; it answers \
                         on this screen and in your DM."
                    } else {
                        "@discord/unfollow: asked the Discord bot to detach."
                    },
                );
            }
            StaffOpKind::DiscordShow { target, .. }
            | StaffOpKind::DiscordUnlink { target, .. }
            | StaffOpKind::DiscordBan { target, .. } => {
                tracing::info!(%actor, %target, kind = ?op.kind, "@discord staff op requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
                }
            }
            StaffOpKind::DiscordUnban { .. } => {
                tracing::info!(%actor, "@discord/unban requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(gateway, actor, "@discord failed: staff executor gone.");
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
    let world = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    });
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

/// **One account record's immutable id** (16b cut F.4): the stored one, else
/// minted from `(name, created)` — the same derivation
/// [`crate::accounts_identity::hydrate`] uses, so a character linked at run time
/// and the same character re-hydrated at the next boot resolve to one id.
///
/// `None` only when the mint itself fails, which leaves the character unmapped
/// and failing closed (and logged by the caller).
fn account_id_of(rec: &AccountRec) -> Option<mush_core::AccountId> {
    rec.id
        .or_else(|| mush_core::AccountId::mint(&rec.name, rec.created).ok())
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
/// a genuine store failure, not a normal path. **16b cut F.4** adds the one
/// thing that window now needs: the create turn told the live account oracle
/// about the character, so a failed link also sends
/// [`WorldHandle::unlink_player`] and the orphan is an orphan on both sides.
/// The confirmation is returned only after the link write returns (N3,
/// ack-gated). The caller is NOT switched to the new character (N4): they keep
/// playing their current one.
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
    let Some(world) = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    }) else {
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
    // **16b cut F.4**: the caller's own account id rides with it, so the alt is
    // mapped to the SAME account before this reply prints — which is what makes
    // "one slot per account" hold for an alt made mid-session, and what stops the
    // alt being told its account is not on record.
    let id = match world.create_player(name, account_id_of(&account)).await {
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
            // **16b cut F.4**: and the live oracle gives the mapping back, for
            // the same reason `create_and_link` does — the document it was
            // inserted against does not exist, so the alt must fail closed on
            // every account policy the way an unlinked character always has.
            match world.unlink_player(id).await {
                Ok(_) => {}
                Err(error) => tracing::error!(
                    account = %account.name, %id, name, %error,
                    "@newchar: the account oracle did not give the unlinked character back; it \
                     stays mapped until the next boot re-derives the map"
                ),
            }
            format!(
                "@newchar: created the character, but linking it to your account failed: {e}. \
                 Please contact staff."
            )
        }
    }
}

/// The sentence appended to every acknowledged `@siteban` set/clear.
///
/// The confirmation is gated on the STORE acknowledgement, never on the
/// gateway: this half's own [`IpGuard`] is mirrored immediately, but in a
/// forked deployment the guard that admits connections lives in the gateway
/// process and re-seeds itself from the store on its own clock
/// ([`SITEBAN_REFRESH`]). Staff who are told "banned" and then watch the
/// address connect for another few seconds should know why before they file it
/// as a bug.
fn siteban_latency_note() -> String {
    format!(
        "Connection gateways re-read the ban list every {}s, so it can take that long to bite.",
        SITEBAN_REFRESH.as_secs()
    )
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
                    let head = match until_unix {
                        None => format!("@siteban: {ip} banned permanently."),
                        Some(until) => format!("@siteban: {ip} banned until unix {until}."),
                    };
                    format!("{head} {}", siteban_latency_note())
                }
                Err(e) => format!("@siteban failed: {e}"),
            },
            StaffOpKind::ClearBan { ip } => match store.clear_ban(&ip.to_string()).await {
                Ok(removed) => {
                    guard.clear_siteban(ip);
                    if removed {
                        format!("@siteban: ban on {ip} lifted. {}", siteban_latency_note())
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
            StaffOpKind::LiftTempban { ip, issued_unix_ms } => {
                let lift = mush_store::TempbanLift::new(ip, issued_unix_ms);
                match store.put_tempban_lift(lift.clone()).await {
                    Ok(()) => {
                        guard.apply_tempban_lift(
                            ip,
                            &lift.id,
                            lift.issued_unix_ms,
                            lift.expires_unix,
                        );
                        format!(
                            "@siteban/lift: password-tempban pardon published for {ip}'s network. Sitebans remain in force. {}",
                            siteban_latency_note()
                        )
                    }
                    Err(e) => format!("@siteban/lift failed: {e}"),
                }
            }
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
            StaffOpKind::DiscordStatus { actor: subject } => {
                run_discord_status(&*store, subject).await
            }
            StaffOpKind::DiscordLinkMint { actor: subject } => {
                run_discord_link_mint(&*store, subject).await
            }
            StaffOpKind::DiscordUnlinkSelf { actor: subject } => {
                run_discord_unlink(&gateway, &*store, subject, false).await
            }
            // Answered on the world loop through the gateway control (D3); it is
            // never queued to this executor, so this arm only keeps the match
            // honest.
            StaffOpKind::DiscordFollow { on, .. } => format!(
                "@discord/{}: routed to the Discord bridge.",
                if on { "follow" } else { "unfollow" }
            ),
            StaffOpKind::DiscordSet {
                actor: subject,
                setting,
            } => {
                // `quiet` also applies live to a follow in progress (D3); the
                // store carries the preference for the next one.
                let live = match setting {
                    crate::staff::DiscordSetting::Quiet(on) => Some(on),
                    _ => None,
                };
                let text = run_discord_set(&*store, subject, setting).await;
                if let Some(on) = live
                    && text.starts_with("@discord/set:")
                {
                    gateway.discord_control(u64::from(subject.0), DiscordControlOp::Quiet(on));
                }
                text
            }
            StaffOpKind::DiscordShow { actor: by, target } => {
                match discord_show_recheck(&world, by, target).await {
                    Ok(()) => run_discord_show(&*store, target).await,
                    Err(text) => text,
                }
            }
            StaffOpKind::DiscordUnlink { target, .. } => {
                run_discord_unlink(&gateway, &*store, target, true).await
            }
            StaffOpKind::DiscordBan {
                actor: by,
                target,
                reason,
            } => run_discord_ban(&gateway, &*store, by, target, &reason).await,
            StaffOpKind::DiscordUnban { target, .. } => run_discord_unban(&*store, target).await,
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

// ---------------------------------------------------------------------------
// The Discord identity family (D1): store-side executors. Every reply below
// goes to the requesting actor alone; the only reply that renders an identity
// is `run_discord_show`, whose caller was privilege-gated and audited.
// ---------------------------------------------------------------------------

/// The display name of a character on an account record, or its dbref.
fn account_char_name(account: &AccountRec, id: ObjId) -> String {
    account
        .characters
        .iter()
        .find(|c| c.id == id)
        .map_or_else(|| id.to_string(), |c| c.name.clone())
}

/// `@discord` — the caller's own link status, identity-free.
async fn run_discord_status(store: &dyn ObjectStore, subject: ObjId) -> String {
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord failed: {e}"),
    };
    match &account.discord {
        None => "@discord: your account has no Discord link. `@discord/link` mints a code to \
                 bind one (see `help discord`)."
            .to_string(),
        Some(link) => {
            let relay = link.relay_character.map_or_else(
                || "your most recently connected character".to_string(),
                |c| account_char_name(&account, c),
            );
            let banned = match store.discord_ban_get(link.user_id).await {
                Ok(Some(_)) => "  BANNED: your Discord user may not link, follow, or relay.",
                _ => "",
            };
            format!(
                "@discord: linked since unix {}; auto-follow {}; quiet {}; relay character: \
                 {relay}.{banned}",
                link.linked_at,
                if link.auto_follow { "on" } else { "off" },
                if link.quiet { "on" } else { "off" },
            )
        }
    }
}

/// `@discord/link` — mint a one-shot code, store its hash with a TTL, and hand
/// the code to the caller alone. The code never enters a log.
async fn run_discord_link_mint(store: &dyn ObjectStore, subject: ObjId) -> String {
    use mush_config::discord::{
        LINK_CODE_TTL_S, generate_link_code, link_code_hash, normalize_link_code,
    };
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord/link failed: {e}"),
    };
    if account.discord.is_some() {
        return "@discord/link: your account is already linked to a Discord user; \
                `@discord/unlink` first."
            .to_string();
    }
    let code = generate_link_code();
    let normalized = normalize_link_code(&code).expect("a minted code is well-formed");
    let hash = link_code_hash(&normalized);
    let expires = unix_now_s().saturating_add(LINK_CODE_TTL_S);
    match store
        .discord_code_put(
            hash,
            DiscordCodeRecord {
                account: account.name.clone(),
                expires,
            },
        )
        .await
    {
        Ok(()) => {
            tracing::info!(account = %account.name, "@discord/link code minted");
            format!(
                "@discord/link: your code is {code}. In Discord, send the game's bot a direct \
                 message: /link {code} — within {} minutes. The code works once; never share it.",
                LINK_CODE_TTL_S / 60
            )
        }
        Err(e) => format!("@discord/link failed: {e}"),
    }
}

/// `@discord/unlink` (self) and `@discord/unlink <player>` (staff): clear the
/// link and release the reverse index.
async fn run_discord_unlink(
    gateway: &Gateway,
    store: &dyn ObjectStore,
    target: ObjId,
    by_staff: bool,
) -> String {
    let whose = if by_staff {
        "that account"
    } else {
        "your account"
    };
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return format!("@discord/unlink: {target} is not linked to an account.");
        }
        Err(e) => return format!("@discord/unlink failed: {e}"),
    };
    let Some(link) = account.discord.as_ref() else {
        return format!("@discord/unlink: {whose} has no Discord link.");
    };
    match store.set_account_discord(&account.name, None).await {
        Ok(true) => {}
        Ok(false) => {
            return format!(
                "@discord/unlink failed: account {} vanished mid-update.",
                account.name
            );
        }
        Err(e) => return format!("@discord/unlink failed: {e}"),
    }
    for character in &account.characters {
        gateway.discord_control(
            u64::from(character.id.0),
            mush_ipc::DiscordControlOp::Revoke,
        );
    }
    if let Err(e) = store.discord_user_release(link.user_id).await {
        // The doc is unlinked; the index is what stops a re-link elsewhere.
        return format!(
            "@discord/unlink: {whose} is unlinked, but releasing the index failed: {e}"
        );
    }
    tracing::info!(account = %account.name, by_staff, "@discord/unlink applied");
    format!("@discord/unlink: {whose} is no longer linked to Discord.")
}

/// `@discord/set` — a self-service preference on the caller's own link.
async fn run_discord_set(
    store: &dyn ObjectStore,
    subject: ObjId,
    setting: crate::staff::DiscordSetting,
) -> String {
    use crate::staff::DiscordSetting;
    let account = match account_of_actor(store, subject).await {
        Ok(Some(a)) => a,
        Ok(None) => return "@discord: your character is not linked to an account.".to_string(),
        Err(e) => return format!("@discord/set failed: {e}"),
    };
    let Some(mut link) = account.discord.clone() else {
        return "@discord/set: your account has no Discord link yet (`@discord/link`).".to_string();
    };
    let what = match setting {
        DiscordSetting::RelayCharacter(Some(c)) => {
            // Only the caller's own characters may speak for the caller.
            if !account.characters.iter().any(|ch| ch.id == c) {
                return "@discord/set: the relay character must be one of your own characters."
                    .to_string();
            }
            link.relay_character = Some(c);
            format!("relay character set to {}", account_char_name(&account, c))
        }
        DiscordSetting::RelayCharacter(None) => {
            link.relay_character = None;
            "relay character reset to your most recently connected character".to_string()
        }
        DiscordSetting::AutoFollow(on) => {
            link.auto_follow = on;
            format!("auto-follow {}", if on { "on" } else { "off" })
        }
        DiscordSetting::Quiet(on) => {
            link.quiet = on;
            format!(
                "quiet {} (applies from your next follow)",
                if on { "on" } else { "off" }
            )
        }
    };
    match store.set_account_discord(&account.name, Some(&link)).await {
        Ok(true) => format!("@discord/set: {what}."),
        Ok(false) => format!(
            "@discord/set failed: account {} vanished mid-update.",
            account.name
        ),
        Err(e) => format!("@discord/set failed: {e}"),
    }
}

/// The execution-time authorization for `@discord/show`: the privilege was
/// tested on the world loop and the `use` audited **before** the op was
/// queued, and this executor is a queue hop later with no world borrow of its
/// own — so `@privilege/revoke` can land in between and the disclosure would
/// still go out. Round-trip one `WorldMsg` (the `@newpassword` precedent) and
/// let the world loop answer.
///
/// Fail closed: a revocation, a busy world (a commit is pending) and a gone
/// world all refuse. The world loop writes the second `Denied` audit row on
/// the revocation; `Err` here is a refusal to disclose, never a panic — this
/// module denies `clippy::unreachable` and the loop must not be lost over a
/// staff op.
async fn discord_show_recheck(
    world: &mpsc::WeakSender<WorldMsg>,
    actor: ObjId,
    target: ObjId,
) -> Result<(), String> {
    let Some(world) = world.upgrade().map(|tx| WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    }) else {
        return Err("@discord/show: the world is shutting down; nothing was shown.".to_string());
    };
    match world
        .recheck_privilege(
            actor,
            Privileges::DISCORD_IDENTITY,
            Some(target),
            "@discord/show",
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => {
            tracing::warn!(
                %actor, %target,
                "@discord/show refused at execution: the privilege was revoked after the command"
            );
            Err("@discord/show: your DISCORD_IDENTITY privilege was revoked before the                  look could be answered. Permission denied."
                .to_string())
        }
        Err(e) => Err(format!(
            "@discord/show: the privilege could not be re-checked ({e}); nothing was shown."
        )),
    }
}

/// `@discord/show <player>` — the one surface that renders the identity. The
/// caller held `DISCORD_IDENTITY` and the `use` is already in the audit.
async fn run_discord_show(store: &dyn ObjectStore, target: ObjId) -> String {
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => return format!("@discord/show: {target} is not linked to an account."),
        Err(e) => return format!("@discord/show failed: {e}"),
    };
    let who = account_char_name(&account, target);
    let Some(link) = account.discord.as_ref() else {
        return format!(
            "@discord/show: {who} (account {}) has no Discord link.",
            account.name
        );
    };
    let relay = link.relay_character.map_or_else(
        || "most recent".to_string(),
        |c| account_char_name(&account, c),
    );
    let ban = match store.discord_ban_get(link.user_id).await {
        Ok(Some(b)) => format!(
            "  BANNED by {} at unix {}: {}",
            b.by_account, b.at, b.reason
        ),
        Ok(None) => String::new(),
        Err(e) => format!("  (ban lookup failed: {e})"),
    };
    format!(
        "@discord/show: {who} (account {}): Discord user {}, linked unix {}, auto-follow {}, \
         relay character {relay}.{ban}",
        account.name,
        link.user_id,
        link.linked_at,
        if link.auto_follow { "on" } else { "off" },
    )
}

/// `@discord/ban <player>=<reason>` — ban the target's linked Discord user by
/// snowflake (the link stays so `@discord/unban <player>` can find it; the
/// bridge refuses link, follow and relay while the ban stands).
async fn run_discord_ban(
    gateway: &Gateway,
    store: &dyn ObjectStore,
    by: ObjId,
    target: ObjId,
    reason: &str,
) -> String {
    let account = match account_of_actor(store, target).await {
        Ok(Some(a)) => a,
        Ok(None) => return format!("@discord/ban: {target} is not linked to an account."),
        Err(e) => return format!("@discord/ban failed: {e}"),
    };
    let who = account_char_name(&account, target);
    let Some(link) = account.discord.as_ref() else {
        return format!("@discord/ban: {who} has no linked Discord user to ban.");
    };
    let by_account = match account_of_actor(store, by).await {
        Ok(Some(a)) => a.name,
        _ => by.to_string(),
    };
    match store
        .discord_ban_put(DiscordBanRec {
            user_id: link.user_id,
            by_account,
            reason: reason.to_string(),
            at: unix_now_s(),
        })
        .await
    {
        Ok(()) => {
            for character in &account.characters {
                gateway.discord_control(
                    u64::from(character.id.0),
                    mush_ipc::DiscordControlOp::Revoke,
                );
            }
            tracing::info!(account = %account.name, "@discord/ban applied");
            format!(
                "@discord/ban: {who}'s Discord user is banned — they can no longer link, \
                 follow, or relay. `@discord/unban {who}` lifts it."
            )
        }
        Err(e) => format!("@discord/ban failed: {e}"),
    }
}

/// `@discord/unban <player>|<snowflake>`.
async fn run_discord_unban(
    store: &dyn ObjectStore,
    target: crate::staff::DiscordBanTarget,
) -> String {
    use crate::staff::DiscordBanTarget;
    let (user_id, who) = match target {
        DiscordBanTarget::User(id) => (id, "that Discord user".to_string()),
        DiscordBanTarget::Player(t) => {
            let account = match account_of_actor(store, t).await {
                Ok(Some(a)) => a,
                Ok(None) => return format!("@discord/unban: {t} is not linked to an account."),
                Err(e) => return format!("@discord/unban failed: {e}"),
            };
            let who = account_char_name(&account, t);
            let Some(link) = account.discord.as_ref() else {
                return format!(
                    "@discord/unban: {who} has no linked Discord user; if they unlinked while \
                     banned, unban by Discord user id."
                );
            };
            (link.user_id, format!("{who}'s Discord user"))
        }
    };
    match store.discord_ban_del(user_id).await {
        Ok(true) => format!("@discord/unban: {who} is no longer banned."),
        Ok(false) => format!("@discord/unban: {who} was not banned."),
        Err(e) => format!("@discord/unban failed: {e}"),
    }
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

/// **Tell the account oracle about a character just created** (16b cut F.4).
///
/// One instance stands behind `Engine`'s oracle and `mush_sim::AccountOracle`,
/// so this one call is what both readers see. Every outcome but success is
/// logged at a level an operator will find, because the consequence is a
/// character that fails closed on every account-scoped surface — and the player
/// is told "your account has no record here", which is only a useful sentence if
/// the log says why.
fn link_created_character(
    engine: &Engine,
    id: ObjId,
    name: &str,
    account: Option<mush_core::AccountId>,
) {
    let Some(account) = account else {
        tracing::error!(
            %id,
            name,
            "created a character with no account id; it is unmapped and every account policy \
             fails closed on it until the next boot re-derives the map"
        );
        return;
    };
    if !engine.link_account_character(id, account) {
        tracing::error!(
            %id,
            name,
            "the account oracle did not take the new character; it is unmapped and fails closed"
        );
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
    resolve_classified_sends(
        reg,
        world,
        out,
        TypedStateLens::Detached(mush_core::DetachedTypedState::Test),
    )
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
    lens: TypedStateLens<'_>,
) -> Vec<ClassifiedSend> {
    let mut sends = Vec::new();
    for (class, audience, event) in out.take_classified() {
        let ev = Arc::new(event);
        // A spoofed emit keeps its `Spoof` kind only for a listener entitled
        // to see its source (NOSPOOF, or Royalty+); everyone else receives
        // the bare `Emit`. Resolved here, per listener, because this is the
        // one place an audience becomes recipients.
        let for_listener = |listener: ObjId, ev: &Arc<OutputEvent>| -> Arc<OutputEvent> {
            match spoof_for_listener(world, listener, ev) {
                Some(downgraded) => Arc::new(downgraded),
                None => Arc::clone(ev),
            }
        };
        match audience {
            Audience::Actor(actor) => {
                let ev = for_listener(actor, &ev);
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
                        // The live/absent choice is the caller's and it stays
                        // explicit: every production step hands this its own
                        // `Live` store, and a caller without one has to name
                        // the reason rather than silently taking the
                        // store-less verdict.
                        && !can_perceive(reg, world, occupant, subject, lens)
                    {
                        continue;
                    }
                    sends.push((
                        class,
                        SendTarget::Char(u64::from(occupant.0)),
                        for_listener(occupant, &ev),
                    ));
                }
            }
            Audience::All => sends.push((class, SendTarget::All, ev)),
            // A relay sink is not a character: no recipient gate. The flag on
            // the channel is the staff's declaration that its lines may leave
            // the world (programme D4).
            Audience::Relay(name) => sends.push((class, SendTarget::Relay(name), ev)),
        }
    }
    sends
}

/// Whether a failed step's own sends already answer its failure, so the
/// generic commit-failure sentence would only repeat it: any `BestEffort` or
/// `OnAbort` send, except a connection's `OUTPUTPREFIX`/`OUTPUTSUFFIX` line
/// (16b cut AP.4) — framing round an answer is not one.
fn answers_an_abort(sends: &[ClassifiedSend]) -> bool {
    sends.iter().any(|(class, _, event)| {
        matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort)
            && !is_output_bracket(event)
    })
}

/// A connection's `OUTPUTPREFIX` or `OUTPUTSUFFIX` line (16b cut AP.4): a
/// `Sys` whose first span carries one of the two tags `Engine::step` gives it.
fn is_output_bracket(event: &OutputEvent) -> bool {
    match event {
        OutputEvent::Sys(text) => text.spans().first().is_some_and(|span| {
            matches!(
                span.tag.as_deref(),
                Some(OUTPUT_PREFIX_TAG | OUTPUT_SUFFIX_TAG)
            )
        }),
        _ => false,
    }
}

/// Split a held batch at its first `Trailer` (a typed line's `OUTPUTSUFFIX`,
/// 16b cut AP.4), `(answer, rest)`, the order kept: a failure path releases
/// the answer, says its own failure sentence, then the rest — the suffix
/// first — so the sentence sits inside the line's pair. Nothing moves: what
/// joined the batch after the suffix (another step's commit-bound sends while
/// the receipt was awaited, a commit's `ActionStateChanged` announce) stays
/// after it, and a batch without a failure sentence goes out exactly as it
/// was held. A batch with no `Trailer` is all answer.
fn split_at_trailer(mut sends: Vec<ClassifiedSend>) -> (Vec<ClassifiedSend>, Vec<ClassifiedSend>) {
    let at = sends
        .iter()
        .position(|(class, _, _)| *class == DeliveryClass::Trailer)
        .unwrap_or(sends.len());
    let rest = sends.split_off(at);
    (sends, rest)
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
    store: &Arc<dyn ObjectStore>,
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
        Admission::Allow => spawn_effect_list(released, tx, llm, store),
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
            // `IngressClass::ExternalEffect` never resolves to the command-text
            // check. If it somehow does, launching nothing is the fail-safe
            // half: a detached effect is the one thing that escapes the loop.
            tracing::error!(
                dropped = released.len(),
                "external effect launch hit command-text admission; suppressed"
            );
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
fn complete_pending_ownership(
    pending: &mut PendingCommit,
    ownership: Result<IntentReceipt, AuthoritativeCommitFailure>,
    world: &World,
    gateway: &Gateway,
    health: &mut StoreHealth,
    persistence_integrity: &PersistenceIntegrityLatch,
) {
    let expected = match &pending.receipt {
        PendingReceipt::Authoritative {
            expected_token,
            expected_hash,
            ..
        } => Some((*expected_token, *expected_hash)),
        PendingReceipt::Legacy { .. } => None,
    };
    let mut result = match (expected, ownership) {
        (Some((token, hash)), Ok(receipt))
            if receipt.token == token && receipt.bundle_hash == hash =>
        {
            Ok(())
        }
        (Some((token, hash)), Ok(receipt)) => Err(AuthoritativeCommitFailure::Integrity(format!(
            "authoritative ownership receipt mismatch: expected {token}/{hash}, received {}/{}",
            receipt.token, receipt.bundle_hash
        ))),
        (Some(_), Err(failure)) => Err(failure),
        (None, _) => Err(AuthoritativeCommitFailure::Integrity(
            "legacy commit produced an authoritative ownership receipt".to_owned(),
        )),
    };

    if let Some(action) = pending.action.as_mut() {
        if result.is_ok()
            && expected.is_some_and(|(token, hash)| {
                action.staged.token != token || action.staged.bundle_hash != hash
            })
        {
            result = Err(AuthoritativeCommitFailure::Integrity(
                "world action identity does not match its authoritative ownership receipt"
                    .to_owned(),
            ));
        }
        action.ownership_valid = result.is_ok();
        if let Some(reply) = action.owned.take() {
            let projected = result
                .as_ref()
                .map(|()| action.staged.clone())
                .map_err(Clone::clone);
            let _ = reply.send(projected);
        }
    }
    if let Err(AuthoritativeCommitFailure::Integrity(detail)) = result {
        apply_integrity_freeze(world, gateway, health, persistence_integrity, &detail);
    }
}

fn fail_action_completion(
    completion: Option<ActionCompletion>,
    failure: AuthoritativeCommitFailure,
) {
    let Some(mut completion) = completion else {
        return;
    };
    if let Some(owned) = completion.owned.take() {
        let _ = owned.send(Err(failure.clone()));
    }
    if let Some(finished) = completion.finished.take() {
        let _ = finished.send(Err(failure));
    }
}

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
    store: &Arc<dyn ObjectStore>,
) -> CompletedPending {
    let disposition_identity = pending.receipt.authoritative_identity();
    let PendingCommit {
        mut sends,
        mut effects,
        failure_actor,
        create,
        mut action,
        prepared_installation,
        barriers,
        coalesced_reattach,
        ..
    } = pending;
    let terminal_failure = ack.as_ref().err().cloned();
    let (outcome, failure, mut integrity_failure) = match ack {
        Ok(()) => (DeliveryOutcome::Committed, None, false),
        Err(AuthoritativeCommitFailure::Refused(detail)) => {
            (DeliveryOutcome::Aborted, Some(detail), false)
        }
        Err(AuthoritativeCommitFailure::Integrity(detail)) => {
            (DeliveryOutcome::Aborted, Some(detail), true)
        }
    };
    let has_abort_output = answers_an_abort(&sends);
    if let Some(installation) = prepared_installation {
        if outcome == DeliveryOutcome::Committed {
            // The exact applied receipt is the only transition that makes the
            // prepared typed post-images live. No provider, validator, codec,
            // serializer, or readiness callback runs here — but the world's
            // observers hear about it (interface plan I13): one
            // `ActionStateChanged` per touched family, emitted after the
            // images are live, whose output and external effects ride this
            // commit's release exactly like the step's own. An observer of
            // this event must NOT persist: a legacy module op queued here
            // would make the next prepared action's flush refuse itself as
            // mixed work (`has_legacy_persistence_work`). The republish
            // observers only emit; the assertion keeps it that way.
            let ops_before = engine.module_ops().len();
            let mut announce_out = Outbox::new();
            let mut announce_fx = Effects::new();
            match engine.install_composed_action_announcing(
                installation,
                &mut announce_out,
                &mut announce_fx,
            ) {
                Ok(()) => {
                    debug_assert_eq!(
                        engine.module_ops().len(),
                        ops_before,
                        "an ActionStateChanged observer queued persistence"
                    );
                    sends.extend(resolve_classified_sends(
                        &engine.registry,
                        &engine.world,
                        &mut announce_out,
                        TypedStateLens::Live(engine.action_state()),
                    ));
                    effects.extend(announce_fx.drain_classified());
                }
                Err(error) => {
                    // A prepared installation that does not match the store it
                    // was minted from is an engine wiring bug, and this runs on
                    // the single world-owning task, which must never panic
                    // (docs/PLAN.md). So persistence freezes instead: the
                    // images never went live, the store checked before it
                    // mutated, and `announce_out`/`announce_fx` are dropped
                    // unread — no observer may be told about a change that did
                    // not happen.
                    integrity_failure = true;
                    apply_integrity_freeze(
                        &engine.world,
                        gateway,
                        health,
                        persistence_integrity,
                        &format!("prepared action-state installation refused: {error}"),
                    );
                }
            }
        } else if integrity_failure {
            // No exact Refused disposition exists. Dropping the installation
            // leaves its typed reservation poisoned; it must never become
            // available for another action while persistence is indeterminate.
            drop(installation);
        } else {
            let observation = action
                .as_mut()
                .and_then(|completion| completion.disposition.take());
            let aborted = match (observation, terminal_failure.as_ref()) {
                (Some(observation), Some(failure)) => {
                    observation.abort_refused(engine, installation, failure, disposition_identity)
                }
                _ => engine
                    .abort_prepared_action(installation)
                    .map_err(|error| error.to_string()),
            };
            if let Err(error) = aborted {
                integrity_failure = true;
                apply_integrity_freeze(
                    &engine.world,
                    gateway,
                    health,
                    persistence_integrity,
                    &format!("prepared action-state abort refused: {error}"),
                );
            }
        }
    }
    if let Some(post_commit) = action
        .as_mut()
        .and_then(|completion| completion.post_commit_decision.take())
        && outcome == DeliveryOutcome::Committed
        && !health.mode.integrity_frozen()
        && let Err(error) = post_commit.reservation.spawn(post_commit.worker)
    {
        apply_integrity_freeze(
            &engine.world,
            gateway,
            health,
            persistence_integrity,
            &format!("committed decision lease could not release worker custody: {error}"),
        );
    }
    let decision_response = action
        .as_mut()
        .and_then(|completion| completion.post_commit_decision_response.take())
        .filter(|_| outcome == DeliveryOutcome::Committed && !health.mode.integrity_frozen());
    if let Some(response) = decision_response.as_ref() {
        tracing::info!(
            owner = %response.fence.controller().owner,
            decision = ?response.fence.decision(),
            revision = response.fence.revision(),
            attempt = response.fence.attempt(),
            request = %response.binding.request,
            request_hash = %response.binding.request_hash,
            "decision response selected action committed; lifecycle binding retained"
        );
    }
    let decision_terminal = action
        .as_mut()
        .and_then(|completion| completion.post_commit_decision_terminal.take())
        .filter(|_| outcome == DeliveryOutcome::Committed && !health.mode.integrity_frozen());
    if let Some(completion) = action.as_mut()
        && let Some(finished) = completion.finished.take()
    {
        let result = if outcome == DeliveryOutcome::Committed && completion.ownership_valid {
            Ok(completion.staged.clone())
        } else {
            Err(terminal_failure.unwrap_or_else(|| {
                AuthoritativeCommitFailure::Integrity(
                    "authoritative action finalized without an exact ownership receipt".to_owned(),
                )
            }))
        };
        let _ = finished.send(result);
    }
    // 16b cut AP.4: a typed line's `OUTPUTSUFFIX` and what joined the batch
    // after it go out after the commit-failure sentence below, so that
    // sentence sits inside the line's pair; on a commit nothing moves.
    let (sends, trailers) = split_at_trailer(sends);
    release_classified_sends(gateway, sends, outcome);
    let effect_mode = if outcome == DeliveryOutcome::Committed && !health.mode.integrity_frozen() {
        // A successful receipt is itself the recovery fact for this batch;
        // StoreStatus(true) may still be queued behind it in the select.
        WorldMode::Writable
    } else {
        health.mode
    };
    release_classified_effects(effect_mode, effects, outcome, tx, llm, store);

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
    release_classified_sends(gateway, trailers, outcome);
    CompletedPending {
        outcome,
        barriers,
        coalesced_reattach,
        decision_response,
        decision_terminal,
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
        let sends = resolve_classified_sends(
            &engine.registry,
            &engine.world,
            &mut reattach_out,
            TypedStateLens::Live(engine.action_state()),
        );
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
    store: &Arc<dyn ObjectStore>,
) {
    let has_abort_output = answers_an_abort(&sends);
    let (sends, trailers) = split_at_trailer(sends);
    release_classified_sends(gateway, sends, DeliveryOutcome::Aborted);
    release_classified_effects(
        health.mode,
        effects,
        DeliveryOutcome::Aborted,
        tx,
        llm,
        store,
    );
    apply_integrity_freeze(world, gateway, health, persistence_integrity, detail);
    if let Some(actor) = failure_actor
        && !has_abort_output
    {
        send_commit_failure(gateway, actor, detail);
    }
    release_classified_sends(gateway, trailers, DeliveryOutcome::Aborted);
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
    store: &Arc<dyn ObjectStore>,
) {
    let has_abort_output = answers_an_abort(&sends);
    let (sends, trailers) = split_at_trailer(sends);
    release_classified_sends(gateway, sends, DeliveryOutcome::Aborted);
    release_classified_effects(
        health.mode,
        effects,
        DeliveryOutcome::Aborted,
        tx,
        llm,
        store,
    );
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
    release_classified_sends(gateway, trailers, DeliveryOutcome::Aborted);
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
    store: &Arc<dyn ObjectStore>,
) {
    for effect in effects {
        match effect {
            // H1: the module store-read seam. Perform the bounded read OFF
            // the world loop against the store handle this task holds, then
            // re-enter as the typed `WorldCommand::StoreReadDone`. The
            // namespace is the requesting module's own (locked by
            // `Ctx::read_store`), one `module_json_get` per key, and the
            // byte/time bounds are decided in `store_read`, never here.
            Effect::StoreRead {
                module,
                actor,
                correlation,
                keys,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(%module, correlation, "world channel closing; store read dropped");
                    continue;
                };
                let store = Arc::clone(store);
                tokio::spawn(async move {
                    let result =
                        crate::store_read::perform_store_read(&*store, module.as_str(), &keys)
                            .await;
                    let done = WorldCommand::StoreReadDone {
                        module,
                        actor,
                        correlation,
                        payload: result.to_payload(),
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            correlation,
                            "store read completion after shutdown dropped"
                        );
                    }
                });
            }
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
                } else if module.as_str() == mush_atlas::NS {
                    // S4 windows: the composite is a pure function of the
                    // snapshot the atlas took on-loop (doc 14 §3.4, P5); it
                    // needs no service, only a thread that is not the world's.
                    tokio::spawn(async move {
                        let job = payload;
                        let result = tokio::task::spawn_blocking(move || {
                            mush_atlas::run_async(&job)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            format!(
                                "{{\"domain\":0,\"revision\":0,\"result\":{{\"Err\":\"the compositor thread failed: {error}\"}}}}"
                            )
                        });
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
                            tracing::debug!(correlation, "atlas composite after shutdown dropped");
                        }
                    });
                } else if module.as_str() == tacsim::store::NS {
                    // **16b cut N.2** (doc 16 §6 "Deliberation"): one squad's
                    // standing order, asked for off-loop against the shared
                    // `LlmService` and re-entering as the serialized
                    // `DeliberationReply` — which `tacsim`'s registered completion
                    // turns into one `IntentKind::Order` per unit of the squad, or
                    // into nothing at all, which is the reflex doctrine.
                    //
                    // With no service on this world task (the repl, a test rig) the
                    // effect would be dropped and the bridge's in-flight count would
                    // never come back, so a missing service re-enters an
                    // `unavailable` refusal instead. That is the same degrade shape
                    // as `[llm].enabled = false`, which the real server always has a
                    // service for.
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "tacsim Effect::ModuleAsync: no LlmService on this world task; the \
                             squad stays on doctrine"
                        );
                        let refusal = tacsim::deliberation::unavailable_reply(&payload);
                        tokio::spawn(async move {
                            let done = WorldCommand::ModuleAsyncDone {
                                module,
                                actor,
                                correlation,
                                payload: refusal,
                            };
                            if tx
                                .send(WorldMsg::Inbound(Inbound::Apply(done)))
                                .await
                                .is_err()
                            {
                                tracing::debug!(
                                    correlation,
                                    "tacsim completion after shutdown dropped"
                                );
                            }
                        });
                        continue;
                    };
                    tokio::spawn(async move {
                        let result =
                            tacsim::deliberation::run_deliberation_async(&llm, &payload).await;
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
                                "tacsim completion after shutdown dropped"
                            );
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

    /// The store-less lens these projection assertions have always been taken
    /// with: no live authoritative store, said out loud.
    const DETACHED: TypedStateLens<'static> =
        TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

    /// H1: the store handle the effect-release helpers now carry (tests that
    /// never launch a store read hand them an empty in-memory store).
    fn test_effect_store() -> Arc<dyn ObjectStore> {
        Arc::new(InMemoryStore::new())
    }
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};

    use mush_core::{
        ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
        ActionPresentation, ActionProvider, ActionStateBootRecord, ArgShape, AsyncCompletion,
        AuthoritativeActionOutput, AuthoritativeBundle, AuthoritativeBundleDraft, AuthorityGuard,
        AuthorityGuardContext, AuthorityGuardKind, AuthorityGuardValidator, CanonicalHash,
        CanonicalId, ClockRef, ClockStamp, CmdError, CommandSpec,
        CommitToken as DurableCommitToken, ContextOutcome, Ctx, DecisionAdmissionIndex,
        DecisionCaptureFamilies, DecisionCapturePlan, DecisionCause, DecisionControllerId,
        DecisionId, DecisionKeyRing, DecisionPrompt, DecisionPromptRecord,
        DecisionPromptStateCodec, DecisionResponse, DecisionSecret, DecisionTerminalReceipt,
        DecisionTerminalReceiptStateCodec, DecisionToken, DecisionWorkerLease, Deferred,
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
        DefinitionRegistryBuilder, DurableScheduleStateCodec, EventHook, FallbackPolicyRef, GOD,
        GameModule, GuardSet, HookPoint, IdempotencyKey, IdempotencyKeyText, IdempotencyScope,
        IngressEnvelope, IngressPayload, IngressSurfaceVersion, InputContext, LIMBO,
        ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, OfferKeyRing, OfferSecret, OwnerId,
        PendingDecision, PendingDecisionStateCodec, PendingDecisionStatus, PendingOfferWitness,
        PerceivedRef, PreparedCausalRef, PreparedTxn, PrincipalId, ProjectedDecisionContext,
        ProjectedLegalAction, ProjectedParameterSchema, ProjectedTargetSchema, ProjectionScopeId,
        ProviderOfferDraft, QueryHandler, Registrar, RequestUid, RevisionKey, RevisionKind,
        RevisionSet, RevisionStamp, RevisionValidator, RulesValue, ScopedIdempotencyKey,
        StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey, TickHandler,
        WorldEventKind, WorldObserver, decision_prompt_state_family_descriptor,
        decision_terminal_receipt_state_family_descriptor,
        durable_schedule_state_family_descriptor, pending_decision_state_family_descriptor,
    };
    use mush_proto::{InputMode, PromptSpec};
    use mush_store::{AuthoritativeOutboxAttemptResult, AuthoritativeStore};
    use mush_tactics::{TacticalUnit, UnitSpec};

    use mush_contract_tests::tactical_gate::{
        TacticalGateState, disable_tactical_gate_weapon, exhaust_tactical_gate_ammo,
        install_tactical_gate_contact, install_tactical_gate_fire_window,
        install_tactical_gate_placement, install_tactical_gate_unit,
        install_tactical_gate_weapons_lease, move_tactical_gate_placement,
        publish_tactical_gate_weapons_station, tactical_gate_world_with_modules,
    };

    use crate::decision_dispatch::{
        DEFAULT_DECISION_PROVIDER_WORKERS, DecisionProviderAdapter, DecisionProviderFailure,
        DecisionProviderFailureKind,
    };

    struct BlackholeGateway;

    #[test]
    fn operated_known_subject_requires_exact_unambiguous_grant_epoch() {
        let mut engine = Engine::new(World::new_minimal(), Registry::new());
        let movement = mush_core::ChannelId::new("movement").unwrap();
        engine
            .world
            .publish_command_subject(mush_core::ChannelGrant {
                actor: GOD,
                subject: LIMBO,
                station: SmolStr::new_static("pilot"),
                relation: SmolStr::new_static("pilot"),
                owner: SmolStr::new_static("sim"),
                priority: 10,
                channels: BTreeSet::from([movement.clone()]),
                epoch: 7,
            });
        let principal = PrincipalId::Object { id: GOD };
        let projected = PerceivedRef::KnownObject {
            id: LIMBO,
            revision: 7,
        };
        let query = legal_action_query(&engine, principal.clone(), GOD, projected.clone()).unwrap();
        assert_eq!(query.truth_subject(), &SubjectKey::Object { id: LIMBO });
        assert_eq!(query.projected_subject(), &projected);

        assert_eq!(
            legal_action_query(
                &engine,
                principal.clone(),
                GOD,
                PerceivedRef::KnownObject {
                    id: LIMBO,
                    revision: 6,
                },
            )
            .unwrap_err(),
            LegalActionReadError::InvalidSubject
        );

        engine
            .world
            .publish_command_subject(mush_core::ChannelGrant {
                actor: GOD,
                subject: LIMBO,
                station: SmolStr::new_static("remote"),
                relation: SmolStr::new_static("remote"),
                owner: SmolStr::new_static("sim"),
                priority: 5,
                channels: BTreeSet::from([movement]),
                epoch: 8,
            });
        assert_eq!(
            legal_action_query(&engine, principal, GOD, projected).unwrap_err(),
            LegalActionReadError::InvalidSubject
        );
    }

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

    const DECISION_HARNESS_OWNER: &str = "decision_harness";
    const DECISION_WAKE_ADAPTER: &str = "decision_wake";

    type PendingDecisionHandle = StateFamilyHandle<DecisionId, PendingDecision>;
    type DecisionTerminalReceiptHandle = StateFamilyHandle<DecisionId, DecisionTerminalReceipt>;

    #[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct DecisionSelectionState {
        count: u64,
    }

    type DecisionSelectionHandle = StateFamilyHandle<CanonicalId, DecisionSelectionState>;

    #[derive(Clone)]
    struct DecisionHarnessFamilies {
        pending: PendingDecisionHandle,
        prompts: StateFamilyHandle<DecisionId, DecisionPromptRecord>,
        terminal_receipts: DecisionTerminalReceiptHandle,
        selections: DecisionSelectionHandle,
    }

    #[derive(Clone)]
    struct DecisionHarnessRegistration {
        families: DecisionHarnessFamilies,
        capture_action: DefinitionRef,
        lease_action: DefinitionRef,
        capability_generation: Arc<AtomicU64>,
    }

    struct DecisionHarnessModule {
        registration: Arc<Mutex<Option<DecisionHarnessRegistration>>>,
        capture_plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
        lease_duration: i64,
        capability_generation: Arc<AtomicU64>,
    }

    struct DecisionCaptureProvider {
        action: DefinitionRef,
        families: DecisionCaptureFamilies,
        plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
    }

    struct DecisionLeaseProvider {
        action: DefinitionRef,
        selections: DecisionSelectionHandle,
        capability_generation: Arc<AtomicU64>,
    }

    struct DecisionCapabilityValidator {
        generation: Arc<AtomicU64>,
    }

    impl AuthorityGuardValidator for DecisionCapabilityValidator {
        fn is_current(
            &self,
            _context: &AuthorityGuardContext<'_>,
            guard: &AuthorityGuard,
        ) -> Result<bool, ActionError> {
            Ok(matches!(
                guard,
                AuthorityGuard::Capability {
                    provider,
                    capability,
                    generation,
                    ..
                } if provider.as_str() == DECISION_HARNESS_OWNER
                    && capability.as_str() == "choose"
                    && *generation == self.generation.load(AtomicOrdering::SeqCst)
            ))
        }
    }

    struct DecisionHarnessFallback {
        identity: CanonicalId,
    }

    impl mush_core::DecisionFallbackAdapter for DecisionHarnessFallback {
        fn identity(&self) -> &CanonicalId {
            &self.identity
        }

        fn select(
            &self,
            _policy: &FallbackPolicyRef,
            prompt: &DecisionPrompt,
        ) -> Result<mush_core::DecisionFallbackSelection, String> {
            let action = prompt
                .actions
                .first()
                .ok_or_else(|| "fallback prompt has no projected actions".to_owned())?;
            Ok(mush_core::DecisionFallbackSelection {
                offer: action.offer(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
        }
    }

    fn harness_action_error(detail: impl Into<String>) -> ActionError {
        ActionError::new(ActionErrorCode::ProviderRejected, false, detail.into())
            .expect("bounded decision harness error")
    }

    impl ActionProvider for DecisionCaptureProvider {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }

        fn prepare_authoritative(
            &self,
            _invocation: &ActionInvocation,
            txn: &mut mush_core::ActionTxn<'_>,
        ) -> Result<AuthoritativeActionOutput, ActionError> {
            let plan = self
                .plan
                .lock()
                .expect("decision capture-plan mutex poisoned")
                .clone()
                .ok_or_else(|| harness_action_error("decision capture plan is not armed"))?;
            let wake = plan
                .stage(txn, &self.families)
                .map_err(|error| harness_action_error(error.to_string()))?;
            let outcome = RulesValue::new(
                self.action.clone(),
                serde_json::json!({
                    "captured": true
                }),
            )
            .map_err(|error| harness_action_error(error.to_string()))?;
            Ok(AuthoritativeActionOutput::new(outcome).with_durable_outbox(vec![wake]))
        }
    }

    impl ActionProvider for DecisionLeaseProvider {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }

        fn prepare_authoritative(
            &self,
            invocation: &ActionInvocation,
            txn: &mut mush_core::ActionTxn<'_>,
        ) -> Result<AuthoritativeActionOutput, ActionError> {
            if invocation.parameters().get("decision").is_none() {
                let key = CanonicalId::new("selected").expect("static selection key");
                match txn
                    .get(&self.selections, &key)
                    .map_err(|error| harness_action_error(error.to_string()))?
                    .cloned()
                {
                    Some(mut selection) => {
                        selection.count = selection.count.saturating_add(1);
                        txn.replace(&self.selections, &key, selection)
                            .map_err(|error| harness_action_error(error.to_string()))?;
                    }
                    None => txn
                        .insert(&self.selections, &key, DecisionSelectionState { count: 1 })
                        .map_err(|error| harness_action_error(error.to_string()))?,
                }
            }
            let outcome = RulesValue::new(
                self.action.clone(),
                serde_json::json!({
                    "leased": true
                }),
            )
            .map_err(|error| harness_action_error(error.to_string()))?;
            Ok(AuthoritativeActionOutput::new(outcome))
        }

        fn project_offer(
            &self,
            action: &DefinitionRef,
            query: &mush_core::LegalActionQuery,
            engine: &Engine,
        ) -> Result<Option<ProviderOfferDraft>, ActionError> {
            let definitions = engine
                .registry
                .definitions()
                .ok_or_else(|| harness_action_error("definition registry unavailable"))?;
            let mut guards = engine
                .registry
                .server_guard_set_builder(RevisionSet::default())
                .map_err(|error| harness_action_error(error.to_string()))?;
            guards
                .add_definition(action.clone(), definitions)
                .map_err(|error| harness_action_error(error.to_string()))?;
            guards
                .add(AuthorityGuard::Capability {
                    provider: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                    actor: query.actor(),
                    capability: CanonicalId::new("choose").expect("static capability"),
                    generation: self.capability_generation.load(AtomicOrdering::SeqCst),
                })
                .map_err(|error| harness_action_error(error.to_string()))?;
            Ok(Some(ProviderOfferDraft {
                fixed_parameters: serde_json::json!({}),
                parameters: ProjectedParameterSchema::empty(),
                targets: ProjectedTargetSchema::None,
                required_channels: Vec::new(),
                expires: None,
                presentation: ActionPresentation::new(
                    "choose",
                    "Choose",
                    "Select the decision action.",
                )
                .map_err(|error| harness_action_error(error.to_string()))?,
                guards: guards
                    .finish()
                    .map_err(|error| harness_action_error(error.to_string()))?,
            }))
        }
    }

    impl GameModule for DecisionHarnessModule {
        fn name(&self) -> &'static str {
            DECISION_HARNESS_OWNER
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            let action_kind = DefinitionKind::new("action")
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?;
            reg.definition_validator(
                action_kind.clone(),
                |context: &mush_core::DefinitionValidationContext<'_>| {
                    context
                        .definition()
                        .payload()
                        .is_object()
                        .then_some(())
                        .ok_or_else(|| {
                            "decision harness action payload must be an object".to_owned()
                        })
                },
            )?;
            let capture_action = reg.definition(
                DefinitionDescriptor::new(
                    action_kind.clone(),
                    DefinitionId::new("capture_decision")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    1,
                    serde_json::json!({}),
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?
                .as_current(),
            )?;
            let lease_action = reg.definition(
                DefinitionDescriptor::new(
                    action_kind,
                    DefinitionId::new("lease_decision")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    1,
                    serde_json::json!({}),
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?
                .as_current(),
            )?;
            let pending = reg.state_family(
                pending_decision_state_family_descriptor(
                    StateFamilyId::new("pending_decisions")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
            )?;
            let prompts = reg.state_family(
                decision_prompt_state_family_descriptor(
                    StateFamilyId::new("decision_prompts")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
            )?;
            let deadlines = reg.state_family(
                durable_schedule_state_family_descriptor(
                    StateFamilyId::new("decision_deadlines")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
            )?;
            let terminal_receipts = reg.state_family(
                decision_terminal_receipt_state_family_descriptor(
                    StateFamilyId::new("decision_terminal_receipts")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                DecisionTerminalReceiptStateCodec,
                DecisionTerminalReceiptStateCodec,
                DecisionTerminalReceiptStateCodec,
                DecisionTerminalReceiptStateCodec,
            )?;
            let selections = reg.state_family(
                StateFamilyDescriptor::new(
                    StateFamilyId::new("decision_selections")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    CanonicalId::new("canonical_id.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    CanonicalId::new("decision_selection.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    1,
                    CanonicalId::new("decision_selection_decode.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    CanonicalId::new("decision_selection_validate.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                    CanonicalId::new("decision_selection_serialize.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                )
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                |key: &CanonicalId| Ok(key.to_string()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                },
                |_value: &DecisionSelectionState| Ok(()),
                |value: &DecisionSelectionState| Ok(serde_json::json!({ "count": value.count })),
            )?;
            let capture = DecisionCaptureFamilies::new(
                pending.clone(),
                prompts.clone(),
                deadlines.clone(),
                terminal_receipts.clone(),
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?;
            reg.authority_guard_validator(
                AuthorityGuardKind::Capability,
                DecisionCapabilityValidator {
                    generation: Arc::clone(&self.capability_generation),
                },
            )?;
            reg.action_provider(
                capture_action.clone(),
                DecisionCaptureProvider {
                    action: capture_action.clone(),
                    families: capture.clone(),
                    plan: Arc::clone(&self.capture_plan),
                },
            )?;
            reg.action_provider(
                lease_action.clone(),
                DecisionLeaseProvider {
                    action: lease_action.clone(),
                    selections: selections.clone(),
                    capability_generation: Arc::clone(&self.capability_generation),
                },
            )?;
            reg.decision_source(
                lease_action.clone(),
                capture.clone(),
                CanonicalId::new(DECISION_WAKE_ADAPTER)
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                self.lease_duration,
                DecisionHarnessFallback {
                    identity: CanonicalId::new("decision_harness_fallback.v1")
                        .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                },
            )?;
            *self
                .registration
                .lock()
                .expect("decision registration mutex poisoned") =
                Some(DecisionHarnessRegistration {
                    families: DecisionHarnessFamilies {
                        pending,
                        prompts,
                        terminal_receipts,
                        selections,
                    },
                    capture_action,
                    lease_action,
                    capability_generation: Arc::clone(&self.capability_generation),
                });
            Ok(())
        }
    }

    struct RecordingDecisionProvider {
        owner: OwnerId,
        calls: Arc<AtomicUsize>,
        called: Arc<tokio::sync::Notify>,
    }

    impl DecisionProviderAdapter for RecordingDecisionProvider {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            lease: DecisionWorkerLease,
        ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
        {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.called.notify_waiters();
            Box::pin(async move {
                Ok(DecisionResponse {
                    token: lease.prompt().token,
                    offer: lease.prompt().actions[0].offer(),
                    targets: Vec::new(),
                    parameters: serde_json::json!({}),
                })
            })
        }
    }

    struct BlockingDecisionProvider {
        owner: OwnerId,
        calls: Arc<AtomicUsize>,
        called: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Semaphore>,
    }

    struct FailingDecisionProvider {
        owner: OwnerId,
        calls: Arc<AtomicUsize>,
        called: Arc<tokio::sync::Notify>,
        kind: DecisionProviderFailureKind,
    }

    #[derive(Clone, Copy, Debug)]
    enum ProjectedPolicyKind {
        Deterministic,
        DecisionTree,
        MockLlm,
    }

    struct ProjectedPolicyProvider {
        owner: OwnerId,
        kind: ProjectedPolicyKind,
    }

    impl DecisionProviderAdapter for ProjectedPolicyProvider {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            lease: DecisionWorkerLease,
        ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
        {
            let kind = self.kind;
            Box::pin(async move {
                let action = match kind {
                    ProjectedPolicyKind::Deterministic => lease.prompt().actions.first().cloned(),
                    ProjectedPolicyKind::DecisionTree => {
                        tokio::task::yield_now().await;
                        lease
                            .prompt()
                            .actions
                            .iter()
                            .find(|action| action.presentation().verb == "choose")
                            .cloned()
                    }
                    ProjectedPolicyKind::MockLlm => {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        let wire =
                            serde_json::to_vec(&lease.prompt().actions).map_err(|error| {
                                DecisionProviderFailure::new(
                                    DecisionProviderFailureKind::Malformed,
                                    error.to_string(),
                                )
                            })?;
                        serde_json::from_slice::<Vec<ProjectedLegalAction>>(&wire)
                            .map_err(|error| {
                                DecisionProviderFailure::new(
                                    DecisionProviderFailureKind::Malformed,
                                    error.to_string(),
                                )
                            })?
                            .into_iter()
                            .find(|action| action.presentation().verb == "choose")
                    }
                }
                .ok_or_else(|| {
                    DecisionProviderFailure::new(
                        DecisionProviderFailureKind::Permanent,
                        "projected policy found no admissible action",
                    )
                })?;
                Ok(DecisionResponse {
                    token: lease.prompt().token,
                    offer: action.offer(),
                    targets: Vec::new(),
                    parameters: serde_json::json!({}),
                })
            })
        }
    }

    impl DecisionProviderAdapter for FailingDecisionProvider {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            _lease: DecisionWorkerLease,
        ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
        {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.called.notify_waiters();
            let kind = self.kind;
            Box::pin(async move { Err(DecisionProviderFailure::new(kind, "injected failure")) })
        }
    }

    impl DecisionProviderAdapter for BlockingDecisionProvider {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            lease: DecisionWorkerLease,
        ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
        {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.called.notify_waiters();
            let release = Arc::clone(&self.release);
            Box::pin(async move {
                let permit = release.acquire_owned().await.map_err(|_| {
                    DecisionProviderFailure::new(
                        crate::decision_dispatch::DecisionProviderFailureKind::Unavailable,
                        "test release semaphore closed",
                    )
                })?;
                permit.forget();
                Ok(DecisionResponse {
                    token: lease.prompt().token,
                    offer: lease.prompt().actions[0].offer(),
                    targets: Vec::new(),
                    parameters: serde_json::json!({}),
                })
            })
        }
    }

    fn decision_harness(
        lease_duration: i64,
    ) -> (
        Engine,
        DecisionHarnessRegistration,
        Arc<Mutex<Option<DecisionCapturePlan>>>,
        DecisionKeyRing,
    ) {
        decision_harness_with(lease_duration, Vec::new())
    }

    /// The decision harness composed beside `extra` modules, so a test can put
    /// a second **mechanics owner** in the composition without giving it a
    /// decision source of its own.
    fn decision_harness_with(
        lease_duration: i64,
        extra: Vec<Box<dyn GameModule>>,
    ) -> (
        Engine,
        DecisionHarnessRegistration,
        Arc<Mutex<Option<DecisionCapturePlan>>>,
        DecisionKeyRing,
    ) {
        let registration = Arc::new(Mutex::new(None));
        let capture_plan = Arc::new(Mutex::new(None));
        let capability_generation = Arc::new(AtomicU64::new(1));
        let module = DecisionHarnessModule {
            registration: Arc::clone(&registration),
            capture_plan: Arc::clone(&capture_plan),
            lease_duration,
            capability_generation,
        };
        let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(module)];
        modules.extend(extra);
        let mut world = mush_testkit::TestWorld::with_modules(modules);
        let keys = DecisionKeyRing::new(
            DecisionSecret::new(1, [0x5a; 32]).expect("test decision secret"),
            [],
        )
        .expect("test decision key ring");
        world.engine_mut().set_decision_keys(keys.clone());
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(
                OfferSecret::new(1, [0x6b; 32]).expect("test offer secret"),
                [],
            )
            .expect("test offer key ring"),
        );
        let _ = world.drain_write_batch();
        let registered = registration
            .lock()
            .expect("decision registration mutex poisoned")
            .clone()
            .expect("decision harness registration");
        (world.into_engine(), registered, capture_plan, keys)
    }

    struct TacticalDecisionHarness {
        engine: Engine,
        registration: DecisionHarnessRegistration,
        tactical_state: TacticalGateState,
        unit_spec: UnitSpec,
        capture_plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
        keys: DecisionKeyRing,
    }

    fn tactical_decision_harness(lease_duration: i64) -> TacticalDecisionHarness {
        let registration = Arc::new(Mutex::new(None));
        let capture_plan = Arc::new(Mutex::new(None));
        let capability_generation = Arc::new(AtomicU64::new(1));
        let module = DecisionHarnessModule {
            registration: Arc::clone(&registration),
            capture_plan: Arc::clone(&capture_plan),
            lease_duration,
            capability_generation,
        };
        let (mut engine, tactical_state, _tactical_definitions, unit_spec) =
            tactical_gate_world_with_modules(vec![Box::new(module)]);
        let keys = DecisionKeyRing::new(
            DecisionSecret::new(1, [0x5a; 32]).expect("test decision secret"),
            [],
        )
        .expect("test decision key ring");
        engine.set_decision_keys(keys.clone());
        let registered = registration
            .lock()
            .expect("decision registration mutex poisoned")
            .clone()
            .expect("decision harness registration");
        TacticalDecisionHarness {
            engine,
            registration: registered,
            tactical_state,
            unit_spec,
            capture_plan,
            keys,
        }
    }

    fn projected_decision_action(engine: &Engine) -> ProjectedLegalAction {
        let query = engine
            .legal_action_query(
                PrincipalId::Object { id: GOD },
                GOD,
                PerceivedRef::SelfActor,
            )
            .expect("decision action query");
        engine
            .project_legal_actions(&query)
            .expect("decision action projection")
            .projected()
            .into_iter()
            .find(|action| action.presentation().verb == "choose")
            .expect("decision harness choice action")
    }

    fn arm_decision_capture(
        engine: &Engine,
        keys: &DecisionKeyRing,
        registration: &DecisionHarnessRegistration,
        capture_plan: &Mutex<Option<DecisionCapturePlan>>,
        salt: u8,
        deadline: i64,
    ) -> PendingDecision {
        let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
        let principal = PrincipalId::Object { id: GOD };
        let controller = DecisionControllerId {
            owner: owner.clone(),
            key: CanonicalId::new("controller_1").expect("static controller id"),
        };
        let cause = DecisionCause {
            kind: CanonicalId::new("test_turn").expect("static cause kind"),
            instance: IdempotencyKeyText::parse_canonical(format!("turn-{salt}"))
                .expect("test cause instance"),
            replaceable_key: None,
        };
        let action_idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("chosen_action").expect("static namespace"),
            },
            1,
            IdempotencyKey::Deterministic(CanonicalHash::from_bytes([salt; 32])),
        );
        let id = DecisionId::derive(&controller, &cause, &action_idempotency)
            .expect("decision identity");
        let projected = projected_decision_action(engine);
        let offer = projected.offer();
        let placeholder = DecisionToken {
            id,
            revision: 1,
            prompt_digest: CanonicalHash::from_bytes([1; 32]),
            key_version: 1,
            tag: CanonicalHash::from_bytes([1; 32]),
        };
        let mut prompt = DecisionPrompt {
            token: placeholder,
            actions: vec![projected],
            context: ProjectedDecisionContext::new(
                CanonicalId::new("test_context.v1").expect("static context id"),
                serde_json::json!({ "turn": salt }),
            )
            .expect("decision context"),
        };
        let prompt_digest = prompt.digest().expect("decision prompt digest");
        prompt.token = keys.sign(id, 1, prompt_digest);
        let prompt = DecisionPromptRecord::new(prompt).expect("decision prompt record");
        let pending = PendingDecision {
            id,
            revision: 1,
            controller,
            principal: principal.clone(),
            actor: GOD,
            truth_subject: SubjectKey::Object { id: GOD },
            viewer_scope: ProjectionScopeId::derive(&principal, 1)
                .expect("decision projection scope"),
            cause,
            offered: vec![PendingOfferWitness {
                offer,
                common_revisions: Vec::new(),
                target_revisions: Vec::new(),
                bridge_definitions: Vec::new(),
            }],
            prompt_digest,
            token_key_version: 1,
            deadline: ClockStamp {
                clock: ClockRef::Utc,
                at: deadline,
            },
            fallback: FallbackPolicyRef(registration.lease_action.clone()),
            action_idempotency,
            malformed_attempts: 0,
            attempt_audit: Vec::new(),
            status: PendingDecisionStatus::Ready { attempt: 1 },
        };
        let admission = DecisionAdmissionIndex::default()
            .admit(&pending, &prompt, keys)
            .expect("decision admission");
        let plan = DecisionCapturePlan::new(
            &owner,
            pending.clone(),
            prompt,
            admission,
            keys,
            CanonicalId::new(DECISION_WAKE_ADAPTER).expect("static wake adapter"),
            0,
        )
        .expect("decision capture plan");
        *capture_plan
            .lock()
            .expect("decision capture-plan mutex poisoned") = Some(plan);
        pending
    }

    #[allow(clippy::too_many_arguments)]
    fn arm_tactical_decision_capture(
        keys: &DecisionKeyRing,
        registration: &DecisionHarnessRegistration,
        capture_plan: &Mutex<Option<DecisionCapturePlan>>,
        actor: mush_core::ObjId,
        projected: ProjectedLegalAction,
        witness: PendingOfferWitness,
        salt: u8,
        deadline: i64,
    ) -> PendingDecision {
        let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
        let principal = PrincipalId::Object { id: actor };
        let controller = DecisionControllerId {
            owner: owner.clone(),
            key: CanonicalId::new("tactical_controller").expect("static controller id"),
        };
        let cause = DecisionCause {
            kind: CanonicalId::new("tactical_turn").expect("static cause kind"),
            instance: IdempotencyKeyText::parse_canonical(format!("tactical-turn-{salt}"))
                .expect("test cause instance"),
            replaceable_key: None,
        };
        let action_idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: OwnerId::new(mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER)
                    .expect("static tactics owner"),
                namespace: CanonicalId::new(mush_contract_tests::tactical_gate::FIRE_ACTION_ID)
                    .expect("static fire namespace"),
            },
            1,
            IdempotencyKey::Deterministic(CanonicalHash::from_bytes([salt; 32])),
        );
        let id = DecisionId::derive(&controller, &cause, &action_idempotency)
            .expect("decision identity");
        let placeholder = DecisionToken {
            id,
            revision: 1,
            prompt_digest: CanonicalHash::from_bytes([1; 32]),
            key_version: 1,
            tag: CanonicalHash::from_bytes([1; 32]),
        };
        let mut prompt = DecisionPrompt {
            token: placeholder,
            actions: vec![projected],
            context: ProjectedDecisionContext::new(
                CanonicalId::new("tactical_context.v1").expect("static context id"),
                serde_json::json!({ "turn": salt }),
            )
            .expect("decision context"),
        };
        let prompt_digest = prompt.digest().expect("decision prompt digest");
        prompt.token = keys.sign(id, 1, prompt_digest);
        let prompt = DecisionPromptRecord::new(prompt).expect("decision prompt record");
        let pending = PendingDecision {
            id,
            revision: 1,
            controller,
            principal: principal.clone(),
            actor,
            truth_subject: SubjectKey::Object { id: actor },
            viewer_scope: ProjectionScopeId::derive(&principal, 1)
                .expect("decision projection scope"),
            cause,
            offered: vec![witness],
            prompt_digest,
            token_key_version: 1,
            deadline: ClockStamp {
                clock: ClockRef::Utc,
                at: deadline,
            },
            fallback: FallbackPolicyRef(registration.lease_action.clone()),
            action_idempotency,
            malformed_attempts: 0,
            attempt_audit: Vec::new(),
            status: PendingDecisionStatus::Ready { attempt: 1 },
        };
        let admission = DecisionAdmissionIndex::default()
            .admit(&pending, &prompt, keys)
            .expect("decision admission");
        let plan = DecisionCapturePlan::new(
            &owner,
            pending.clone(),
            prompt,
            admission,
            keys,
            CanonicalId::new(DECISION_WAKE_ADAPTER).expect("static wake adapter"),
            0,
        )
        .expect("decision capture plan");
        *capture_plan
            .lock()
            .expect("decision capture-plan mutex poisoned") = Some(plan);
        pending
    }

    fn lease_captured_decision(
        engine: &mut Engine,
        registration: &DecisionHarnessRegistration,
        pending: &PendingDecision,
        keys: &DecisionKeyRing,
    ) -> (
        mush_core::DecisionCompletionFence,
        DecisionResponse,
        DecisionWorkerLease,
    ) {
        let prompt = engine
            .action_state()
            .get(&registration.families.prompts, &pending.id)
            .expect("decision prompt read")
            .expect("decision prompt")
            .clone();
        let current = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("pending decision read")
            .expect("pending decision")
            .clone();
        let now = ClockStamp {
            clock: ClockRef::Utc,
            at: pending.deadline.at - 30,
        };
        let wake = mush_core::DecisionWake {
            id: pending.id,
            revision: pending.revision,
            prompt_digest: pending.prompt_digest,
        };
        let transition = current
            .plan_worker_lease(&prompt, &wake, keys, &now, 20)
            .expect("decision lease plan");
        let (leased, worker) = transition.into_parts();
        let mut transaction = engine
            .action_transaction(&registration.families.pending)
            .expect("decision lease transaction");
        transaction
            .replace(&registration.families.pending, &pending.id, leased.clone())
            .expect("decision lease post-image");
        let (_, installation) = transaction
            .prepare()
            .expect("decision lease prepare")
            .into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        let fence = mush_core::DecisionCompletionFence::new(
            worker.controller().clone(),
            worker.decision(),
            worker.revision(),
            worker.attempt(),
            worker.lease_until().clone(),
        )
        .expect("decision completion fence");
        let response = DecisionResponse {
            token: prompt.prompt.token,
            offer: prompt.prompt.actions[0].offer(),
            targets: match prompt.prompt.actions[0].targets() {
                ProjectedTargetSchema::Choices { choices, .. } => vec![choices[0].clone()],
                ProjectedTargetSchema::None => Vec::new(),
            },
            parameters: serde_json::json!({}),
        };
        (fence, response, worker)
    }

    async fn commit_armed_capture(
        engine: &mut Engine,
        store: &dyn AuthoritativeStore,
        registration: &DecisionHarnessRegistration,
        salt: u8,
    ) {
        let request = RequestUid::from_bytes([salt; 16]);
        let principal = PrincipalId::Object { id: GOD };
        let idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                namespace: CanonicalId::new("capture_decision").expect("static namespace"),
            },
            1,
            IdempotencyKey::Request(request),
        );
        let ingress = IngressEnvelope::new(
            request,
            principal.clone(),
            IngressSurfaceVersion::new(
                CanonicalId::new("test.decision_capture").expect("static ingress surface"),
                1,
            )
            .expect("capture ingress version"),
            IngressPayload::structured(serde_json::json!({ "salt": salt }))
                .expect("capture ingress payload"),
            idempotency.clone(),
        )
        .expect("capture ingress");
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal,
            actor: GOD,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([salt; 32])),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: serde_json::json!({}),
            idempotency,
        })
        .expect("capture invocation");
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(store, ingress)
            .await
            .expect("capture preflight")
        else {
            panic!("capture idempotency was not vacant")
        };
        let guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .expect("capture guard builder")
            .finish()
            .expect("capture guards");
        ActionCoordinator::prepare_and_stage(
            permit,
            engine,
            &registration.capture_action,
            &invocation,
            guards,
        )
        .expect("capture action staging");
        let staged = engine
            .take_staged_action()
            .expect("capture admission")
            .expect("capture staged action");
        let (bundle, installation) = staged.into_parts();
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("capture intent reservation");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("capture reconciliation");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
    }

    async fn wait_for_persisted_decision_terminal(
        store: &dyn ObjectStore,
        registration: &DecisionHarnessRegistration,
        decision: DecisionId,
    ) -> DecisionTerminalReceipt {
        let pending_key = format!(
            "action-state/v1/{}/{}",
            registration.families.pending.descriptor().id(),
            decision.canonical_hash().to_hex()
        );
        let receipt_key = format!(
            "action-state/v1/{}/{}",
            registration.families.terminal_receipts.descriptor().id(),
            decision.canonical_hash().to_hex()
        );
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(value) = store
                    .module_json_get(DECISION_HARNESS_OWNER, &receipt_key)
                    .await
                    .expect("decision compact receipt read")
                {
                    break serde_json::from_value(value["value"].clone())
                        .expect("valid compact decision receipt");
                }
                if let Some(value) = store
                    .module_json_get(DECISION_HARNESS_OWNER, &pending_key)
                    .await
                    .expect("decision state read")
                    && value["value"]["status"]["state"] == "terminal"
                {
                    let pending: PendingDecision = serde_json::from_value(value["value"].clone())
                        .expect("valid terminal pending decision");
                    break DecisionTerminalReceipt::from_terminal(&pending)
                        .expect("valid terminal attribution");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("decision did not terminalize through deterministic fallback")
    }

    async fn discard_decision_wake(store: &dyn AuthoritativeStore) {
        let pending = store
            .pending_authoritative_outbox()
            .await
            .expect("pending decision wake");
        assert_eq!(pending.len(), 1, "capture must emit one durable wake");
        let wake = &pending[0];
        let started = store
            .begin_authoritative_outbox_attempt(
                wake.item_id(),
                wake.intent().payload_hash(),
                wake.attempts(),
            )
            .await
            .expect("begin discarded wake");
        store
            .complete_authoritative_outbox_attempt(
                started.item_id(),
                started.intent().payload_hash(),
                started.attempts(),
                AuthoritativeOutboxAttemptResult::Delivered,
            )
            .await
            .expect("discard decision wake");
    }

    fn decision_test_deps(store: Arc<dyn ObjectStore>) -> ServerDeps {
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        }
    }

    async fn stop_test_server(handles: PersistHandles) {
        assert!(
            handles
                .handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        handles.world_join.await.expect("world task shutdown");
        handles.writer_join.await.expect("writer task shutdown");
        handles.outbox_join.await.expect("outbox task shutdown");
    }

    #[tokio::test]
    async fn decision_dispatch_rediscovery_leases_after_capture_ack_and_lost_wake() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x31,
            unix_now_s() + 60,
        );
        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        let authoritative = store
            .authoritative_store()
            .expect("in-memory store is authoritative");
        commit_armed_capture(&mut engine, authoritative, &registration, 0x41).await;

        // Simulate the volatile wake disappearing after its durable outbox
        // delivery was acknowledged. The typed pair must remain sufficient
        // for the periodic canonical scan to recover the obligation.
        discard_decision_wake(authoritative).await;
        assert!(
            authoritative
                .pending_authoritative_outbox()
                .await
                .expect("post-discard outbox scan")
                .is_empty()
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let called = Arc::new(tokio::sync::Notify::new());
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&called),
            }))
            .expect("decision provider registration");
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            deps,
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if calls.load(AtomicOrdering::SeqCst) == 1 {
                    break;
                }
                called.notified().await;
            }
        })
        .await
        .expect("periodic decision rediscovery did not dispatch");

        let persistence_key = format!(
            "action-state/v1/{}/{}",
            registration.families.pending.descriptor().id(),
            pending.id.canonical_hash().to_hex()
        );
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);

        let selection_key = format!(
            "action-state/v1/{}/selected",
            registration.families.selections.descriptor().id()
        );
        let selected = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(value) = store
                    .module_json_get(DECISION_HARNESS_OWNER, &selection_key)
                    .await
                    .expect("selected action read")
                {
                    break value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("authenticated decision response did not commit its selected action");
        assert_eq!(selected["value"]["count"], 1);

        let terminal = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let value = store
                    .module_json_get(DECISION_HARNESS_OWNER, &persistence_key)
                    .await
                    .expect("terminal decision read")
                    .expect("terminal decision record");
                if value["value"]["status"]["state"] == "terminal" {
                    break value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("selected action did not advance through resolving to terminal");
        assert_eq!(terminal["value"]["status"]["outcome"], "executed");
        assert!(terminal["value"]["status"]["action_binding"]["request"].is_string());
        assert!(terminal["value"]["status"]["action_binding"]["request_hash"].is_string());

        stop_test_server(handles).await;
    }

    #[tokio::test]
    async fn expired_decision_lease_is_rediscovered_after_process_restart() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(1);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x33,
            unix_now_s() + 30,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        commit_armed_capture(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            &registration,
            0x43,
        )
        .await;

        let calls = Arc::new(AtomicUsize::new(0));
        let first_called = Arc::new(tokio::sync::Notify::new());
        let first_release = Arc::new(tokio::sync::Semaphore::new(0));
        let mut first_providers = DecisionProviderRegistry::new();
        first_providers
            .register(Arc::new(BlockingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&first_called),
                release: Arc::clone(&first_release),
            }))
            .expect("first decision provider registration");
        let first = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            first_providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("first decision runtime startup");
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(AtomicOrdering::SeqCst) < 1 {
                first_called.notified().await;
            }
        })
        .await
        .expect("first decision lease did not dispatch");
        stop_test_server(first).await;
        first_release.add_permits(1);

        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let (mut restarted, restarted_registration, _plan, _keys) = decision_harness(1);
        let boot_records = store
            .module_json_list(DECISION_HARNESS_OWNER)
            .await
            .expect("decision state boot scan")
            .into_iter()
            .filter(|(key, _)| mush_core::is_action_state_persistence_key(key))
            .map(|(key, value)| ActionStateBootRecord::new(DECISION_HARNESS_OWNER, key, value))
            .collect::<Vec<_>>();
        restarted
            .hydrate_action_state(boot_records)
            .expect("decision state hydration after restart");

        let second_called = Arc::new(tokio::sync::Notify::new());
        let second_release = Arc::new(tokio::sync::Semaphore::new(0));
        let mut second_providers = DecisionProviderRegistry::new();
        second_providers
            .register(Arc::new(BlockingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&second_called),
                release: Arc::clone(&second_release),
            }))
            .expect("second decision provider registration");
        let second = spawn_persist_with_gateway_and_decisions(
            restarted,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            second_providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("restarted decision runtime startup");
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(AtomicOrdering::SeqCst) < 2 {
                second_called.notified().await;
            }
        })
        .await
        .expect("expired lease was not rediscovered after restart");

        let persistence_key = format!(
            "action-state/v1/{}/{}",
            restarted_registration.families.pending.descriptor().id(),
            pending.id.canonical_hash().to_hex()
        );
        let persisted = store
            .module_json_get(DECISION_HARNESS_OWNER, &persistence_key)
            .await
            .expect("retried lease read")
            .expect("retried lease post-image");
        assert_eq!(persisted["value"]["status"]["state"], "leased");
        assert_eq!(persisted["value"]["status"]["attempt"], 2);
        stop_test_server(second).await;
        second_release.add_permits(1);
    }

    #[tokio::test]
    async fn worker_saturation_leaves_second_decision_ready_until_capacity_returns() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let first_pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x34,
            unix_now_s() + 60,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let authoritative = store
            .authoritative_store()
            .expect("in-memory store is authoritative");
        commit_armed_capture(&mut engine, authoritative, &registration, 0x44).await;
        let second_pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x35,
            unix_now_s() + 60,
        );
        commit_armed_capture(&mut engine, authoritative, &registration, 0x45).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let called = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(BlockingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&called),
                release: Arc::clone(&release),
            }))
            .expect("blocking provider registration");
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("bounded decision runtime startup");
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(AtomicOrdering::SeqCst) < 1 {
                called.notified().await;
            }
        })
        .await
        .expect("first provider did not start");

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            1,
            "worker saturation must not commit another lease without capacity"
        );
        let first_key = format!(
            "action-state/v1/{}/{}",
            registration.families.pending.descriptor().id(),
            first_pending.id.canonical_hash().to_hex()
        );
        let second_key = format!(
            "action-state/v1/{}/{}",
            registration.families.pending.descriptor().id(),
            second_pending.id.canonical_hash().to_hex()
        );
        let states = [first_key.as_str(), second_key.as_str()]
            .into_iter()
            .map(|key| async {
                store
                    .module_json_get(DECISION_HARNESS_OWNER, key)
                    .await
                    .expect("saturated decision read")
                    .expect("saturated decision state")["value"]["status"]["state"]
                    .as_str()
                    .expect("decision state name")
                    .to_owned()
            });
        let [first_state, second_state] = futures::future::join_all(states)
            .await
            .try_into()
            .expect("two decision states");
        assert_eq!(
            [first_state.as_str(), second_state.as_str()]
                .into_iter()
                .filter(|state| *state == "leased")
                .count(),
            1
        );
        assert_eq!(
            [first_state.as_str(), second_state.as_str()]
                .into_iter()
                .filter(|state| *state == "ready")
                .count(),
            1
        );

        release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(AtomicOrdering::SeqCst) < 2 {
                called.notified().await;
            }
        })
        .await
        .expect("second provider did not start after capacity returned");
        release.add_permits(1);
        stop_test_server(handles).await;
    }

    #[tokio::test]
    async fn refused_decision_lease_aborts_post_image_without_spawning_provider() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x32,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x42).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::new(tokio::sync::Notify::new()),
            }))
            .expect("decision provider registration");
        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let DurableDecisionDrive::Staged(staged_dispatch) = driver
            .drive_one(&mut engine, &store, None)
            .await
            .expect("decision lease drive")
        else {
            panic!("ready decision did not stage a lease")
        };
        let StagedDecisionDispatch {
            staged,
            reservation,
        } = *staged_dispatch;
        let StagedDecisionLease { staged, worker } = staged;
        let prepared = engine
            .take_staged_action()
            .expect("lease admission")
            .expect("staged lease action");
        let (_bundle, installation) = prepared.into_parts();
        let (_receipt_tx, receipt_rx) = oneshot::channel();
        let pending_commit = PendingCommit {
            token: CommitToken::Legacy(1),
            receipt: PendingReceipt::Legacy {
                expected_serial: 1,
                receiver: receipt_rx,
            },
            sends: Vec::new(),
            effects: Vec::new(),
            failure_actor: None,
            create: None,
            action: Some(ActionCompletion {
                disposition: None,
                staged,
                owned: None,
                finished: None,
                ownership_valid: false,
                post_commit_decision: Some(DecisionPostCommit {
                    reservation,
                    worker,
                }),
                post_commit_decision_response: None,
                post_commit_decision_terminal: None,
            }),
            prepared_installation: Some(installation),
            barriers: Vec::new(),
            coalesced_reattach: None,
            exit_after_ack: false,
        };
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let mut health = StoreHealth::new();
        let integrity = PersistenceIntegrityLatch::new();
        let (world_tx, _world_rx) = mpsc::channel(1);
        let _ = complete_pending_commit(
            pending_commit,
            Err(AuthoritativeCommitFailure::Refused(
                "injected lease refusal".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &integrity,
            &world_tx.downgrade(),
            None,
            &test_effect_store(),
        );

        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
        assert!(matches!(
            engine
                .action_state()
                .get(&registration.families.pending, &pending.id)
                .expect("pending decision read")
                .expect("pending decision remains")
                .status,
            PendingDecisionStatus::Ready { attempt: 1 }
        ));
        assert!(!engine.action_state().has_prepared());
    }

    #[tokio::test]
    async fn malformed_provider_attempts_commit_then_execute_deterministic_fallback() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x37,
            unix_now_s() + 60,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        commit_armed_capture(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            &registration,
            0x47,
        )
        .await;

        let calls = Arc::new(AtomicUsize::new(0));
        let called = Arc::new(tokio::sync::Notify::new());
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(FailingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&called),
                kind: DecisionProviderFailureKind::Malformed,
            }))
            .expect("failing provider registration");
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        tokio::time::timeout(Duration::from_secs(4), async {
            while calls.load(AtomicOrdering::SeqCst)
                < usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS)
            {
                called.notified().await;
            }
        })
        .await
        .expect("bounded malformed attempts did not run");

        let fallback =
            wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
        assert_eq!(
            fallback.malformed_attempts,
            mush_core::DEFAULT_DECISION_ATTEMPTS
        );
        assert_eq!(
            fallback.outcome,
            mush_core::DecisionTerminalOutcome::Fallback
        );
        assert_eq!(
            fallback.fallback_reason,
            Some(mush_core::DecisionFallbackReason::MalformedExhausted)
        );
        assert_eq!(
            fallback
                .attempt_audit
                .iter()
                .map(|attempt| (attempt.attempt, attempt.failure))
                .collect::<Vec<_>>(),
            vec![
                (1, mush_core::DecisionAttemptFailureKind::Malformed),
                (2, mush_core::DecisionAttemptFailureKind::Malformed),
                (3, mush_core::DecisionAttemptFailureKind::Malformed),
            ]
        );
        let selection_key = format!(
            "action-state/v1/{}/selected",
            registration.families.selections.descriptor().id()
        );
        let selected = store
            .module_json_get(DECISION_HARNESS_OWNER, &selection_key)
            .await
            .expect("fallback selection read")
            .expect("fallback selected action state");
        assert_eq!(selected["value"]["count"], 1);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS),
            "fallback terminalization must stop provider redispatch"
        );
        stop_test_server(handles).await;
    }

    #[tokio::test]
    async fn provider_outage_exhausts_bounded_attempts_then_executes_fallback() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x3b,
            unix_now_s() + 60,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        commit_armed_capture(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            &registration,
            0x4b,
        )
        .await;

        let calls = Arc::new(AtomicUsize::new(0));
        let called = Arc::new(tokio::sync::Notify::new());
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(FailingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::clone(&called),
                kind: DecisionProviderFailureKind::Unavailable,
            }))
            .expect("failing provider registration");
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        tokio::time::timeout(Duration::from_secs(4), async {
            while calls.load(AtomicOrdering::SeqCst)
                < usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS)
            {
                called.notified().await;
            }
        })
        .await
        .expect("bounded unavailable-provider attempts did not run");
        let terminal =
            wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
        assert_eq!(terminal.malformed_attempts, 0);
        assert_eq!(
            terminal.outcome,
            mush_core::DecisionTerminalOutcome::Fallback
        );
        assert_eq!(
            terminal.fallback_reason,
            Some(mush_core::DecisionFallbackReason::ProviderExhausted)
        );
        assert_eq!(
            terminal
                .attempt_audit
                .iter()
                .map(|attempt| (attempt.attempt, attempt.failure))
                .collect::<Vec<_>>(),
            vec![
                (1, mush_core::DecisionAttemptFailureKind::Unavailable),
                (2, mush_core::DecisionAttemptFailureKind::Unavailable),
                (3, mush_core::DecisionAttemptFailureKind::Unavailable),
            ]
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS),
            "provider outage must stop at the bounded attempt policy"
        );
        stop_test_server(handles).await;
    }

    #[tokio::test]
    async fn unavailable_provider_stages_exact_fallback_custody() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x38,
            unix_now_s() + 60,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        commit_armed_capture(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            &registration,
            0x48,
        )
        .await;

        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(DecisionProviderRegistry::new()), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let DurableDecisionDrive::StagedLifecycle(staged) = driver
            .drive_one(
                &mut engine,
                store
                    .authoritative_store()
                    .expect("in-memory store is authoritative"),
                None,
            )
            .await
            .expect("provider-unavailable fallback drive")
        else {
            panic!("provider unavailability did not stage fallback custody")
        };
        let prepared = engine
            .take_staged_action()
            .expect("fallback custody admission")
            .expect("staged fallback custody action");
        let (bundle, installation) = prepared.into_parts();
        assert_eq!(bundle.token(), staged.token);
        assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
        let authoritative = store
            .authoritative_store()
            .expect("in-memory store is authoritative");
        authoritative
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("fallback custody intent");
        authoritative
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("fallback custody reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        assert!(matches!(
            engine
                .action_state()
                .get(&registration.families.pending, &pending.id)
                .expect("fallback custody read")
                .expect("pending decision")
                .status,
            PendingDecisionStatus::FallbackReady {
                reason: mush_core::DecisionFallbackReason::ProviderUnavailable,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn recovery_audits_all_durable_limits_before_first_dispatch() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let store = InMemoryStore::new();
        for offset in 0_u8..=u8::try_from(mush_core::MAX_NONTERMINAL_DECISIONS_PER_CONTROLLER)
            .expect("controller cap fits u8")
        {
            arm_decision_capture(
                &engine,
                &keys,
                &registration,
                &capture_plan,
                0x50 + offset,
                unix_now_s() + 60,
            );
            commit_armed_capture(&mut engine, &store, &registration, 0x60 + offset).await;
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::new(tokio::sync::Notify::new()),
            }))
            .expect("decision provider registration");
        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let error = match driver.drive_one(&mut engine, &store, None).await {
            Err(error) => error,
            Ok(_) => panic!("over-cap durable decisions must freeze recovery"),
        };
        assert!(error.contains("limit reached"), "{error}");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
        assert!(!engine.action_state().has_prepared());
    }

    #[tokio::test]
    async fn expired_deadline_wins_before_available_provider_is_called() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x39,
            unix_now_s() - 1,
        );
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        commit_armed_capture(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            &registration,
            0x49,
        )
        .await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls: Arc::clone(&calls),
                called: Arc::new(tokio::sync::Notify::new()),
            }))
            .expect("decision provider registration");
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        let terminal =
            wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
        assert_eq!(
            terminal.outcome,
            mush_core::DecisionTerminalOutcome::Fallback
        );
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
        stop_test_server(handles).await;
    }

    #[tokio::test]
    async fn cancellation_ack_installs_terminal_state_before_token_rejection() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x3a,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x4a).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls,
                called: Arc::new(tokio::sync::Notify::new()),
            }))
            .expect("decision provider registration");
        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let DurableDecisionDrive::Staged(staged_dispatch) = driver
            .drive_one(&mut engine, &store, None)
            .await
            .expect("decision lease drive")
        else {
            panic!("ready decision did not stage a lease")
        };
        let StagedDecisionDispatch {
            staged,
            reservation,
        } = *staged_dispatch;
        let StagedDecisionLease { staged, worker } = staged;
        let prepared = engine
            .take_staged_action()
            .expect("lease admission")
            .expect("staged lease action");
        let (bundle, installation) = prepared.into_parts();
        assert_eq!(bundle.token(), staged.token);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("lease intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("lease reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        drop(reservation);

        let fence = mush_core::DecisionCompletionFence::new(
            worker.controller().clone(),
            worker.decision(),
            worker.revision(),
            worker.attempt(),
            worker.lease_until().clone(),
        )
        .expect("worker fence");
        let response = DecisionResponse {
            token: worker.prompt().token,
            offer: worker.prompt().actions[0].offer(),
            targets: Vec::new(),
            parameters: serde_json::json!({}),
        };
        engine
            .decision_response_ingress(&fence, &response)
            .expect("live lease accepts its exact response");

        let ingress = ActionCoordinator::decision_cancellation_ingress(
            &engine,
            &pending.controller,
            pending.id,
            pending.revision,
            1,
        )
        .expect("cancellation ingress");
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
            .await
            .expect("cancellation preflight")
        else {
            panic!("cancellation idempotency was not vacant")
        };
        let staged = ActionCoordinator::prepare_and_stage_decision_cancellation(
            permit,
            &mut engine,
            &pending.controller,
            pending.id,
            pending.revision,
        )
        .expect("cancellation staging");
        engine
            .decision_response_ingress(&fence, &response)
            .expect("unacknowledged cancellation has not changed live authority");
        let prepared = engine
            .take_staged_action()
            .expect("cancellation admission")
            .expect("staged cancellation action");
        let (bundle, installation) = prepared.into_parts();
        assert_eq!(bundle.token(), staged.token);
        assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("cancellation intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("cancellation reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        assert!(engine.decision_response_ingress(&fence, &response).is_err());
        assert!(matches!(
            engine
                .action_state()
                .get(&registration.families.pending, &pending.id)
                .expect("cancelled decision read")
                .expect("cancelled decision")
                .status,
            PendingDecisionStatus::Terminal {
                outcome: mush_core::DecisionTerminalOutcome::Cancelled,
                action_binding: None,
                ..
            }
        ));

        let compacted = loop {
            match driver
                .drive_one(&mut engine, &store, None)
                .await
                .expect("cancelled decision compaction drive")
            {
                DurableDecisionDrive::StagedLifecycle(staged) => break staged,
                DurableDecisionDrive::Idle | DurableDecisionDrive::RecoveryAuditProgress => {}
                _ => panic!("cancelled decision did not stage no-action compaction"),
            }
        };
        let prepared = engine
            .take_staged_action()
            .expect("compaction admission")
            .expect("staged compaction action");
        let (bundle, installation) = prepared.into_parts();
        assert_eq!(bundle.token(), compacted.token);
        assert_eq!(bundle.bundle_hash(), compacted.bundle_hash);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("compaction intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("compaction reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        assert!(
            engine
                .action_state()
                .get(&registration.families.pending, &pending.id)
                .expect("compacted pending read")
                .is_none()
        );
        let receipt = engine
            .action_state()
            .get(&registration.families.terminal_receipts, &pending.id)
            .expect("compact receipt read")
            .expect("compact receipt");
        assert_eq!(
            receipt.outcome,
            mush_core::DecisionTerminalOutcome::Cancelled
        );
        assert!(receipt.action_binding.is_none());
    }

    #[tokio::test]
    async fn selected_action_terminal_cannot_compact_without_exact_final_receipt() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x3c,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x4c).await;

        let binding = mush_core::DecisionActionBinding {
            request: RequestUid::from_bytes([0x5c; 16]),
            request_hash: CanonicalHash::from_bytes([0x6c; 32]),
        };
        let mut terminal = pending.clone();
        terminal
            .finish(
                terminal.revision,
                mush_core::DecisionTerminalOutcome::Executed,
                Some(binding),
            )
            .expect("synthetic selected terminal");
        let pending_handle = registration.families.pending.clone();
        let mut transaction = engine
            .action_transaction(&pending_handle)
            .expect("terminal test transaction");
        transaction
            .replace(&pending_handle, &pending.id, terminal)
            .expect("terminal test replacement");
        let (_, installation) = transaction
            .prepare()
            .expect("terminal test prepare")
            .into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");

        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(DecisionProviderRegistry::new()), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let error = match driver.drive_one(&mut engine, &store, None).await {
            Err(error) => error,
            Ok(_) => panic!("vacant selected action must prevent compaction"),
        };
        assert!(
            error.contains("no authoritative selected action"),
            "{error}"
        );
        assert!(!engine.action_state().has_prepared());
        assert!(
            engine
                .action_state()
                .get(&registration.families.terminal_receipts, &pending.id)
                .expect("receipt absence read")
                .is_none()
        );
    }

    #[tokio::test]
    async fn decision_response_requires_exact_live_fence_before_offer_resolution() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x36,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x46).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(RecordingDecisionProvider {
                owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                calls,
                called: Arc::new(tokio::sync::Notify::new()),
            }))
            .expect("decision provider registration");
        let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
        let dispatcher = Arc::new(
            DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
                .expect("decision dispatcher"),
        );
        let mut driver = DurableDecisionDriver::new(1, dispatcher);
        let DurableDecisionDrive::Staged(staged_dispatch) = driver
            .drive_one(&mut engine, &store, None)
            .await
            .expect("decision lease drive")
        else {
            panic!("ready decision did not stage a lease")
        };
        let StagedDecisionDispatch {
            staged,
            reservation,
        } = *staged_dispatch;
        let StagedDecisionLease { staged, worker } = staged;
        let prepared = engine
            .take_staged_action()
            .expect("lease admission")
            .expect("staged lease action");
        let (bundle, installation) = prepared.into_parts();
        assert_eq!(bundle.token(), staged.token);
        assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("lease intent reservation");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("lease reconciliation");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        drop(reservation);

        let fence = mush_core::DecisionCompletionFence::new(
            worker.controller().clone(),
            worker.decision(),
            worker.revision(),
            worker.attempt(),
            worker.lease_until().clone(),
        )
        .expect("worker completion fence");
        let response = DecisionResponse {
            token: worker.prompt().token,
            offer: worker.prompt().actions[0].offer(),
            targets: Vec::new(),
            parameters: serde_json::json!({}),
        };
        let mut forged = response.clone();
        forged.token.tag = CanonicalHash::from_bytes([0xf0; 32]);
        assert!(matches!(
            engine.decision_response_ingress(&fence, &forged),
            Err(mush_core::DecisionResponseIngressError::StaleCompletion)
        ));
        let wrong_attempt = mush_core::DecisionCompletionFence::new(
            worker.controller().clone(),
            worker.decision(),
            worker.revision(),
            worker.attempt() + 1,
            worker.lease_until().clone(),
        )
        .expect("wrong-attempt fence is structurally valid");
        assert!(matches!(
            engine.decision_response_ingress(&wrong_attempt, &response),
            Err(mush_core::DecisionResponseIngressError::StaleCompletion)
        ));

        let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
            .expect("valid response ingress");
        let first_ingress = ingress.clone();
        assert_eq!(ingress.idempotency(), &pending.action_idempotency);
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
            .await
            .expect("response preflight")
        else {
            panic!("selected action idempotency was not vacant")
        };
        let selected = ActionCoordinator::prepare_and_stage_decision_response(
            permit,
            &mut engine,
            fence.clone(),
            &response,
        )
        .expect("decision response offer resolution");
        assert_ne!(
            selected.binding.request_hash,
            CanonicalHash::from_bytes([0; 32])
        );
        let staged = engine
            .take_staged_action()
            .expect("selected action admission")
            .expect("selected staged action");
        let (_bundle, installation) = staged.into_parts();
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");

        let retry_at = worker.lease_until().clone();
        engine.set_now(retry_at.at);
        let mut retry = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("leased decision read")
            .expect("leased decision")
            .clone();
        retry
            .retry_expired_lease(retry.revision, &retry_at)
            .expect("expire first lease");
        let second_lease_until = ClockStamp {
            clock: retry_at.clock.clone(),
            at: retry_at.at + 1,
        };
        retry
            .lease(retry.revision, second_lease_until.clone())
            .expect("second lease");
        let mut transaction = engine
            .action_transaction(&registration.families.pending)
            .expect("retry transaction");
        transaction
            .replace(&registration.families.pending, &pending.id, retry)
            .expect("retry post-image");
        let prepared = transaction.prepare().expect("retry preparation");
        let (_, installation) = prepared.into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        let second_fence = mush_core::DecisionCompletionFence::new(
            worker.controller().clone(),
            worker.decision(),
            worker.revision(),
            2,
            second_lease_until,
        )
        .expect("second completion fence");
        let second_ingress =
            ActionCoordinator::decision_response_ingress(&engine, &second_fence, &response)
                .expect("second-attempt response ingress");
        assert!(matches!(
            ActionCoordinator::decision_response_ingress(&engine, &fence, &response),
            Err(crate::action_coordinator::ActionCoordinatorError::DecisionResponseIngress(_))
        ));
        assert_eq!(second_ingress.request(), first_ingress.request());
        assert_eq!(
            second_ingress.ingress_hash(),
            first_ingress.ingress_hash(),
            "operational retry fences must not change semantic action identity"
        );

        let duplicate_ingress = second_ingress.clone();
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, second_ingress)
            .await
            .expect("second response preflight")
        else {
            panic!("second-attempt selected action idempotency was not vacant")
        };
        let selected = ActionCoordinator::prepare_and_stage_decision_response(
            permit,
            &mut engine,
            second_fence.clone(),
            &response,
        )
        .expect("second-attempt response resolution");
        let binding = selected.binding.clone();
        let staged = engine
            .take_staged_action()
            .expect("selected action admission")
            .expect("selected action stage");
        let (bundle, installation) = staged.into_parts();
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("selected action intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("selected action reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        assert!(matches!(
            ActionCoordinator::preflight(&store, duplicate_ingress)
                .await
                .expect("duplicate response preflight"),
            ActionPreflight::Finalized(_)
        ));

        assert!(
            matches!(
                engine
                    .action_state()
                    .get(&registration.families.pending, &pending.id)
                    .expect("pending read")
                    .expect("pending record")
                    .status,
                PendingDecisionStatus::Leased { attempt: 2, .. }
            ),
            "selected action receipt alone must not cross the owner-local lifecycle boundary"
        );
        let resolving = stage_decision_resolving(
            &mut engine,
            &store,
            &second_fence,
            &binding,
            mush_core::DecisionTerminalOutcome::Executed,
            1,
        )
        .await
        .expect("resolving stage")
        .expect("resolving lifecycle was vacant");
        let staged = engine
            .take_staged_action()
            .expect("resolving action admission")
            .expect("resolving action stage");
        let (bundle, installation) = staged.into_parts();
        assert_eq!(bundle.token(), resolving.token);
        assert_eq!(bundle.bundle_hash(), resolving.bundle_hash);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("resolving intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("resolving reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        let resolved = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("resolved pending read")
            .expect("resolved pending record");
        assert!(matches!(
            &resolved.status,
            PendingDecisionStatus::Resolving {
                attempt: 2,
                action_binding,
                ..
            } if action_binding == &binding
        ));

        // Simulate losing the volatile post-Resolving follow-up. The bounded
        // typed-family scan must rediscover the binding, authenticate its
        // finalized ordinary receipt, and stage the exact terminal action.
        driver.cursor = None;
        driver.retry = None;
        driver.reconciliation = None;
        let DurableDecisionDrive::StagedLifecycle(terminal) = driver
            .drive_one(&mut engine, &store, None)
            .await
            .expect("resolving recovery drive")
        else {
            panic!("resolving recovery did not stage the terminal lifecycle")
        };
        let staged = engine
            .take_staged_action()
            .expect("terminal action admission")
            .expect("terminal action stage");
        let (bundle, installation) = staged.into_parts();
        assert_eq!(bundle.token(), terminal.token);
        assert_eq!(bundle.bundle_hash(), terminal.bundle_hash);
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("terminal intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("terminal reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        let terminal = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("terminal pending read")
            .expect("terminal pending record");
        assert!(matches!(
            &terminal.status,
            PendingDecisionStatus::Terminal {
                outcome: mush_core::DecisionTerminalOutcome::Executed,
                action_binding: Some(action_binding),
                ..
            } if action_binding == &binding
        ));
    }

    #[tokio::test]
    async fn delayed_response_rejects_revoked_capability_before_provider_preparation() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x5c,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x6c).await;
        let (fence, response, _worker) =
            lease_captured_decision(&mut engine, &registration, &pending, &keys);
        registration
            .capability_generation
            .store(2, AtomicOrdering::SeqCst);
        let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
            .expect("live lease still authenticates before capability resolution");
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
            .await
            .expect("capability stale preflight")
        else {
            panic!("capability stale action idempotency must remain vacant")
        };
        assert!(
            ActionCoordinator::prepare_and_stage_decision_response(
                permit,
                &mut engine,
                fence,
                &response,
            )
            .is_err()
        );
        assert!(engine.take_staged_action().unwrap().is_none());
        assert!(!engine.action_state().has_prepared());
        assert!(
            engine
                .action_state()
                .get(
                    &registration.families.selections,
                    &CanonicalId::new("selected").unwrap(),
                )
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn provider_latency_cannot_change_delayed_action_identity_or_authority() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x5d,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x6d).await;
        let (fence, expected_response, worker) =
            lease_captured_decision(&mut engine, &registration, &pending, &keys);
        let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
        let mut bundles = Vec::new();

        for kind in [
            ProjectedPolicyKind::Deterministic,
            ProjectedPolicyKind::DecisionTree,
            ProjectedPolicyKind::MockLlm,
        ] {
            let adapter = ProjectedPolicyProvider {
                owner: owner.clone(),
                kind,
            };
            let response = adapter
                .decide(worker.clone())
                .await
                .unwrap_or_else(|error| panic!("{kind:?} provider failed: {error:?}"));
            assert_eq!(response, expected_response, "{kind:?} changed selection");
            let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
                .unwrap_or_else(|error| panic!("{kind:?} ingress failed: {error}"));
            let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
                .await
                .expect("provider-policy preflight")
            else {
                panic!("{kind:?} did not retain vacant action authority")
            };
            ActionCoordinator::prepare_and_stage_decision_response(
                permit,
                &mut engine,
                fence.clone(),
                &response,
            )
            .unwrap_or_else(|error| panic!("{kind:?} action preparation failed: {error}"));
            let staged = engine
                .take_staged_action()
                .expect("provider-policy admission")
                .expect("provider-policy staged action");
            let (bundle, installation) = staged.into_parts();
            bundles.push(serde_json::to_value(&*bundle).expect("canonical bundle JSON"));
            engine
                .abort_prepared_action(installation)
                .expect("the fixture aborts");
        }

        assert!(bundles.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            engine
                .action_state()
                .get(
                    &registration.families.selections,
                    &CanonicalId::new("selected").unwrap(),
                )
                .unwrap(),
            None,
            "aborted latency variants must not install provider state"
        );
    }

    #[tokio::test]
    async fn provider_and_fallback_race_selects_one_exact_action_result() {
        let (mut engine, registration, capture_plan, keys) = decision_harness(30);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x5e,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x6e).await;
        let (fence, provider_response, _worker) =
            lease_captured_decision(&mut engine, &registration, &pending, &keys);
        let provider_ingress =
            ActionCoordinator::decision_response_ingress(&engine, &fence, &provider_response)
                .expect("provider contender ingress");

        let leased = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .unwrap()
            .unwrap()
            .clone();
        let fallback = leased
            .plan_fallback_ready(
                leased.revision,
                fence.lease_until(),
                mush_core::DecisionFallbackReason::ProviderUnavailable,
            )
            .expect("fallback race transition")
            .post_image()
            .clone();
        let mut txn = engine
            .action_transaction(&registration.families.pending)
            .expect("fallback race transaction");
        txn.replace(&registration.families.pending, &pending.id, fallback)
            .expect("fallback race post-image");
        let (_, installation) = txn.prepare().unwrap().into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");

        let page = engine
            .discover_decision_dispatches(None, 8)
            .expect("fallback discovery");
        let candidate = page
            .fallbacks()
            .first()
            .expect("fallback candidate")
            .clone();
        let fallback_response = engine
            .select_decision_fallback(&candidate)
            .expect("deterministic fallback selection");
        let fallback_ingress = engine
            .decision_fallback_ingress(&candidate, &fallback_response)
            .expect("fallback contender ingress");
        assert_eq!(fallback_response, provider_response);
        assert_eq!(fallback_ingress, provider_ingress);

        let ActionPreflight::Vacant(permit) =
            ActionCoordinator::preflight(&store, fallback_ingress)
                .await
                .unwrap()
        else {
            panic!("fallback contender must initially be vacant")
        };
        ActionCoordinator::prepare_and_stage_decision_fallback(
            permit,
            &mut engine,
            &candidate,
            &fallback_response,
        )
        .expect("fallback contender preparation");
        let staged = engine
            .take_staged_action()
            .expect("fallback action admission")
            .expect("fallback action staged");
        let (bundle, installation) = staged.into_parts();
        store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("fallback intent");
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .expect("fallback reconcile");
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");

        assert!(matches!(
            ActionCoordinator::preflight(&store, provider_ingress)
                .await
                .expect("late provider preflight"),
            ActionPreflight::Finalized(_)
        ));
        assert_eq!(
            engine
                .action_state()
                .get(
                    &registration.families.selections,
                    &CanonicalId::new("selected").unwrap(),
                )
                .unwrap()
                .expect("one selected action")
                .count,
            1
        );
    }

    // ------------------------------------------------------------------
    // ---- convergence fold ----
    // ------------------------------------------------------------------

    /// A composed module that owns no mechanics at all. It exists only so a
    /// decision-source scan has a **second** mechanics owner to leave alone.
    struct CvBystanderModule;

    impl GameModule for CvBystanderModule {
        fn name(&self) -> &'static str {
            "cv_bystander"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            Ok(())
        }
    }

    /// 08j S6 — a malformed decision freezes its **owner**, not the world.
    ///
    /// 09 §K7.4 says a mismatched pending/prompt pair, a duplicate identity or
    /// an impossible transition freezes the decision owner. Inside
    /// `DecisionSourceRegistry::scan` those three were raised by a bare `?`
    /// into the unattributed `DecisionSourceScanError::Decision`, so
    /// `DurableDecisionDriver::drive_one` left `failure_owner` unset and
    /// `world_task` took the `else` branch into a whole-world
    /// `WorldMode::IntegrityFrozen`. The scan now names the loop's owner, the
    /// driver freezes exactly that owner, and every other owner keeps running.
    #[tokio::test]
    async fn cv_6_a_malformed_decision_freezes_its_owner_not_every_other_owner() {
        let (mut engine, registration, capture_plan, keys) =
            decision_harness_with(30, vec![Box::new(CvBystanderModule)]);
        let pending = arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x7a,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, 0x7b).await;

        // Break the pending <-> prompt linkage exactly the way a hand-patched
        // or half-migrated record would: same durable identity, a revision the
        // prompt record does not carry. `validate_prompt_pair` answers
        // `DecisionError::PromptLinkage`.
        let mut tampered = engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .unwrap()
            .unwrap()
            .clone();
        tampered.revision = tampered.revision.saturating_add(1);
        let mut txn = engine
            .action_transaction(&registration.families.pending)
            .expect("tamper transaction");
        txn.replace(&registration.families.pending, &pending.id, tampered)
            .expect("tampered post-image");
        let (_, installation) = txn.prepare().unwrap().into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");

        let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
        let bystander = OwnerId::new("cv_bystander").expect("static bystander owner");
        let error = engine
            .discover_decision_dispatches(None, 8)
            .expect_err("a broken prompt pair must fail the scan closed");
        assert!(
            matches!(
                error,
                mush_core::DecisionSourceScanError::OwnedDecision { .. }
            ),
            "the fault must be attributed, not dropped: {error}"
        );
        assert_eq!(
            error.owner(),
            Some(&owner),
            "the scan loop names the decision owner"
        );

        // This is what `DurableDecisionDriver::drive_one` does once
        // `failure_owner` is set; the `else` branch is the whole-world freeze.
        engine
            .registry
            .freeze_mechanics_owner(
                &owner,
                mush_core::IncidentId(0x5106),
                mush_core::RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("decision_recovery").expect("static recovery code"),
                },
            )
            .expect("freeze the decision owner");
        assert!(
            engine
                .registry
                .mechanics_owner_freeze(&owner)
                .expect("decision owner readiness")
                .is_some(),
            "the decision owner is quarantined"
        );
        assert!(
            engine
                .registry
                .mechanics_owner_freeze(&bystander)
                .expect("bystander readiness")
                .is_none(),
            "a malformed decision from one owner freezes nothing of another"
        );

        // And the registry keeps scanning: the quarantined owner's records are
        // skipped, the world is not stopped.
        let page = engine
            .discover_decision_dispatches(None, 8)
            .expect("the scan continues past a quarantined owner");
        assert!(page.candidates().is_empty());
    }

    #[tokio::test]
    async fn delayed_tactical_gate4_stale_matrix_preserves_unselected_target_movement() {
        #[derive(Clone, Copy)]
        enum Change {
            SelectedPlacement,
            UnselectedPlacement,
            StationGrant,
            ControlLease,
            SourceComponent,
            SourceResource,
            Contact,
            Window,
            DefinitionOwner,
            BridgeOwner,
        }

        impl Change {
            const fn remains_admissible(self) -> bool {
                matches!(self, Self::UnselectedPlacement)
            }
        }

        async fn run(change: Change, salt: u8) {
            let TacticalDecisionHarness {
                mut engine,
                registration,
                tactical_state,
                unit_spec,
                capture_plan,
                keys,
            } = tactical_decision_harness(30);
            let source = GOD;
            let selected_target = mush_core::ObjId(971);
            let unselected_target = mush_core::ObjId(972);
            for object in [source, selected_target, unselected_target] {
                install_tactical_gate_unit(
                    &mut engine,
                    &tactical_state,
                    TacticalUnit::from_spec(&unit_spec, object).expect("valid tactical unit"),
                );
            }
            install_tactical_gate_placement(&mut engine, &tactical_state, source, 0, 0);
            install_tactical_gate_placement(&mut engine, &tactical_state, selected_target, 1, 0);
            install_tactical_gate_placement(&mut engine, &tactical_state, unselected_target, 2, 0);
            install_tactical_gate_weapons_lease(
                &mut engine,
                &tactical_state,
                source,
                PrincipalId::Object { id: source },
            );
            publish_tactical_gate_weapons_station(&mut engine, source, source, 1);
            install_tactical_gate_contact(
                &mut engine,
                &tactical_state,
                mush_core::ContactId::from_bytes([0x11; 16]),
                selected_target,
                PrincipalId::Object { id: source },
            );
            install_tactical_gate_contact(
                &mut engine,
                &tactical_state,
                mush_core::ContactId::from_bytes([0x22; 16]),
                unselected_target,
                PrincipalId::Object { id: source },
            );
            install_tactical_gate_fire_window(&mut engine, &tactical_state);

            assert_eq!(engine.world.command_subjects_of(source).len(), 1);
            assert_eq!(
                engine
                    .action_state()
                    .scan_family(&tactical_state.contacts, None, 8)
                    .expect("contact scan")
                    .entries()
                    .len(),
                2
            );
            assert!(
                engine
                    .action_state()
                    .get(&tactical_state.units, &source)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                engine
                    .clock_now(&mush_contract_tests::tactical_gate::tactical_gate_clock())
                    .expect("tactical clock")
                    .at,
                7
            );

            let principal = PrincipalId::Object { id: source };
            let query = engine
                .legal_action_query(principal, source, PerceivedRef::SelfActor)
                .expect("tactical decision query");
            let projection = engine
                .project_decision_offers(&query)
                .expect("tactical decision projection");
            let fire_index = projection
                .actions()
                .iter()
                .position(|offer| offer.presentation().verb == "fire")
                .unwrap_or_else(|| panic!("fire offer missing from {projection:?}"));
            let projected = projection.actions()[fire_index].clone();
            let witness = projection.offered()[fire_index].clone();
            assert_eq!(witness.offer, projected.offer());
            let _ = engine.world.take_dirty();
            let _ = engine.take_module_ops();
            let pending = arm_tactical_decision_capture(
                &keys,
                &registration,
                &capture_plan,
                source,
                projected,
                witness,
                salt,
                unix_now_s() + 60,
            );
            let store = InMemoryStore::new();
            commit_armed_capture(&mut engine, &store, &registration, salt.wrapping_add(0x10)).await;
            let (fence, response, _worker) =
                lease_captured_decision(&mut engine, &registration, &pending, &keys);
            let (chosen_contact, chosen_target) = match response.targets.as_slice() {
                [PerceivedRef::Contact { id, .. }]
                    if id == &mush_core::ContactId::from_bytes([0x11; 16]) =>
                {
                    (*id, selected_target)
                }
                [PerceivedRef::Contact { id, .. }]
                    if id == &mush_core::ContactId::from_bytes([0x22; 16]) =>
                {
                    (*id, unselected_target)
                }
                _ => panic!("unexpected tactical response target"),
            };
            let other_target = if chosen_target == selected_target {
                unselected_target
            } else {
                selected_target
            };
            let source_before_change = engine
                .action_state()
                .canonical(&tactical_state.units, &source)
                .expect("source canonical state")
                .expect("source state")
                .as_slice()
                .to_vec();
            let selected_before_change = engine
                .action_state()
                .canonical(&tactical_state.units, &chosen_target)
                .expect("target canonical state")
                .expect("target state")
                .as_slice()
                .to_vec();

            match change {
                Change::SelectedPlacement | Change::UnselectedPlacement => {
                    move_tactical_gate_placement(
                        &mut engine,
                        &tactical_state,
                        if matches!(change, Change::SelectedPlacement) {
                            chosen_target
                        } else {
                            other_target
                        },
                        4,
                        0,
                    );
                }
                Change::StationGrant => {
                    assert_eq!(
                        engine.world.retract_command_subjects(
                            mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER,
                            Some(source),
                        ),
                        1
                    );
                    let _ = engine.world.take_dirty();
                }
                Change::ControlLease => {
                    let channel = mush_core::ChannelId::new("weapons").unwrap();
                    let key = mush_core::ControlLeaseKey::new(source, channel);
                    let lease = engine
                        .action_state()
                        .get(&tactical_state.control_leases, &key)
                        .unwrap()
                        .unwrap()
                        .clone();
                    let request = RequestUid::from_bytes([salt.wrapping_add(2); 16]);
                    let requested = lease
                        .checked_request_handoff(
                            lease.revision(),
                            mush_core::ControlHandoffDraft {
                                request,
                                requested_by: PrincipalId::Object { id: source },
                                next_holder: None,
                                requested_at:
                                    mush_contract_tests::tactical_gate::tactical_gate_tick(7),
                                boundary: mush_contract_tests::tactical_gate::tactical_gate_tick(8),
                                reason: CanonicalId::new("gate4_revoke").unwrap(),
                            },
                        )
                        .unwrap();
                    let revoked = requested
                        .checked_apply_handoff(
                            requested.revision(),
                            request,
                            mush_contract_tests::tactical_gate::tactical_gate_tick(8),
                        )
                        .unwrap();
                    let mut txn = engine
                        .action_transaction(&tactical_state.control_leases)
                        .expect("lease-revoke transaction");
                    txn.replace(&tactical_state.control_leases, &key, revoked)
                        .expect("lease-revoke post-image");
                    let (_, installation) = txn.prepare().unwrap().into_parts();
                    engine
                        .install_composed_action(installation)
                        .expect("the fixture installs");
                }
                Change::SourceComponent => {
                    disable_tactical_gate_weapon(&mut engine, &tactical_state, source);
                }
                Change::SourceResource => {
                    exhaust_tactical_gate_ammo(&mut engine, &tactical_state, source);
                }
                Change::Contact => {
                    let contact = engine
                        .action_state()
                        .get(&tactical_state.contacts, &chosen_contact)
                        .unwrap()
                        .unwrap()
                        .clone();
                    let lost = contact
                        .checked_mark_lost(
                            contact.revision(),
                            mush_contract_tests::tactical_gate::tactical_gate_tick(7),
                            CanonicalId::new("gate4_occluded").unwrap(),
                        )
                        .unwrap();
                    let mut txn = engine
                        .action_transaction(&tactical_state.contacts)
                        .expect("contact-loss transaction");
                    txn.replace(&tactical_state.contacts, &chosen_contact, lost)
                        .expect("contact-loss post-image");
                    let (_, installation) = txn.prepare().unwrap().into_parts();
                    engine
                        .install_composed_action(installation)
                        .expect("the fixture installs");
                }
                Change::Window => {
                    let id = mush_contract_tests::tactical_gate::tactical_gate_window_id();
                    let window = engine
                        .action_state()
                        .get(&tactical_state.windows, &id)
                        .unwrap()
                        .unwrap()
                        .clone();
                    let closed = window
                        .checked_close_due(
                            window.revision(),
                            RequestUid::from_bytes([salt.wrapping_add(1); 16]),
                            &mush_contract_tests::tactical_gate::tactical_gate_tick(10),
                        )
                        .unwrap()
                        .into_record();
                    let mut txn = engine
                        .action_transaction(&tactical_state.windows)
                        .expect("window-close transaction");
                    txn.replace(&tactical_state.windows, &id, closed)
                        .expect("window-close post-image");
                    let (_, installation) = txn.prepare().unwrap().into_parts();
                    engine
                        .install_composed_action(installation)
                        .expect("the fixture installs");
                }
                Change::BridgeOwner => {
                    engine
                        .registry
                        .freeze_mechanics_owner(
                            &OwnerId::new(
                                mush_contract_tests::tactical_gate::TACTICAL_PLACEMENT_OWNER,
                            )
                            .unwrap(),
                            mush_core::IncidentId(u64::from(salt)),
                            mush_core::RecoveryReason::IntegrityViolation {
                                code: CanonicalId::new("gate4_bridge_freeze").unwrap(),
                            },
                        )
                        .expect("bridge owner freeze");
                }
                Change::DefinitionOwner => {
                    engine
                        .registry
                        .freeze_mechanics_owner(
                            &OwnerId::new(mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER)
                                .unwrap(),
                            mush_core::IncidentId(u64::from(salt)),
                            mush_core::RecoveryReason::IntegrityViolation {
                                code: CanonicalId::new("gate4_definition_freeze").unwrap(),
                            },
                        )
                        .expect("definition owner freeze");
                }
            }
            let (source_before, selected_before) = if matches!(change, Change::DefinitionOwner) {
                (source_before_change, selected_before_change)
            } else {
                (
                    engine
                        .action_state()
                        .canonical(&tactical_state.units, &source)
                        .expect("source canonical state after stale change")
                        .expect("source state after stale change")
                        .as_slice()
                        .to_vec(),
                    engine
                        .action_state()
                        .canonical(&tactical_state.units, &chosen_target)
                        .expect("target canonical state after stale change")
                        .expect("target state after stale change")
                        .as_slice()
                        .to_vec(),
                )
            };
            let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
                .expect("exact live decision response ingress");
            let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
                .await
                .expect("tactical decision preflight")
            else {
                panic!("tactical decision idempotency must be vacant")
            };
            let result = ActionCoordinator::prepare_and_stage_decision_response(
                permit,
                &mut engine,
                fence,
                &response,
            );

            if matches!(change, Change::SelectedPlacement) {
                assert!(matches!(
                    &result,
                    Err(crate::action_coordinator::ActionCoordinatorError::StaleGuards)
                ));
            }

            if !matches!(change, Change::DefinitionOwner) {
                assert_eq!(
                    engine
                        .action_state()
                        .canonical(&tactical_state.units, &source)
                        .unwrap()
                        .unwrap()
                        .as_slice(),
                    source_before
                );
                assert_eq!(
                    engine
                        .action_state()
                        .canonical(&tactical_state.units, &chosen_target)
                        .unwrap()
                        .unwrap()
                        .as_slice(),
                    selected_before
                );
            }
            if !change.remains_admissible() {
                assert!(
                    result.is_err(),
                    "stale delayed action must fail before prepare"
                );
                assert!(engine.take_staged_action().unwrap().is_none());
                assert!(!engine.action_state().has_prepared());
            } else {
                result.expect("unselected movement must remain admissible");
                let staged = engine
                    .take_staged_action()
                    .expect("selected action admission")
                    .expect("selected action staged");
                let (bundle, installation) = staged.into_parts();
                assert!(bundle.resolution().is_some(), "RNG evidence is durable");
                assert_eq!(bundle.semantic_events().len(), 1);
                assert!(bundle.durable_outbox().is_empty());
                engine
                    .abort_prepared_action(installation)
                    .expect("the fixture aborts");
            }
        }

        for (change, salt) in [
            (Change::SelectedPlacement, 0x61),
            (Change::UnselectedPlacement, 0x62),
            (Change::StationGrant, 0x63),
            (Change::ControlLease, 0x64),
            (Change::SourceComponent, 0x65),
            (Change::SourceResource, 0x66),
            (Change::Contact, 0x67),
            (Change::Window, 0x68),
            (Change::DefinitionOwner, 0x69),
            (Change::BridgeOwner, 0x6a),
        ] {
            run(change, salt).await;
        }
    }

    #[tokio::test]
    async fn durable_schedule_poll_executes_one_atomic_cursor_and_domain_bundle() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_test_writer()
            .try_init();
        use mush_contract_tests::{
            TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
            toy_definitions, toy_world_with_state,
        };
        use mush_core::{
            DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
            ScheduleRestartPolicy,
        };

        let (mut world, families) = toy_world_with_state();
        install_toy_focus(world.engine_mut(), &families, GOD, 1);
        let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x31; 16]));
        let (prepared, _) = world
            .engine_mut()
            .prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                DurableCommitToken::from_bytes([0x32; 16]),
            )
            .expect("prepare guarded fixture");
        let (_, installation) = prepared.into_parts();
        world
            .engine_mut()
            .install_composed_action(installation)
            .expect("the fixture installs");
        let effect = world
            .engine()
            .action_state()
            .get(&families.guarded, &GOD)
            .unwrap()
            .unwrap()
            .effect
            .id();
        let owner = OwnerId::new(TOY_OWNER).unwrap();
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("world_poll_expiry").unwrap())
                .unwrap(),
            owner,
            clock: mush_core::ClockRef::Utc,
            next_due: mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 0,
            },
            cadence: None,
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .unwrap();
        let schedule_key = format!(
            "action-state/v1/schedules/{}",
            schedule.id().canonical_hash().to_hex()
        );
        install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);

        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        let handles =
            spawn_persist_with_gateway(world.into_engine(), deps, |_| Arc::new(BlackholeGateway));
        let mut completed = false;
        for _ in 0..40 {
            if store
                .module_json_get(TOY_OWNER, &schedule_key)
                .await
                .unwrap()
                .is_some_and(|document| document["value"]["state"] == "completed")
            {
                completed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let persisted = store.module_json_list(TOY_OWNER).await.unwrap();
        assert!(
            completed,
            "durable schedule was not committed by the wake driver: {persisted:?}"
        );
        assert!(
            store
                .module_json_get(TOY_OWNER, "action-state/v1/guarded/1")
                .await
                .unwrap()
                .is_none(),
            "the cursor and effect removal must share one authoritative commit"
        );

        assert!(
            handles
                .handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        handles.world_join.await.unwrap();
        handles.writer_join.await.unwrap();
        handles.outbox_join.await.unwrap();
    }

    #[tokio::test]
    async fn durable_schedule_driver_rebases_retired_server_epoch() {
        use mush_contract_tests::{
            TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
            toy_definitions, toy_world_with_state,
        };
        use mush_core::{
            DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
            ScheduleRestartPolicy,
        };

        let (mut world, families) = toy_world_with_state();
        install_toy_focus(world.engine_mut(), &families, GOD, 1);
        let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x41; 16]));
        let (prepared, _) = world
            .engine_mut()
            .prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                DurableCommitToken::from_bytes([0x42; 16]),
            )
            .expect("prepare guarded fixture");
        let (_, installation) = prepared.into_parts();
        world
            .engine_mut()
            .install_composed_action(installation)
            .expect("the fixture installs");
        let effect = world
            .engine()
            .action_state()
            .get(&families.guarded, &GOD)
            .unwrap()
            .unwrap()
            .effect
            .id();
        let owner = OwnerId::new(TOY_OWNER).unwrap();
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("retired_epoch_expiry").unwrap())
                .unwrap(),
            owner,
            clock: mush_core::ClockRef::Utc,
            next_due: mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 90,
            },
            cadence: None,
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .unwrap();
        install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);
        world.world_mut().set_now(100);
        let candidate = world
            .engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 1)
            .unwrap()
            .candidates()[0]
            .clone();
        let old_ingress =
            ActionCoordinator::scheduled_ingress(candidate.action(), candidate.advance(), 1)
                .unwrap();
        let store = mush_store::InMemoryStore::new();
        store
            .retire_authoritative_epochs(old_ingress.idempotency().scope(), 9)
            .await
            .expect("seed retired scheduler epoch");
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = DurableScheduleDriver::new(1);

        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            DurableScheduleDrive::Staged(_)
        ));
        let staged = engine
            .take_staged_action()
            .unwrap()
            .expect("rebased schedule action staged");
        let (bundle, installation) = staged.into_parts();
        assert_eq!(bundle.idempotency().epoch(), 9);
        assert_eq!(driver.idempotency_epoch, 9);
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
    }

    #[tokio::test]
    async fn durable_schedule_driver_discards_stale_retained_candidate() {
        use mush_contract_tests::{
            TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
            toy_definitions, toy_world_with_state,
        };
        use mush_core::{
            DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
            ScheduleRestartPolicy,
        };

        let (mut world, families) = toy_world_with_state();
        install_toy_focus(world.engine_mut(), &families, GOD, 1);
        let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x51; 16]));
        let (prepared, _) = world
            .engine_mut()
            .prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                DurableCommitToken::from_bytes([0x52; 16]),
            )
            .expect("prepare guarded fixture");
        let (_, installation) = prepared.into_parts();
        world
            .engine_mut()
            .install_composed_action(installation)
            .expect("the fixture installs");
        let effect = world
            .engine()
            .action_state()
            .get(&families.guarded, &GOD)
            .unwrap()
            .unwrap()
            .effect
            .id();
        let owner = OwnerId::new(TOY_OWNER).unwrap();
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("stale_retained").unwrap()).unwrap(),
            owner,
            clock: mush_core::ClockRef::Utc,
            next_due: mush_core::ClockStamp {
                clock: mush_core::ClockRef::Utc,
                at: 90,
            },
            cadence: None,
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .unwrap();
        let schedule_id = schedule.id();
        install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);
        world.world_mut().set_now(100);
        let candidate = world
            .engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 1)
            .unwrap()
            .candidates()[0]
            .clone();

        // A competing authoritative transition changes the cursor after this
        // candidate was retained for retry. The old candidate is ordinary
        // optimistic-concurrency loss, not an integrity breach.
        let mut transaction = world
            .engine_mut()
            .action_transaction(&families.schedules)
            .expect("schedule update transaction");
        transaction
            .replace(
                &families.schedules,
                &schedule_id,
                candidate.advance().post_image().clone(),
            )
            .expect("install competing cursor advance");
        let prepared = transaction.prepare().expect("prepare cursor update");
        let (_, installation) = prepared.into_parts();
        world
            .engine_mut()
            .install_composed_action(installation)
            .expect("the fixture installs");
        let _ = world.drain_write_batch();

        let mut engine = world.into_engine();
        let mut driver = DurableScheduleDriver::new(1);
        driver.retry = Some(candidate);
        let store = mush_store::InMemoryStore::new();
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            DurableScheduleDrive::DiscardedStale(_)
        ));
        assert!(driver.retry.is_none());
        assert!(!engine.action_state().has_prepared());
        assert!(engine.take_staged_action().unwrap().is_none());
    }

    #[derive(Default)]
    struct RecordingGateway {
        discord: Mutex<Vec<(u64, DiscordControlOp)>>,
        sends: Mutex<Vec<(SendTarget, Arc<OutputEvent>)>>,
        /// The length of every `transmit` call, in order (16b cut AP.4: a
        /// bracketed world-task refusal is ONE batch).
        batches: Mutex<Vec<usize>>,
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
        fn discord_control(&self, character: u64, op: DiscordControlOp) {
            self.discord.lock().unwrap().push((character, op));
        }
        fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
            self.batches
                .lock()
                .expect("recording gateway poisoned")
                .push(sends.len());
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

    #[tokio::test]
    async fn all_in_one_offer_submission_reaches_durable_ownership() {
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            base_registry(),
            vec![Box::new(mush_tactics::TacticsModule::new())],
        )
        .expect("tactics module composition");
        engine.set_offer_keys(
            mush_core::OfferKeyRing::new(
                mush_core::OfferSecret::new(1, [0x51; 32]).expect("test offer secret"),
                [],
            )
            .expect("test offer key ring"),
        );

        let store = Arc::new(InMemoryStore::new());
        let recorder = Arc::new(RecordingGateway::default());
        let sink = Arc::clone(&recorder);
        let handles = spawn_persist_with_gateway(
            engine,
            ServerDeps {
                store: store.clone(),
                guard: Arc::new(IpGuard::new()),
                staff: Arc::new(StaffQueue::new()),
                llm: None,
                metrics: Arc::new(Metrics::new()),
                quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
                outbox_adapters: Vec::new(),
            },
            move |_| -> Gateway { sink },
        );
        let PersistHandles {
            handle,
            world_join,
            writer_join,
            outbox_join,
            ..
        } = handles;

        const CONN: u64 = 0x5151;
        assert!(
            handle
                .inbound(Inbound::ConnUp {
                    conn: CONN,
                    actor: GOD,
                    caps_structured: true,
                    from_linkdead: false,
                })
                .await
        );
        assert!(
            handle.barrier().await,
            "connection must become authoritative"
        );
        let offer = handle
            .legal_actions(CONN, GOD, PerceivedRef::SelfActor)
            .await
            .expect("legal-action projection")
            .into_iter()
            .find(|offer| offer.presentation().verb == "brace")
            .expect("tactics brace offer");

        let ActionSend::Finished(receipt) = handle
            .submit_legal_action(GatewayAction {
                conn: CONN,
                actor: GOD,
                offered_action: offer.offer(),
                subject: offer.subject().clone(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
            .await
        else {
            panic!("all-in-one action did not return its exact projected receipt");
        };
        assert_eq!(receipt.result().unwrap().summary(), "Brace committed.");
        assert!(handle.barrier().await, "authoritative receipt must settle");
        let stance = store
            .module_json_get("tactics", "action-state/v1/stance/1")
            .await
            .expect("read persisted tactics stance")
            .expect("brace post-image must be durable");
        assert_eq!(stance["value"]["braced"], true);
        assert_eq!(stance["value"]["revision"], 1);

        assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
        tokio::time::timeout(Duration::from_secs(2), world_join)
            .await
            .expect("world shutdown timed out")
            .expect("world task panicked");
        tokio::time::timeout(Duration::from_secs(2), writer_join)
            .await
            .expect("writer shutdown timed out")
            .expect("writer task panicked");
        tokio::time::timeout(Duration::from_secs(2), outbox_join)
            .await
            .expect("outbox shutdown timed out")
            .expect("outbox task panicked");
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
            context: &mush_core::RevisionValidatorContext<'_>,
            _key: &RevisionKey,
        ) -> Result<Option<u64>, ActionError> {
            Ok(context
                .module_state::<PreparedLaneGeneration>()
                .map(|generation| generation.0))
        }
    }

    /// The I13 seam's audience in this fixture: an observer that reports every
    /// announced family to GOD, so a test can tell which install path ran.
    struct PreparedLaneAnnounce;

    impl mush_core::WorldObserver for PreparedLaneAnnounce {
        fn on_event(&self, ctx: &mut mush_core::Ctx<'_>, ev: &WorldEvent) {
            if let WorldEvent::ActionStateChanged {
                owner,
                family,
                keys,
            } = ev
            {
                ctx.out
                    .info(GOD, format!("announced {owner}/{family} {keys:?}"));
            }
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
            registrar.subscribe(
                mush_core::WorldEventKind::ActionStateChanged,
                Arc::new(PreparedLaneAnnounce),
            );
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
        installation.into()
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
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome_definition.clone()],
            writes: prepared.authoritative_replay_writes(),
            outcome: RulesValue::new(
                outcome_definition.clone(),
                serde_json::json!({ "accepted": true }),
            )
            .unwrap(),
            resolution: None,
            semantic_events: vec![
                mush_core::AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &action,
                    RulesValue::new(outcome_definition, serde_json::json!({ "accepted": true }))
                        .unwrap(),
                    vec![principal.clone()],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![principal],
            durable_outbox: vec![],
        })
        .unwrap()
    }

    /// Play the writer's side of one queued K3 job against an in-memory store:
    /// reserve the exact intent, release ownership, then reconcile and send the
    /// exact receipt. Returns the bundle the world actually queued.
    async fn settle_queued_authoritative_job(job: StoreJob) -> AuthoritativeBundle {
        let StoreJob::Authoritative {
            bundle,
            ownership,
            notify,
        } = job
        else {
            panic!("the prepared lane must queue one exact authoritative bundle");
        };
        let store = InMemoryStore::new();
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        let intent = store
            .reserve_authoritative_intent((*bundle).clone())
            .await
            .expect("the exact intent reserves");
        ownership
            .send(Ok(intent))
            .expect("the world still owns the ownership channel");
        let exact = store
            .reconcile_authoritative(token, hash)
            .await
            .expect("the exact receipt reconciles");
        notify
            .send(Ok(exact))
            .expect("the world still owns the exact receipt");
        *bundle
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
            action: None,
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
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
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

    /// 16b cut AC.1 (16c `B-82`): a probe that stands where the tacsim module
    /// stands on the effect dispatcher. Its `name()` is `tacsim::store::NS`, the
    /// name the dispatcher's tacsim arm matches AND the name
    /// `mush_core::apply_async_completion` routes the re-entry by; no real
    /// tacsim module is composed in this rig, so the name is free.
    struct DeliberationProbeModule {
        /// The re-entered completion payload, once it arrives.
        reply: Arc<Mutex<Option<String>>>,
    }

    struct DeliberationProbeCompletion(Arc<Mutex<Option<String>>>);

    impl AsyncCompletion for DeliberationProbeCompletion {
        fn on_complete(&self, _ctx: &mut Ctx<'_>, _correlation: u64, payload: &str) {
            *self.0.lock().expect("the probe's reply slot") = Some(payload.to_string());
        }
    }

    impl GameModule for DeliberationProbeModule {
        fn name(&self) -> &'static str {
            tacsim::store::NS
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
            reg.command(
                CommandSpec {
                    name: "probe-deliberate".into(),
                    shape: ArgShape::None,
                    help: "test-only tacsim deliberation effect".into(),
                    ..CommandSpec::default()
                },
                Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                    let payload =
                        serde_json::to_string(&tacsim::deliberation::DeliberationJob::Deliberate {
                            brief: probe_squad_brief(),
                        })
                        .expect("a brief encodes");
                    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
                        module: SmolStr::new_static(tacsim::store::NS),
                        actor: GOD,
                        correlation: 1,
                        payload,
                    }));
                    Ok(())
                }),
            )?;
            reg.async_completion(Arc::new(DeliberationProbeCompletion(Arc::clone(
                &self.reply,
            ))))?;
            Ok(())
        }
    }

    /// The `SquadBrief` of `tacsim::deliberation`'s own round-trip test
    /// (`a_job_and_a_reply_round_trip_their_wire_forms`), copied: every field
    /// is public, and `contacts` stays empty so the un-re-exported
    /// `BriefContact` is never named.
    fn probe_squad_brief() -> tacsim::deliberation::SquadBrief {
        use tacsim::deliberation::{BriefMember, SquadBrief, SquadId};
        SquadBrief {
            squad: SquadId {
                team: tacsim::arena::state::Team::Red,
                index: 1,
            },
            match_id: 7,
            commander: 4,
            // 17b cut L.1: the squad's lance-leader, never in the prompt.
            leader: tacsim::deliberation::CharacterRef::Persona {
                squad: SquadId {
                    team: tacsim::arena::state::Team::Red,
                    index: 1,
                },
            },
            reason: "no_order".to_string(),
            units: 2,
            health_permille: 1_000,
            states: vec![("patrol".to_string(), 2)],
            cell: Some((1, 1)),
            window: (16, 16),
            default_ttl_ms: 30_000,
            contacts: Vec::new(),
            members: vec![
                BriefMember {
                    unit: 4,
                    ordinal: 1,
                    state: "patrol".to_string(),
                    health_permille: 1_000,
                    band: "here".to_string(),
                    bearing_deg: 0,
                },
                BriefMember {
                    unit: 5,
                    ordinal: 2,
                    state: "patrol".to_string(),
                    health_permille: 1_000,
                    band: "close".to_string(),
                    bearing_deg: 270,
                },
            ],
            objective: None,
        }
    }

    /// `deliberation.rs`'s `mock_service` shape: a tool-calling
    /// `MockProvider` named `openrouter` (where route derivation sends every
    /// non-`anthropic/` model) behind the `tacsim_deliberate` route.
    fn probe_mock_service() -> (Arc<LlmService>, Arc<mush_llm::MockProvider>) {
        let mock = Arc::new(mush_llm::MockProvider::new("openrouter").with_caps(
            mush_llm::ProviderCaps {
                tool_calling: true,
                forced_tool_choice: true,
                ..mush_llm::ProviderCaps::default()
            },
        ));
        let provider: Arc<dyn mush_llm::LlmProvider> = mock.clone();
        let llm = Arc::new(
            LlmService::builder()
                .provider("openrouter", provider)
                .route(
                    "tacsim_deliberate",
                    mush_llm::RouteConfig {
                        model: "z-ai/glm-5.3-flash".to_string(),
                        max_tokens: 512,
                        timeout_s: 30,
                        reasoning: None,
                        reasoning_max_tokens: None,
                    },
                )
                .global_enabled(true)
                .build(),
        );
        (llm, mock)
    }

    /// Spawn a world with the deliberation probe over `llm`, raise one tacsim
    /// `Effect::ModuleAsync`, wait for its re-entry, shut the world down, and
    /// return the re-entered reply.
    async fn probe_deliberation_reply(
        llm: Option<Arc<LlmService>>,
    ) -> tacsim::deliberation::DeliberationReply {
        let reply = Arc::new(Mutex::new(None));
        let engine = Engine::with_modules(
            World::new_minimal(),
            base_registry(),
            vec![Box::new(DeliberationProbeModule {
                reply: Arc::clone(&reply),
            })],
        )
        .expect("the deliberation probe composes");
        let recorder = Arc::new(RecordingGateway::default());
        let handles = spawn_persist_with_gateway(
            engine,
            ServerDeps {
                llm,
                ..ServerDeps::in_memory()
            },
            move |_| -> Gateway { recorder },
        );
        assert!(
            handles
                .handle
                .inbound(Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: "probe-deliberate".into(),
                })
                .await
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let payload = loop {
            if let Some(payload) = reply.lock().expect("the probe's reply slot").clone() {
                break payload;
            }
            assert!(
                Instant::now() < deadline,
                "the dispatcher must run the deliberation against `deps.llm` and re-enter \
                 its reply (or re-enter the refusal when there is none)"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            handles
                .handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        handles.world_join.await.unwrap();
        handles.writer_join.await.unwrap();
        handles.outbox_join.await.unwrap();
        serde_json::from_str(&payload).expect("the re-entered payload is a DeliberationReply")
    }

    /// **The effect dispatcher runs a squad's deliberation against `deps.llm`,
    /// and refuses it without one** (16b cut AC.1, 16c `B-82`).
    ///
    /// The split half used to hand the world task no service, so on dev1 every
    /// deliberation took the second branch below. With the service, the mock's
    /// default empty answer never parses — `structured.rs` makes the original
    /// attempt and one repair, then refuses by name — so the proof that the
    /// call LEFT the world task is a `malformed` refusal after exactly two
    /// calls (read after the re-entry, so the count is exact). Without it, the
    /// dispatcher re-enters `unavailable_reply` and nothing is called. This is
    /// the cheap pin; `mush-gateway`'s `two_process_llm.rs` is the expensive
    /// form of it on the production shape.
    #[tokio::test]
    async fn a_tacsim_module_async_effect_runs_against_the_service_and_is_refused_without_it() {
        use tacsim::deliberation::{DeliberationReply, RefusedReason};

        let (llm, mock) = probe_mock_service();

        // The control first: the same world with no service is the B-82 shape.
        match probe_deliberation_reply(None).await {
            DeliberationReply::Refused { reason, .. } => assert_eq!(
                reason,
                RefusedReason::Unavailable.as_str(),
                "a world task with no service re-enters `unavailable_reply`"
            ),
            other => panic!("a world with no service cannot make an order: {other:?}"),
        }
        assert_eq!(
            mock.call_count(),
            0,
            "this world was never handed the service, so nothing was called; the \
             observed thing is the refusal"
        );

        // The same world over the service.
        match probe_deliberation_reply(Some(Arc::clone(&llm))).await {
            DeliberationReply::Refused { reason, .. } => assert_eq!(
                reason,
                RefusedReason::Malformed.as_str(),
                "the mock's empty answer never parses, so the service's refusal is `malformed`"
            ),
            other => panic!("the empty answer cannot make an order: {other:?}"),
        }
        assert_eq!(
            mock.call_count(),
            2,
            "the original attempt plus one repair: the deliberation reached the provider"
        );
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
                account: None,
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
        // A relayed line speaks on a channel: a world mutation.
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::RelaySpeak {
                req: mush_ipc::RelaySpeak {
                    user_id: 1,
                    account: mush_core::AccountId::mint("nick", 1).unwrap(),
                    actor: GOD,
                    channel: "Public".to_string(),
                    kind: mush_proto::NarrativeKind::Say,
                    text: "hi".to_string(),
                },
                reply,
            }),
            IngressClass::WorldMutation
        );
        // A forwarded Discord interaction runs a module handler that may
        // mutate: a world mutation, never a read.
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::DiscordInteraction {
                interaction: mush_ipc::DiscordInteraction {
                    user_id: 1,
                    account: None,
                    actor: None,
                    custom_id: "oncall:ack:1".to_string(),
                    values: vec![],
                },
                reply,
            }),
            IngressClass::WorldMutation
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
            classify_world_msg(&WorldMsg::RecheckPrivilege {
                actor: GOD,
                privilege: Privileges::DISCORD_IDENTITY,
                subject: None,
                command: "@discord/show",
                reply,
            }),
            IngressClass::WorldMutation
        );
        let (reply, _rx) = oneshot::channel();
        assert_eq!(
            classify_world_msg(&WorldMsg::Barrier { reply }),
            IngressClass::OrderingBarrier
        );
    }

    /// The disclosure is refused whenever the world cannot say the privilege
    /// is still held: revoked, busy, or gone. Nothing about a failed read may
    /// be read as an authorization.
    #[tokio::test]
    async fn discord_show_refuses_unless_the_world_re_confirms_the_privilege() {
        let (tx, mut rx) = mpsc::channel::<WorldMsg>(4);
        let weak = tx.downgrade();
        let target = ObjId(7);

        // The world answers the re-check; the first says the grant is gone.
        let answers = tokio::spawn(async move {
            let mut sent = Vec::new();
            while let Some(msg) = rx.recv().await {
                let WorldMsg::RecheckPrivilege {
                    actor,
                    privilege,
                    subject,
                    command,
                    reply,
                } = msg
                else {
                    continue;
                };
                sent.push((actor, privilege, subject, command));
                let held = sent.len() > 2;
                let _ = reply.send(if sent.len() == 2 {
                    Err(WorldReadError::Busy)
                } else {
                    Ok(held)
                });
            }
            sent
        });

        // (1) Revoked in the window.
        let refusal = discord_show_recheck(&weak, GOD, target)
            .await
            .expect_err("a revoked privilege must refuse");
        assert!(refusal.contains("revoked"), "{refusal}");
        assert!(refusal.contains("Permission denied"), "{refusal}");
        // (2) The world is committing and cannot answer: refuse too.
        let refusal = discord_show_recheck(&weak, GOD, target)
            .await
            .expect_err("an unanswerable re-check must refuse");
        assert!(refusal.contains("could not be re-checked"), "{refusal}");
        // (3) Still held: the executor proceeds to the disclosure.
        discord_show_recheck(&weak, GOD, target)
            .await
            .expect("a held privilege discloses");

        drop(tx);
        let sent = answers.await.expect("answering task");
        assert_eq!(sent.len(), 3);
        for (actor, privilege, subject, command) in sent {
            assert_eq!(actor, GOD);
            assert_eq!(privilege, Privileges::DISCORD_IDENTITY);
            assert_eq!(subject, Some(target));
            assert_eq!(command, "@discord/show");
        }

        // (4) The world task is gone: no disclosure on a dead channel.
        let refusal = discord_show_recheck(&weak, GOD, target)
            .await
            .expect_err("a gone world must refuse");
        assert!(refusal.contains("shutting down"), "{refusal}");
    }

    #[test]
    fn only_transferred_semantic_or_lifecycle_custody_uses_deferred_slot() {
        assert!(!must_defer_behind_commit(&WorldMsg::Inbound(cmd("look"))));
        let (reply, _rx) = oneshot::channel();
        assert!(!must_defer_behind_commit(&WorldMsg::CreatePlayer {
            name: "RetryMe".to_string(),
            account: None,
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
            (
                StaffOpKind::LiftTempban {
                    ip,
                    issued_unix_ms: 1_000,
                },
                IngressClass::StaffMutation,
            ),
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

    /// **The account map's two halves, through the real world task** (16b cut
    /// F.4).
    ///
    /// The insert rides on `CreatePlayer` and must be visible before the create
    /// reply lands — that is the whole of the K6 fix. The undo is
    /// `UnlinkPlayer`, which the caller sends when the account document the
    /// insert was made against could not be written: without it the running
    /// server would hold a mapping that no boot could ever re-derive, and the
    /// two halves would disagree in the permissive direction.
    #[tokio::test]
    async fn f4_the_account_map_learns_on_create_and_forgets_on_unlink() {
        use mush_core::AccountIdentity;

        let map = Arc::new(crate::accounts_identity::AccountMap::default());
        let mut engine = Engine::new(World::new_minimal(), base_registry());
        engine.set_account_identity(Arc::clone(&map) as Arc<dyn AccountIdentity>);
        let handles = spawn_persist_with_gateway(engine, ServerDeps::in_memory(), |_| {
            Arc::new(BlackholeGateway) as Gateway
        });

        let account = mush_core::AccountId::mint("Nick", 1).expect("a well-formed account id");
        let nicky = handles
            .handle
            .create_player("Nicky", Some(account))
            .await
            .expect("the world created the character");
        assert_eq!(
            map.account_of(nicky),
            Some(account),
            "the create turn must have told the oracle before its reply left"
        );

        // The compensating undo, and its idempotence.
        assert_eq!(handles.handle.unlink_player(nicky).await, Ok(true));
        assert_eq!(
            map.account_of(nicky),
            None,
            "an orphan on disk is an orphan in RAM"
        );
        assert_eq!(
            handles.handle.unlink_player(nicky).await,
            Ok(false),
            "forgetting twice removes nothing"
        );

        // A create with no resolvable account id maps nothing and still works.
        let ghost = handles
            .handle
            .create_player("Ghost", None)
            .await
            .expect("the world created the character");
        assert_eq!(map.account_of(ghost), None);

        assert!(
            handles
                .handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        handles.world_join.await.unwrap();
        handles.writer_join.await.unwrap();
        handles.outbox_join.await.unwrap();
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
                outbox_adapters: Vec::new(),
            },
            Arc::new(DurableOutboxRegistry::new()),
            None,
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
            handle.create_player("BlockedPlayer", None).await,
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
            &store,
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
            action: None,
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
        let bundle = prepared_lane_bundle(&prepared, 7);
        let expected_token = bundle.token();
        let guards = empty_guards(&engine);
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .expect("clean engine accepts prepared action");

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
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
        let queued = settle_queued_authoritative_job(
            writer_rx.recv().await.expect("prepared write was queued"),
        )
        .await;
        assert_eq!(queued.token(), expected_token);
        assert_eq!(queued.writes().len(), 1);

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

        let mut receipt = receipt;
        let ack = receipt.receive().await;
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
            &test_effect_store(),
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
    /// Mint one prepared installation against a *different* engine's action
    /// store. Applying it anywhere else is a pure wiring bug: nothing a player
    /// can type produces one.
    fn foreign_prepared_installation() -> PreparedInstallation {
        let (mut other, other_handle) = prepared_lane_fixture();
        let mut transaction = other
            .action_transaction(&other_handle)
            .expect("test transaction begins");
        transaction
            .insert(&other_handle, &41, PreparedLaneState { amount: 7 })
            .expect("test state is valid");
        transaction
            .prepare()
            .expect("test transaction prepares")
            .into_parts()
            .1
            .into()
    }

    /// Whether staff were told, in the words `apply_integrity_freeze` uses.
    fn wizard_heard(recorder: &RecordingGateway, needle: &str) -> bool {
        recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .any(|(_, event)| match event.as_ref() {
                OutputEvent::Sys(text) => text.to_plain_text().contains(needle),
                _ => false,
            })
    }

    /// core-action-7: the commit seam used to `assert!` when a prepared
    /// installation did not belong to the engine's own store — on the one task
    /// that owns the world and may never panic (docs/PLAN.md). Now the seam
    /// fails closed: persistence freezes, staff are told, nothing is announced
    /// to observers, no typed image goes live, and this call RETURNS. The
    /// liveness assertion is the point — the failure mode is "a wiring bug
    /// takes down the single writer".
    #[test]
    fn a_foreign_prepared_installation_freezes_the_commit_seam_and_the_writer_lives() {
        let (mut engine, handle) = prepared_lane_fixture();
        engine.world.set_connected(GOD, true);
        let pending = pending_with_prepared_installation(9, foreign_prepared_installation());
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);

        let completed = complete_pending_commit(
            pending,
            Ok(()),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );

        // Reaching this line at all is the proof the single writer survived.
        assert_eq!(completed.outcome, DeliveryOutcome::Committed);
        assert!(health.mode.integrity_frozen());
        assert!(persistence_integrity.is_latched());
        assert!(wizard_heard(&recorder, "persistence integrity breach"));
        assert_eq!(
            engine.action_state().revision(),
            0,
            "a refused install must leave live action state untouched"
        );
        assert!(
            engine
                .action_state()
                .get(&handle, &41)
                .expect("state lookup succeeds")
                .is_none(),
            "no typed post-image may go live from a refused installation"
        );
        assert!(
            !wizard_heard(&recorder, "announced "),
            "an install that did not happen must not be announced to observers \
             (the fixture's I13 observer says `announced <owner>/<family>`)"
        );
    }

    /// The refusal half of the same seam: an abort that does not match the
    /// store freezes rather than asserting, and the loop still returns.
    #[test]
    fn a_foreign_installation_on_the_abort_path_freezes_and_the_writer_lives() {
        let (mut engine, _handle) = prepared_lane_fixture();
        engine.world.set_connected(GOD, true);
        let pending = pending_with_prepared_installation(9, foreign_prepared_installation());
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);

        let completed = complete_pending_commit(
            pending,
            Err(AuthoritativeCommitFailure::Refused(
                "store refused the batch".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );

        assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
        assert!(health.mode.integrity_frozen());
        assert!(persistence_integrity.is_latched());
        assert!(wizard_heard(&recorder, "persistence integrity breach"));
        assert_eq!(engine.action_state().revision(), 0);
    }

    /// core-action-8, the third ending: nobody answered for the installation
    /// at all. `into_parts` had already handed the module ops to the writer,
    /// so the store may hold a durable write whose typed RAM post-images went
    /// out with the dropped value. RAM and Redis have diverged and this task
    /// cannot reconcile them, so the loop's supervision check quarantines the
    /// world instead of admitting the next line and answering it out of RAM.
    #[test]
    fn an_installation_loss_quarantines_rather_than_answering_the_player() {
        let (mut engine, handle) = prepared_lane_fixture();
        engine.world.set_connected(GOD, true);
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let mut seen = engine.action_state().lost_installations();

        // A healthy store passes the check untouched — it is on the head of
        // every loop iteration, so it may not freeze anything by itself.
        assert!(!freeze_on_lost_installation(
            &engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &mut seen,
        ));
        assert!(!health.mode.integrity_frozen());
        assert!(!persistence_integrity.is_latched());

        // Stage the durable half, then lose the installation the way a dropped
        // `PendingCommit` does.
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &41, PreparedLaneState { amount: 7 })
            .expect("test state is valid");
        let (module_ops, installation) = transaction
            .prepare()
            .expect("test transaction prepares")
            .into_parts();
        assert_eq!(module_ops.len(), 1);
        drop(installation);

        assert!(freeze_on_lost_installation(
            &engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &mut seen,
        ));
        assert!(health.mode.integrity_frozen());
        assert!(persistence_integrity.is_latched());
        assert!(wizard_heard(&recorder, "persistence integrity breach"));
        // No success disposition: the post-image never went live, and the next
        // line cannot even open a transaction to make one.
        assert_eq!(engine.action_state().revision(), 0);
        assert!(
            engine
                .action_state()
                .get(&handle, &41)
                .expect("state lookup succeeds")
                .is_none()
        );
        assert!(matches!(
            engine.action_transaction(&handle).err(),
            Some(mush_core::ActionStateError::InstallationLost { reservation: 1 })
        ));
        // Frozen means frozen: the check does not re-alert on a loss it has
        // already answered for, and the freeze does not auto-lift.
        assert!(!freeze_on_lost_installation(
            &engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &mut seen,
        ));
        assert!(health.mode.integrity_frozen());
    }

    // ---- WS01-S5: the world loop's impossible states fail closed ----
    //
    // Every proof below drives the converted site directly, because that is
    // the only way to reach it: each was an `unreachable!` or an `expect`
    // precisely because no ordinary input produces the state. The failure mode
    // they all guard is the same one — the task that owns the world unwinds
    // while the store writer keeps its queue and the players keep their
    // connections — so "the call returned" is a real assertion.

    /// A commit receipt with no pending commit to release it against. Both
    /// halves of the receipt (ownership and final ack) used to `expect` here,
    /// on the single writer, over a store writer that would have gone on
    /// serving its queue. Now the world freezes, staff are told, and the loop
    /// is still there to be shut down.
    #[test]
    fn an_orphan_commit_receipt_freezes_the_world_instead_of_ending_the_writer() {
        for which in ["ownership", "commit"] {
            let (mut engine, _handle) = prepared_lane_fixture();
            engine.world.set_connected(GOD, true);
            let recorder = Arc::new(RecordingGateway::default());
            let gateway: Gateway = recorder.clone();
            let mut health = StoreHealth::new();
            let persistence_integrity = PersistenceIntegrityLatch::new();

            freeze_orphan_commit_receipt(
                &engine.world,
                &gateway,
                &mut health,
                &persistence_integrity,
                which,
            );

            // Reaching this line at all is the liveness half of the proof.
            assert!(health.mode.integrity_frozen(), "{which}: the world froze");
            assert!(persistence_integrity.is_latched(), "{which}: and latched");
            assert!(
                wizard_heard(&recorder, "persistence integrity breach"),
                "{which}: staff were told"
            );
            assert!(
                wizard_heard(
                    &recorder,
                    &format!("{which} receipt resolved with no pending commit"),
                ),
                "{which}: and told which receipt was orphaned"
            );
            // The freeze is a mutation barrier, not a stopped loop: shutdown is
            // still admitted, which is what makes the freeze observable and the
            // exit path reachable.
            assert_eq!(
                admission_for(health.mode, IngressClass::Command),
                Admission::Refuse(RefusalReason::IntegrityFrozen),
                "{which}: mutations are paused"
            );
            assert_eq!(
                admission_for(health.mode, IngressClass::Shutdown),
                Admission::Allow,
                "{which}: the loop is still there to shut down"
            );
        }
    }

    /// `refuse_world_msg` used to `unreachable!` on the two messages that are
    /// admitted in every world mode. If one is ever routed to it, refusing a
    /// shutdown only costs the operator a second attempt; ending the loop from
    /// inside the refusal path costs the game.
    #[test]
    fn refuse_world_msg_drops_messages_it_should_never_have_been_given() {
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();

        refuse_world_msg(
            WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)),
            RefusalReason::CommitPending,
            &gateway,
            None,
        );
        refuse_world_msg(
            WorldMsg::ReAttach {
                recs: Vec::new().into_boxed_slice(),
            },
            RefusalReason::CommitPending,
            &gateway,
            None,
        );

        // Both returned, and neither invented output for a nonexistent actor.
        assert!(
            recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .is_empty(),
            "a dropped control message speaks to nobody"
        );
    }

    /// The store-read-only admission is resolved by message body, and only
    /// `Inbound::Cmd` classifies as a command. A message that reaches the
    /// resolver anyway means the classifier drifted: it must refuse, not kill
    /// the loop that asked.
    #[test]
    fn a_misclassified_store_read_only_admission_refuses_instead_of_asserting() {
        let world = World::new_minimal();

        assert_eq!(
            resolve_store_read_only_command(
                &world,
                &WorldMsg::ReAttach {
                    recs: Vec::new().into_boxed_slice(),
                },
            ),
            Admission::Refuse(RefusalReason::StoreUnavailable),
            "a non-inbound message fails closed"
        );

        // The controls: the resolver still answers real command lines the way
        // the read-only allowlist says.
        assert_eq!(
            resolve_store_read_only_command(
                &world,
                &WorldMsg::Inbound(Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: "@dig somewhere".to_string(),
                }),
            ),
            Admission::Refuse(RefusalReason::StoreUnavailable),
            "a mutating line is still refused while the store is read-only"
        );
        assert_eq!(
            resolve_store_read_only_command(
                &world,
                &WorldMsg::Inbound(Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: "\"hello".to_string(),
                }),
            ),
            Admission::Allow,
            "speech still passes"
        );
    }

    /// The prepared lane is selected only when a staged action is in hand.
    /// If the selection and the engine disagree the flush refuses: nothing is
    /// persisted, nothing is released, and the caller is told so.
    #[tokio::test]
    async fn a_prepared_flush_with_nothing_staged_refuses_instead_of_asserting() {
        let (mut engine, _handle) = prepared_lane_fixture();
        assert!(
            !engine.has_staged_action(),
            "the bypass is calling the prepared lane with nothing staged"
        );

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        // The lane takes no serial to consume: a K3 commit is named by its own
        // token, and `flush_engine_step` keeps the legacy counter to itself.
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
        )
        .await;

        match outcome {
            FlushOutcome::Refused(detail) => assert!(
                detail.contains("no staged action"),
                "unexpected refusal: {detail}"
            ),
            _ => panic!("an empty prepared flush must refuse"),
        }
        assert!(
            matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "and hands the writer nothing"
        );
        assert!(health.last_write_unix.is_none());
    }

    /// The writer-gone arm of `flush_dirty` carries the `let ... else` that
    /// used to be an `unreachable!`. A closed writer queue is the reachable
    /// half of that path: it must report the batch as dropped rather than
    /// letting an ack-gated caller confirm a write that never happened.
    #[tokio::test]
    async fn flush_dirty_reports_writer_gone_when_the_queue_is_closed() {
        let (mut engine, _handle) = prepared_lane_fixture();
        engine
            .world
            .create(ObjKind::Thing, "orphan orb", GOD, Some(LIMBO))
            .expect("test object creation succeeds");
        assert!(engine.has_legacy_persistence_work());

        let (writer, writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        drop(writer_rx);
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let outcome = flush_dirty(
            &mut engine,
            &writer,
            &mut serial,
            true,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &crate::quarantine::QuarantineSpool::in_memory(),
            &persistence_integrity,
        )
        .await;

        assert!(
            matches!(outcome, FlushOutcome::WriterGone),
            "a closed writer queue is a dropped batch, never a durable one"
        );
        assert!(
            health.last_write_unix.is_none(),
            "and nothing may be recorded as written"
        );
    }

    /// Commands/maintenance must bind pending acknowledgements without an
    /// ArenaEgress wake; this also covers a queued run resumed after rollover.
    #[tokio::test]
    async fn generic_batches_stamp_acknowledgements_without_live_arena_traffic() {
        struct AckModule(tacsim::arena_bridge::ArenaTasks);
        impl GameModule for AckModule {
            fn name(&self) -> &'static str {
                "batch_ack_test"
            }
            fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
                let frames = reg.panel_feed("batch_ack.frames")?;
                let observers = reg.panel_feed("batch_ack.observers")?;
                reg.state(tacsim::arena_bridge::ArenaRegistry::new(
                    self.0.clone(),
                    frames,
                    observers,
                ));
                Ok(())
            }
        }
        let tasks = tacsim::arena_bridge::ArenaTasks::new(2);
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(AckModule(tasks.clone()))],
        )
        .unwrap();
        engine.world.take_dirty();
        let (writer, mut receiver) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut serial = 40;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(40);
        let integrity = PersistenceIntegrityLatch::new();
        let spool = crate::quarantine::QuarantineSpool::in_memory();
        assert!(tasks.live_ids().is_empty());
        for expected in [41, 42] {
            // The same unstamped table owns live-ledger and private-bulk latches.
            // Simulate a new command/tick write; no egress message is staged.
            tasks.note_ledger_write(tacsim::arena_task::MatchId(1), expected);
            tasks.ack_ledger_upto(expected - 1);
            assert_eq!(tasks.pending_ledger_acks(), 1);
            engine
                .world
                .create(ObjKind::Thing, "quota write", GOD, Some(LIMBO))
                .unwrap();
            let outcome = flush_dirty(
                &mut engine,
                &writer,
                &mut serial,
                true,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
                &spool,
                &integrity,
            )
            .await;
            assert!(matches!(outcome, FlushOutcome::Queued { .. }));
            let StoreJob::Write { batch, .. } = receiver.try_recv().unwrap() else {
                panic!("write expected");
            };
            assert_eq!(batch.serial, expected);
            tasks.ack_ledger_upto(expected - 1);
            assert_eq!(tasks.pending_ledger_acks(), 1, "queueing is not commitment");
            high_water.store(expected, Ordering::Relaxed);
            tasks.ack_ledger_upto(high_water.load(Ordering::Relaxed));
            assert_eq!(
                tasks.pending_ledger_acks(),
                0,
                "actual batch serial releases the acknowledgement without egress"
            );
        }
    }

    /// Interface plan I13: the exact receipt not only installs the typed
    /// post-images, it announces them — the fixture's observer hears one
    /// `ActionStateChanged` for the touched family and its line reaches the
    /// gateway with the commit's own release.
    #[tokio::test]
    async fn committed_prepared_action_announces_the_touched_family_to_observers() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &41, PreparedLaneState { amount: 7 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let bundle = prepared_lane_bundle(&prepared, 13);
        let expected_token = bundle.token();
        let guards = empty_guards(&engine);
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .expect("clean engine accepts prepared action");

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
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
        settle_queued_authoritative_job(writer_rx.recv().await.expect("prepared write was queued"))
            .await;
        let mut receipt = receipt;
        let ack = receipt.receive().await;
        let mut pending = pending_with_prepared_installation(0, installation);
        pending.token = CommitToken::from_authoritative(expected_token);
        let recorder = Arc::new(RecordingGateway {
            discord: Mutex::default(),
            sends: Mutex::new(Vec::new()),
            batches: Mutex::default(),
        });
        let gateway: Gateway = recorder.clone();
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
            &test_effect_store(),
        );
        assert_eq!(completed.outcome, DeliveryOutcome::Committed);
        assert_eq!(
            engine
                .action_state()
                .get(&handle, &41)
                .expect("state lookup succeeds"),
            Some(&PreparedLaneState { amount: 7 })
        );
        let announced: Vec<String> = recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .filter_map(|(target, event)| match (target, event.as_ref()) {
                (SendTarget::Char(1), OutputEvent::Sys(text)) => Some(text.to_plain_text()),
                _ => None,
            })
            .collect();
        assert_eq!(
            announced,
            vec!["announced prepared_lane_test/receipt_state [\"41\"]".to_string()],
            "one announcement per touched family, released with the commit"
        );
    }

    /// The other half of the seam: a refused receipt installs nothing and
    /// announces nothing — observers never hear about images that never went
    /// live.
    #[test]
    fn refused_prepared_action_announces_nothing() {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &43, PreparedLaneState { amount: 2 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let (_, installation) = prepared.into_parts();
        let pending = pending_with_prepared_installation(7, installation.into());
        let recorder = Arc::new(RecordingGateway {
            discord: Mutex::default(),
            sends: Mutex::new(Vec::new()),
            batches: Mutex::default(),
        });
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);
        let completed = complete_pending_commit(
            pending,
            Err(AuthoritativeCommitFailure::Refused(
                "test refusal".to_owned(),
            )),
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );
        assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
        assert!(
            engine
                .action_state()
                .get(&handle, &43)
                .expect("state lookup succeeds")
                .is_none(),
            "a refused receipt installs nothing"
        );
        let heard = recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .any(|(_, event)| {
                matches!(event.as_ref(), OutputEvent::Sys(text) if text.to_plain_text().starts_with("announced "))
            });
        assert!(
            !heard,
            "nothing is announced for images that never went live"
        );
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

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let outcome = flush_engine_step(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &crate::quarantine::QuarantineSpool::in_memory(),
            &persistence_integrity,
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
            ownership,
            notify,
        } = writer_rx.recv().await.expect("exact bundle queued")
        else {
            panic!("K3 staging must not use legacy WriteBatch");
        };
        assert_eq!(queued.as_ref(), &bundle);

        let store = InMemoryStore::new();
        let intent = store
            .reserve_authoritative_intent((*queued).clone())
            .await
            .unwrap();
        ownership.send(Ok(intent)).unwrap();
        let exact = store
            .reconcile_authoritative(expected_token, expected_hash)
            .await
            .unwrap();
        notify.send(Ok(exact)).unwrap();
        let owned = pending_receipt.receive_ownership().await.unwrap();
        assert_eq!(owned.token, expected_token);
        assert_eq!(owned.bundle_hash, expected_hash);
        let ack = pending_receipt.receive().await;

        let mut pending = pending_with_prepared_installation(0, installation);
        pending.token = CommitToken::from_authoritative(expected_token);
        let gateway: Gateway = Arc::new(BlackholeGateway);
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
            &test_effect_store(),
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
            &test_effect_store(),
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
        engine
            .abort_prepared_action(first_installation)
            .expect("the fixture aborts");

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
        let bundle = prepared_lane_bundle(&prepared, 23);
        let guards = prepared_lane_guards(&engine, 1);
        engine
            .stage_authoritative_action(prepared, guards, bundle)
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

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let outcome = flush_engine_step(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &crate::quarantine::QuarantineSpool::in_memory(),
            &persistence_integrity,
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
                &test_effect_store(),
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
        let bundle = prepared_lane_bundle(&prepared, 37);
        let guards = empty_guards(&engine);
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .expect("clean engine accepts prepared action");

        let (writer, writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        drop(writer_rx);
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let outcome = flush_prepared_action(
            &mut engine,
            &writer,
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

    /// With the K2 shim gone, the recheck at the head of `flush_prepared_action`
    /// is the only remaining mixed-lane refusal, and it is permanent: staging
    /// proves the step was clean, this proves it *stayed* clean. Both ways a
    /// step can pick up legacy work after staging — a dirtied world and a
    /// queued module op — must reject at submission, consume no commit serial,
    /// and hand the writer nothing.
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
            let bundle = prepared_lane_bundle(&prepared, 41);
            let guards = empty_guards(&engine);
            engine
                .stage_authoritative_action(prepared, guards, bundle)
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

            let (writer, mut writer_rx) =
                persist::test_store_writer_channel(Arc::new(Metrics::new()));
            let mut serial = 0;
            let mut health = StoreHealth::new();
            let metrics = Metrics::new();
            let mut retained = RetainedSeals::new();
            let high_water = AtomicU64::new(0);
            let persistence_integrity = PersistenceIntegrityLatch::new();
            let outcome = flush_engine_step(
                &mut engine,
                &writer,
                &mut serial,
                &mut health,
                &metrics,
                &mut retained,
                &high_water,
                &crate::quarantine::QuarantineSpool::in_memory(),
                &persistence_integrity,
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
            &test_effect_store(),
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
        let handle = WorldHandle {
            tx,
            shutdown: None,
            direct_actions: None,
        };
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

    // ---- 16b cut B.5 (`B-13`): the defer queue --------------------------------
    //
    // A line that meets another action's commit used to be refused at once. It
    // now waits in a bounded queue (four a connection, 64 in all, 250 ms from its
    // arrival) and is served when the receipt lands, or refused in a player's
    // words if the commit outlives the bound. The pure queue is tested first;
    // then the loop, on the held-receipt rig: the store writer is a test channel,
    // so a commit stays pending exactly as long as the test holds its receipt.

    fn defer_line(conn: u64, line: &str) -> WorldMsg {
        WorldMsg::Inbound(Inbound::Cmd {
            conn,
            actor: GOD,
            line: line.to_owned(),
        })
    }

    fn defer_read() -> WorldMsg {
        WorldMsg::NameFree {
            name: "probe".to_owned(),
            reply: oneshot::channel().0,
        }
    }

    /// The typed line a queue entry holds, if it is one.
    fn deferred_line(deferred: &super::Deferred) -> Option<&str> {
        match &deferred.entry {
            DeferEntry::Msg(WorldMsg::Inbound(Inbound::Cmd { line, .. })) => Some(line.as_str()),
            _ => None,
        }
    }

    #[test]
    fn the_defer_queue_bounds_typed_lines_per_connection() {
        let now = DeferInstant::now();
        let mut queue = DeferQueue::default();
        for n in 0..COMMAND_DEFER_DEPTH {
            assert!(
                queue
                    .push_msg(defer_line(1, &format!("line {n}")), now)
                    .is_ok(),
                "line {n} of connection 1 waits"
            );
        }
        let refused = queue
            .push_msg(defer_line(1, "one too many"), now)
            .expect_err("a fifth typed line from one connection is handed back");
        assert!(matches!(
            refused,
            WorldMsg::Inbound(Inbound::Cmd { ref line, .. }) if line == "one too many"
        ));
        // Another connection's first line is its own.
        assert!(queue.push_msg(defer_line(2, "another player"), now).is_ok());
        // A read or a query counts against the cap only, never a connection.
        assert!(queue.push_msg(defer_read(), now).is_ok());
        assert!(
            queue
                .push_msg(
                    WorldMsg::Inbound(Inbound::Query {
                        conn: 1,
                        actor: GOD,
                        id: 7,
                        name: "who".into(),
                        args: serde_json::json!({}),
                    }),
                    now,
                )
                .is_ok(),
            "a query from a connection with four lines waiting still waits"
        );
        assert_eq!(queue.entries.len(), COMMAND_DEFER_DEPTH + 3);
    }

    #[test]
    fn the_defer_queue_is_capped_and_a_barrier_at_the_cap_is_handed_back() {
        let now = DeferInstant::now();
        let mut queue = DeferQueue::default();
        for _ in 0..COMMAND_DEFER_CAP {
            assert!(queue.push_msg(defer_read(), now).is_ok());
        }
        assert!(
            queue.push_msg(defer_read(), now).is_err(),
            "the 65th entry is handed back"
        );
        assert!(
            queue.push_msg(defer_line(9, "look"), now).is_err(),
            "and so is a connection's first line once the cap is reached"
        );
        // A barrier at the cap is handed back unanswered (the loop refuses it
        // `Busy`): answering it `Ok` would overtake the 64 entries ahead of it
        // and the commit they wait on.
        let (reply, mut answered) = oneshot::channel();
        let handed_back = queue
            .push_barrier(reply, now)
            .expect_err("a barrier at the cap is handed back");
        assert!(
            matches!(
                answered.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "the queue never answers a barrier itself"
        );
        drop(handed_back);
        assert_eq!(queue.entries.len(), COMMAND_DEFER_CAP);
        assert_eq!(queue.barriers(), 0);
        // One place free and the barrier waits like any other entry.
        assert!(queue.pop_front().is_some());
        let (reply, _answered) = oneshot::channel();
        assert!(queue.push_barrier(reply, now).is_ok());
        assert_eq!(queue.barriers(), 1);
    }

    #[test]
    fn the_defer_queue_expires_only_the_aged_in_arrival_order_and_never_a_barrier() {
        let start = DeferInstant::now();
        let later = start + Duration::from_millis(100);
        let mut queue = DeferQueue::default();
        assert_eq!(
            queue.oldest_deadline(),
            None,
            "an empty queue has no deadline"
        );
        let (early_barrier, mut early_barrier_rx) = oneshot::channel();
        assert!(queue.push_barrier(early_barrier, start).is_ok());
        assert_eq!(
            queue.oldest_deadline(),
            None,
            "a barrier alone never arms the timer"
        );
        assert!(queue.push_msg(defer_line(1, "first"), start).is_ok());
        assert!(queue.push_msg(defer_line(2, "second"), start).is_ok());
        let (late_barrier, _late_barrier_rx) = oneshot::channel();
        assert!(queue.push_barrier(late_barrier, later).is_ok());
        assert!(queue.push_msg(defer_line(1, "third"), later).is_ok());
        assert_eq!(
            queue.oldest_deadline(),
            Some(start + COMMAND_DEFER),
            "the earliest message's arrival plus the bound"
        );

        // A millisecond short of the bound nothing goes.
        assert!(
            queue
                .expire(start + COMMAND_DEFER - Duration::from_millis(1))
                .is_empty()
        );
        // At the bound the two early lines go, front to back; the late one and
        // both barriers stay, in their places.
        let expired = queue.expire(start + COMMAND_DEFER);
        let lines: Vec<&str> = expired
            .iter()
            .filter_map(|msg| match msg {
                WorldMsg::Inbound(Inbound::Cmd { line, .. }) => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, ["first", "second"]);
        assert_eq!(queue.entries.len(), 3);
        assert!(matches!(queue.entries[0].entry, DeferEntry::Barrier(_)));
        assert!(matches!(queue.entries[1].entry, DeferEntry::Barrier(_)));
        assert_eq!(deferred_line(&queue.entries[2]), Some("third"));
        assert_eq!(queue.barriers(), 2);
        assert_eq!(queue.oldest_deadline(), Some(later + COMMAND_DEFER));
        // Long after, the last line goes and the barriers still stay.
        assert_eq!(queue.expire(later + Duration::from_secs(60)).len(), 1);
        assert_eq!(queue.barriers(), 2);
        assert_eq!(queue.oldest_deadline(), None);
        assert!(
            matches!(
                early_barrier_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "an expiry never answers a barrier"
        );
    }

    #[test]
    fn the_defer_queue_pops_in_arrival_order_and_a_kept_entry_keeps_its_place() {
        let start = DeferInstant::now();
        let mut queue = DeferQueue::default();
        for (n, line) in ["a", "b", "c"].into_iter().enumerate() {
            assert!(
                queue
                    .push_msg(
                        defer_line(1, line),
                        start + Duration::from_millis(u64::try_from(n).expect("small")),
                    )
                    .is_ok()
            );
        }
        let first = queue.pop_front().expect("a");
        assert_eq!(deferred_line(&first), Some("a"));
        assert_eq!(first.since, start);
        // A drained line that met a commit again goes back to the FRONT with
        // its first deadline, not to the back with a new one.
        queue.push_front_kept(first);
        assert_eq!(queue.oldest_deadline(), Some(start + COMMAND_DEFER));
        let order: Vec<String> = queue
            .drain_all()
            .iter()
            .filter_map(|deferred| deferred_line(deferred).map(str::to_owned))
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert!(queue.is_empty());
        assert!(queue.pop_front().is_none());
    }

    #[test]
    fn what_waits_in_the_defer_queue_is_retryable_input_that_holds_no_custody() {
        let (reply, _) = oneshot::channel();
        for msg in [
            defer_line(1, "look"),
            WorldMsg::Inbound(Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 1,
                name: "who".into(),
                args: serde_json::json!({}),
            }),
            defer_read(),
            WorldMsg::UnlinkPlayer {
                character: GOD,
                reply: oneshot::channel().0,
            },
            WorldMsg::InInputContext {
                actor: GOD,
                reply: oneshot::channel().0,
            },
            WorldMsg::ConnectionReady {
                conn: 1,
                actor: GOD,
                reply: oneshot::channel().0,
            },
            WorldMsg::LegalActions {
                conn: 1,
                actor: GOD,
                subject: PerceivedRef::SelfActor,
                reply: oneshot::channel().0,
            },
        ] {
            assert!(waits_in_defer_queue(&msg));
            assert!(!must_defer_behind_commit(&msg));
        }
        for msg in [
            WorldMsg::Inbound(Inbound::Composing {
                conn: 1,
                actor: GOD,
                active: true,
            }),
            WorldMsg::CreatePlayer {
                name: "Nope".into(),
                account: None,
                reply: oneshot::channel().0,
            },
            WorldMsg::Inbound(Inbound::Control(ControlMsg::Dump)),
            WorldMsg::Barrier { reply },
        ] {
            assert!(!waits_in_defer_queue(&msg));
        }
        assert_eq!(
            RefusalReason::CommitTimedOut.text(),
            "The world was busy too long for that line; please retry in a moment."
        );
        assert!(
            RefusalReason::CommitTimedOut
                .text()
                .ends_with("please retry in a moment."),
            "the phrase every rig's commit-pending retry needle matches"
        );
    }

    /// A timed-out entry that is not a typed line hears exactly what
    /// `CommitPending` gives it: a read's typed `Busy`, a query its `Reply`
    /// wrapping the `CommitPending` sentence. Only a line gets the new words.
    #[test]
    fn a_timed_out_read_or_query_is_refused_as_commit_pending() {
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();

        let (reply, mut legal) = oneshot::channel();
        refuse_world_msg(
            WorldMsg::LegalActions {
                conn: 1,
                actor: GOD,
                subject: PerceivedRef::SelfActor,
                reply,
            },
            RefusalReason::CommitTimedOut,
            &gateway,
            None,
        );
        assert_eq!(legal.try_recv(), Ok(Err(LegalActionReadError::Busy)));

        let (reply, mut ready) = oneshot::channel();
        refuse_world_msg(
            WorldMsg::ConnectionReady {
                conn: 1,
                actor: GOD,
                reply,
            },
            RefusalReason::CommitTimedOut,
            &gateway,
            None,
        );
        assert_eq!(ready.try_recv(), Ok(Err(WorldReadError::Busy)));

        refuse_world_msg(
            WorldMsg::Inbound(Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 9,
                name: "who".into(),
                args: serde_json::json!({}),
            }),
            RefusalReason::CommitTimedOut,
            &gateway,
            None,
        );
        assert_eq!(
            recorder.reply_text(9).as_deref(),
            Some(RefusalReason::CommitPending.text())
        );

        refuse_world_msg(
            defer_line(1, "look"),
            RefusalReason::CommitTimedOut,
            &gateway,
            None,
        );
        let lines: Vec<String> = recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .filter_map(|(_, event)| match event.as_ref() {
                OutputEvent::Sys(text) => Some(text.to_plain_text()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, [RefusalReason::CommitTimedOut.text()]);
    }

    /// The world task on a store writer the test answers by hand.
    struct HeldReceiptRig {
        handle: WorldHandle,
        writer_rx: persist::StoreWriterTestReceiver,
        metrics: Arc<Metrics>,
        recorder: Arc<RecordingGateway>,
        world_join: tokio::task::JoinHandle<WorldOutcome>,
    }

    type HeldReceipt = oneshot::Sender<Result<u64, String>>;

    ///
    /// A freshly booted minimal world is dirty (Limbo and God were never
    /// written), and whichever wake runs first flushes that as a write of its
    /// own. The rig settles it before handing over — two barriers, answering
    /// every write meanwhile — so the test's first command is the first commit
    /// and meets no predecessor.
    async fn held_receipt_rig() -> HeldReceiptRig {
        let engine = boot_engine();
        let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = WorldHandle {
            tx: tx.clone(),
            shutdown: Some(shutdown_tx),
            direct_actions: None,
        };
        let (staff_tx, _staff_rx) = mpsc::channel(STAFF_OPS_CAPACITY);
        let (_signal_tx, signal_rx) = writer_signal_channel();
        let metrics = Arc::new(Metrics::new());
        let (writer, writer_rx) = persist::test_store_writer_channel(Arc::clone(&metrics));
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
            1,
            PersistenceIntegrityLatch::new(),
            WorldExitStatus::new(),
            None,
            None,
            watch::channel(false).0,
            // 16b cut T.2: this rig composes no game layer, so there is no arena
            // table and the egress arm stays disarmed.
            None,
        ));
        let mut rig = HeldReceiptRig {
            handle,
            writer_rx,
            metrics,
            recorder,
            world_join,
        };
        rig.settle().await;
        rig.settle().await;
        rig
    }

    impl HeldReceiptRig {
        /// Run a barrier to completion, answering every write meanwhile, so
        /// nothing the world had to flush is left pending for the proof.
        async fn settle(&mut self) {
            let barrier = tokio::spawn({
                let handle = self.handle.clone();
                async move { handle.barrier().await }
            });
            tokio::time::timeout(Duration::from_secs(2), async {
                while !barrier.is_finished() {
                    if let Ok(StoreJob::Write {
                        batch,
                        notify: Some(receipt),
                    }) = self.writer_rx.try_recv()
                    {
                        let _ = receipt.send(Ok(batch.serial));
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the pending write settles");
            assert!(
                barrier.await.expect("barrier task"),
                "the pending write lands"
            );
        }

        /// Attach `conn` as God, settled, so reads that ask about the
        /// connection have one to find.
        async fn attach(&mut self, conn: u64) {
            assert!(
                self.handle
                    .inbound(Inbound::ConnUp {
                        conn,
                        actor: GOD,
                        caps_structured: true,
                        from_linkdead: false,
                    })
                    .await
            );
            self.settle().await;
        }

        /// The guard a served-path proof takes before it sends the receipt:
        /// the 250 ms bound is wall-clock, so a starved runner can age a queued
        /// entry out before the test answers, and that must name itself rather
        /// than read as a queue defect.
        fn nothing_timed_out_yet(&self) {
            assert_eq!(
                self.metrics.snapshot().command_defer_timed_out_total,
                0,
                "the runner was too slow: a queued entry outlived its 250 ms \
                 bound before the receipt was sent, so this proof cannot run"
            );
        }
        /// Type one line as God on `conn`.
        async fn line(&self, conn: u64, line: &str) {
            assert!(
                self.handle
                    .inbound(Inbound::Cmd {
                        conn,
                        actor: GOD,
                        line: line.to_owned(),
                    })
                    .await
            );
        }

        /// The next exact-receipt write, left unanswered.
        async fn next_write(&mut self) -> (u64, HeldReceipt) {
            match tokio::time::timeout(Duration::from_secs(2), self.writer_rx.recv())
                .await
                .expect("a dirty step must enqueue a write")
                .expect("writer channel open")
            {
                StoreJob::Write {
                    batch,
                    notify: Some(receipt),
                } => (batch.serial, receipt),
                _ => panic!("expected an exact-receipt write"),
            }
        }

        /// Every line of text the gateway was handed, in order.
        fn heard(&self) -> Vec<String> {
            self.recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .filter_map(|(_, event)| match event.as_ref() {
                    OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                        Some(text.to_plain_text())
                    }
                    _ => None,
                })
                .collect()
        }

        fn heard_any(&self, needle: &str) -> bool {
            self.heard().iter().any(|line| line.contains(needle))
        }

        /// Wait (bounded) until `done` holds, answering every exact-receipt
        /// write the world makes meanwhile, so a line that turns out to commit
        /// cannot wedge the proof.
        async fn serve_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !done(self) {
                    if let Ok(StoreJob::Write {
                        batch,
                        notify: Some(receipt),
                    }) = self.writer_rx.try_recv()
                    {
                        let _ = receipt.send(Ok(batch.serial));
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}; heard {:?}", self.heard()));
        }

        async fn shut_down(self) {
            assert!(
                self.handle
                    .inbound(Inbound::Control(ControlMsg::Shutdown))
                    .await
            );
            tokio::time::timeout(Duration::from_secs(2), self.world_join)
                .await
                .expect("world shutdown timed out")
                .expect("world task panicked");
        }
    }

    /// **16b cut B.5 (`B-13`), the served case.** Four lines typed behind a
    /// held commit wait; a fifth from the same connection is refused at once
    /// in the old words; the receipt lands and the four are answered, in the
    /// order they were typed, each counted once.
    #[tokio::test]
    async fn a_line_behind_a_commit_is_served_when_the_receipt_lands() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        for n in 1..=4 {
            rig.line(1, &format!("think typed-{n}")).await;
        }
        rig.line(1, "think one-too-many").await;
        rig.serve_until("the fifth line's refusal", |rig| {
            rig.heard_any("committing another action")
        })
        .await;
        rig.nothing_timed_out_yet();
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");

        rig.serve_until("the four queued lines", |rig| {
            (1..=4).all(|n| rig.heard_any(&format!("typed-{n}")))
        })
        .await;
        let heard = rig.heard();
        let at = |needle: &str| {
            heard
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} not heard: {heard:?}"))
        };
        assert!(
            at("Created: receipt-anchor") < at("typed-1")
                && at("typed-1") < at("typed-2")
                && at("typed-2") < at("typed-3")
                && at("typed-3") < at("typed-4"),
            "the anchor's answer, then the four lines in the order typed: {heard:?}"
        );
        assert!(
            !rig.heard_any("one-too-many"),
            "the refused fifth line never ran"
        );
        assert!(
            !rig.heard_any("busy too long"),
            "nothing outlived the bound: {heard:?}"
        );
        let snapshot = rig.metrics.snapshot();
        assert_eq!(snapshot.command_deferred_total, 4);
        assert_eq!(snapshot.command_defer_timed_out_total, 0);
        assert_eq!(
            snapshot.commands_total, 6,
            "the anchor, the four served lines and the refused fifth, once each"
        );
        rig.shut_down().await;
    }

    // ---- 16b cut AP.4 (16c `N20`): OUTPUTPREFIX/OUTPUTSUFFIX at the boundary ----
    //
    // The engine pushes the prefix `BestEffort` first and the suffix as a
    // `Trailer` last; these prove what the world task's two partitions and its
    // receipt release make of them, and that its own pre-engine refusals carry
    // the same row.

    /// Every `(target, text)` the recorder was handed, `Sys` and narrative.
    fn ap4_heard(recorder: &RecordingGateway) -> Vec<(SendTarget, String)> {
        recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .filter_map(|(target, event)| match event.as_ref() {
                OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                    Some((target.clone(), text.to_plain_text()))
                }
                _ => None,
            })
            .collect()
    }

    /// **The committing order, end to end on the held-receipt rig.** A typed
    /// line whose step queues a receipt transmits `>>>` before the ack (with
    /// the step's `BestEffort` sends) and nothing else; the ack then releases
    /// the line's `OnCommit` answer and `<<<` after it.
    #[tokio::test]
    async fn ap4_a_committing_line_reads_its_prefix_before_the_ack_and_its_suffix_after_it() {
        let mut rig = held_receipt_rig().await;
        rig.attach(1).await;
        rig.line(1, "OUTPUTPREFIX >>>").await;
        rig.line(1, "OUTPUTSUFFIX <<<").await;
        rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
            .await;
        let before = ap4_heard(&rig.recorder).len();

        rig.line(1, "@create bracket-anchor").await;
        let (serial, receipt) = rig.next_write().await;
        rig.serve_until("the prefix, before the ack", |rig| {
            ap4_heard(&rig.recorder)[before..]
                .iter()
                .any(|(_, text)| text == ">>>")
        })
        .await;
        let held = ap4_heard(&rig.recorder)[before..].to_vec();
        assert_eq!(
            held,
            vec![(SendTarget::Conn(1), ">>>".to_owned())],
            "before the ack only the prefix crosses: {held:?}"
        );
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        rig.serve_until("the suffix, after the ack", |rig| {
            ap4_heard(&rig.recorder)[before..]
                .iter()
                .any(|(_, text)| text == "<<<")
        })
        .await;
        let heard = ap4_heard(&rig.recorder)[before..].to_vec();
        assert_eq!(
            heard.first(),
            Some(&(SendTarget::Conn(1), ">>>".to_owned()))
        );
        assert_eq!(heard.last(), Some(&(SendTarget::Conn(1), "<<<".to_owned())));
        assert!(
            heard[1..heard.len() - 1]
                .iter()
                .any(|(_, text)| text.contains("Created: bracket-anchor")),
            "the line's committed answer sits inside the pair: {heard:?}"
        );
        rig.shut_down().await;
    }

    /// **The aborted order, end to end.** The same committing line, its
    /// receipt failed: `>>>` crossed before the ack, the `OnCommit` answer is
    /// suppressed, the commit-failure sentence still reaches the player (the
    /// prefix is framing, not an answer that would silence it) and sits INSIDE
    /// the pair, and `<<<` closes it.
    #[tokio::test]
    async fn ap4_an_aborted_line_reads_its_refusal_inside_the_pair_and_none_of_its_success() {
        let mut rig = held_receipt_rig().await;
        rig.attach(1).await;
        rig.line(1, "OUTPUTPREFIX >>>").await;
        rig.line(1, "OUTPUTSUFFIX <<<").await;
        rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
            .await;
        let before = ap4_heard(&rig.recorder).len();

        rig.line(1, "@create bracket-anchor").await;
        let (_serial, receipt) = rig.next_write().await;
        receipt
            .send(Err("ap4 test failure".to_owned()))
            .expect("world still owns the exact receipt");
        rig.serve_until("the suffix, after the failed ack", |rig| {
            ap4_heard(&rig.recorder)[before..]
                .iter()
                .any(|(_, text)| text == "<<<")
        })
        .await;
        let heard = ap4_heard(&rig.recorder)[before..].to_vec();
        assert_eq!(
            heard.first(),
            Some(&(SendTarget::Conn(1), ">>>".to_owned()))
        );
        assert_eq!(heard.last(), Some(&(SendTarget::Conn(1), "<<<".to_owned())));
        assert!(
            heard
                .iter()
                .all(|(_, text)| !text.contains("Created: bracket-anchor")),
            "nothing of the OnCommit answer: {heard:?}"
        );
        assert!(
            heard[1..heard.len() - 1]
                .iter()
                .any(|(_, text)| text.contains("database rejected your last action")),
            "the failure sentence is said, inside the pair: {heard:?}"
        );
        rig.shut_down().await;
    }

    /// **The aborted order on the receipt release itself.** What a committing
    /// step holds for the ack (its `OnCommit` answer, an `OnAbort` refusal and
    /// its `Trailer`; the `BestEffort` prefix already crossed) releases on an
    /// abort as the refusal then `<<<`, nothing of the success, and on a
    /// commit as the success then `<<<`, nothing of the refusal. What another
    /// step joined to the batch while the receipt was awaited stays AFTER the
    /// suffix on a commit (nothing moves inside the pair) and is dropped on an
    /// abort.
    #[test]
    fn ap4_the_held_trailer_closes_the_step_on_either_ack() {
        let held = || -> Vec<ClassifiedSend> {
            vec![
                (
                    DeliveryClass::OnCommit,
                    SendTarget::Char(u64::from(GOD.0)),
                    Arc::new(OutputEvent::Sys(RichText::plain("Set."))),
                ),
                (
                    DeliveryClass::OnAbort,
                    SendTarget::Char(u64::from(GOD.0)),
                    Arc::new(OutputEvent::Sys(RichText::tag("error", "refused"))),
                ),
                (
                    DeliveryClass::Trailer,
                    SendTarget::Conn(1),
                    Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_SUFFIX_TAG, "<<<"))),
                ),
            ]
        };
        for (ack, expected) in [
            (Ok(()), vec!["Set.", "<<<", "joined"]),
            (
                Err(AuthoritativeCommitFailure::Refused("stale".to_owned())),
                vec!["refused", "<<<"],
            ),
        ] {
            let mut pending = pending_for_custody_test();
            pending.sends = held();
            // A tick's commit-bound line, joined behind the suffix while the
            // receipt was awaited (`pending.sends.extend` in `run`).
            pending.sends.push((
                DeliveryClass::OnCommit,
                SendTarget::Char(u64::from(GOD.0)),
                Arc::new(OutputEvent::Sys(RichText::plain("joined"))),
            ));
            pending.effects.clear();
            let mut engine = Engine::new(World::new_minimal(), base_registry());
            let recorder = Arc::new(RecordingGateway::default());
            let gateway: Gateway = recorder.clone();
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
                &test_effect_store(),
            );
            finish_pending_operational(&mut engine, &gateway, completed);
            let heard: Vec<String> = ap4_heard(&recorder)
                .into_iter()
                .map(|(_, text)| text)
                .collect();
            assert_eq!(heard, expected);
        }

        // Refused before submission (a stale precondition): nothing crossed
        // early, so the step's whole batch is here — the prefix, the success,
        // the suffix. The prefix is framing, not an answer: the generic
        // "not committed" sentence is still said, and inside the pair.
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let (tx, _rx) = mpsc::channel(1);
        let mut step = vec![(
            DeliveryClass::BestEffort,
            SendTarget::Conn(1),
            Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_PREFIX_TAG, ">>>"))),
        )];
        step.extend(
            held()
                .into_iter()
                .filter(|(class, _, _)| *class != DeliveryClass::OnAbort),
        );
        refuse_before_submission(
            step,
            Vec::new(),
            Some(GOD),
            None,
            "stale guard",
            &gateway,
            &StoreHealth::new(),
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );
        let heard: Vec<String> = ap4_heard(&recorder)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(
            heard,
            vec![
                ">>>".to_owned(),
                "Your action was not committed (stale guard). Please retry against the current state."
                    .to_owned(),
                "<<<".to_owned(),
            ]
        );

        // Rejected by the store with no receipt (`abort_without_receipt`, the
        // Rejected / WriterGone / no-receipt arms): the whole batch is here
        // too. The prefix is not an answer, so the integrity sentence is still
        // said, inside the pair; the suffix still closes it; the success is
        // suppressed.
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let (tx, _rx) = mpsc::channel(1);
        let mut health = StoreHealth::new();
        let mut step = vec![(
            DeliveryClass::BestEffort,
            SendTarget::Conn(1),
            Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_PREFIX_TAG, ">>>"))),
        )];
        step.extend(
            held()
                .into_iter()
                .filter(|(class, _, _)| *class != DeliveryClass::OnAbort),
        );
        abort_without_receipt(
            step,
            Vec::new(),
            Some(GOD),
            None,
            "rejected batch",
            &World::new_minimal(),
            &gateway,
            &mut health,
            &PersistenceIntegrityLatch::new(),
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );
        let heard: Vec<String> = ap4_heard(&recorder)
            .into_iter()
            .map(|(_, text)| text)
            .filter(|text| !text.starts_with("GAME: persistence integrity breach"))
            .collect();
        assert_eq!(
            heard,
            vec![
                ">>>".to_owned(),
                "The database rejected your last action (rejected batch). Its success output \
                 and external effects were suppressed; the world is now integrity-frozen."
                    .to_owned(),
                "<<<".to_owned(),
            ]
        );
    }

    /// **A line held behind a save, deterministically.** The receipt of a
    /// bracketed connection's committing line is held; a second line typed
    /// meanwhile waits in 16b cut B.5's defer queue, so only the first line's
    /// `>>>` has crossed. The ack releases the first line's answer and `<<<`,
    /// then the served line's own `>>>`, its answer and `<<<` — two pairs in the
    /// order typed, neither inside the other.
    #[tokio::test]
    async fn ap4_a_line_held_behind_a_save_reads_its_own_pair_after_the_saving_one() {
        let mut rig = held_receipt_rig().await;
        rig.attach(1).await;
        rig.line(1, "OUTPUTPREFIX >>>").await;
        rig.line(1, "OUTPUTSUFFIX <<<").await;
        rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
            .await;
        let before = ap4_heard(&rig.recorder).len();

        rig.line(1, "@create held-anchor").await;
        let (serial, receipt) = rig.next_write().await;
        rig.line(1, "think held-line").await;
        rig.serve_until("the think, deferred behind the receipt", |rig| {
            rig.metrics.snapshot().command_deferred_total == 1
        })
        .await;
        let held = ap4_heard(&rig.recorder)[before..].to_vec();
        assert_eq!(
            held,
            vec![(SendTarget::Conn(1), ">>>".to_owned())],
            "before the ack only the saving line's prefix crosses: {held:?}"
        );
        rig.nothing_timed_out_yet();
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        rig.serve_until("both suffixes", |rig| {
            ap4_heard(&rig.recorder)[before..]
                .iter()
                .filter(|(_, text)| text == "<<<")
                .count()
                == 2
        })
        .await;
        let heard: Vec<String> = ap4_heard(&rig.recorder)[before..]
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        let marks: Vec<usize> = heard
            .iter()
            .enumerate()
            .filter(|(_, text)| *text == ">>>" || *text == "<<<")
            .map(|(at, _)| at)
            .collect();
        assert_eq!(
            marks
                .iter()
                .map(|at| heard[*at].as_str())
                .collect::<Vec<_>>(),
            vec![">>>", "<<<", ">>>", "<<<"],
            "two pairs, neither inside the other: {heard:?}"
        );
        assert_eq!(marks[0], 0, "the saving line's prefix is first: {heard:?}");
        assert_eq!(
            marks[3],
            heard.len() - 1,
            "the served line's suffix is last: {heard:?}"
        );
        let first = &heard[marks[0] + 1..marks[1]];
        let second = &heard[marks[2] + 1..marks[3]];
        assert!(
            first
                .iter()
                .any(|text| text.contains("Created: held-anchor")),
            "the saving line's answer is in its own pair: {heard:?}"
        );
        assert!(
            second.iter().any(|text| text.contains("held-line"))
                && second.iter().all(|text| !text.contains("Created:")),
            "the served line's answer, and only that, is in its own pair: {heard:?}"
        );
        assert_eq!(rig.metrics.snapshot().command_defer_timed_out_total, 0);
        rig.shut_down().await;
    }

    /// **The world task's own refusal.** `refuse_world_msg` on a typed line
    /// from a bracketed connection transmits the prefix, the refusal (worded
    /// and tagged as `send_sys` words it) and the suffix to that connection in
    /// ONE batch; on a connection with no row, the actor-addressed refusal
    /// alone, as before.
    #[test]
    fn ap4_a_world_task_refusal_is_bracketed_from_the_lines_own_row() {
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut world = World::new_minimal();
        world.set_output_prefix(1, Some(">>>".to_owned()));
        world.set_output_suffix(1, Some("<<<".to_owned()));

        let msg = defer_line(1, "look");
        let brackets = output_brackets_of(&world, &msg);
        refuse_world_msg(msg, RefusalReason::CommitPending, &gateway, brackets);
        // ONE transmit: nothing the world sends can land between the three.
        let batches =
            std::mem::take(&mut *recorder.batches.lock().expect("recording gateway poisoned"));
        assert_eq!(batches, vec![3], "one batch of three");
        let sends =
            std::mem::take(&mut *recorder.sends.lock().expect("recording gateway poisoned"));
        let shaped: Vec<(SendTarget, Option<String>, String)> = sends
            .iter()
            .map(|(target, event)| match event.as_ref() {
                OutputEvent::Sys(text) => (
                    target.clone(),
                    text.spans()
                        .first()
                        .and_then(|span| span.tag.as_deref().map(str::to_owned)),
                    text.to_plain_text(),
                ),
                other => panic!("only Sys lines: {other:?}"),
            })
            .collect();
        assert_eq!(
            shaped,
            vec![
                (
                    SendTarget::Conn(1),
                    Some(OUTPUT_PREFIX_TAG.to_owned()),
                    ">>>".to_owned()
                ),
                (
                    SendTarget::Conn(1),
                    Some("system".to_owned()),
                    RefusalReason::CommitPending.text().to_owned()
                ),
                (
                    SendTarget::Conn(1),
                    Some(OUTPUT_SUFFIX_TAG.to_owned()),
                    "<<<".to_owned()
                ),
            ]
        );

        // A prefix alone frames the refusal from one side only.
        world.set_output_suffix(1, None);
        let msg = defer_line(1, "look");
        let brackets = output_brackets_of(&world, &msg);
        refuse_world_msg(msg, RefusalReason::StoreUnavailable, &gateway, brackets);
        let batches =
            std::mem::take(&mut *recorder.batches.lock().expect("recording gateway poisoned"));
        assert_eq!(batches, vec![2], "one batch of two");
        let heard = ap4_heard(&recorder);
        assert_eq!(
            heard,
            vec![
                (SendTarget::Conn(1), ">>>".to_owned()),
                (
                    SendTarget::Conn(1),
                    RefusalReason::StoreUnavailable.text().to_owned()
                ),
            ]
        );
        recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .clear();

        // Another connection's line, no row: the actor-addressed refusal.
        let msg = defer_line(2, "look");
        let brackets = output_brackets_of(&world, &msg);
        assert_eq!(brackets, None);
        refuse_world_msg(msg, RefusalReason::CommitTimedOut, &gateway, brackets);
        assert_eq!(
            ap4_heard(&recorder),
            vec![(
                SendTarget::Char(u64::from(GOD.0)),
                RefusalReason::CommitTimedOut.text().to_owned()
            )]
        );

        // Nothing that is not a typed line carries a row.
        let query = WorldMsg::Inbound(Inbound::Query {
            conn: 1,
            actor: GOD,
            id: 3,
            name: "who".into(),
            args: serde_json::json!({}),
        });
        assert_eq!(output_brackets_of(&world, &query), None);
    }

    /// The two words join the read-only allowlist: a RAM row on the World
    /// needs no store, so a store outage does not refuse them.
    #[test]
    fn ap4_the_bracket_settings_pass_the_store_read_only_gate() {
        let world = World::new_minimal();
        for line in ["OUTPUTPREFIX >>>", "outputsuffix <<<", "OUTPUTSUFFIX"] {
            let cmd = Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: line.to_owned(),
            };
            assert!(
                refuse_store_read_only(&world, &cmd).is_none(),
                "{line} passes the read-only gate"
            );
        }
    }

    /// **16b cut B.5, the timed-out case.** A line queued behind a commit that
    /// outlives the 250 ms bound is refused in a player's words while the
    /// commit still pends, and never runs.
    #[tokio::test]
    async fn a_line_behind_a_commit_that_outlives_the_bound_is_refused_in_a_players_words() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        let sent = Instant::now();
        rig.line(1, "think too-late").await;
        rig.serve_until("the timed-out refusal", |rig| {
            rig.heard_any(RefusalReason::CommitTimedOut.text())
        })
        .await;
        assert!(
            sent.elapsed() >= COMMAND_DEFER,
            "the line waited its whole bound before it was refused"
        );
        assert!(
            !rig.heard_any("committing another action"),
            "a line that waited is refused in the new words, not the old"
        );
        let snapshot = rig.metrics.snapshot();
        assert_eq!(snapshot.command_deferred_total, 1);
        assert_eq!(snapshot.command_defer_timed_out_total, 1);
        assert_eq!(snapshot.commands_total, 2, "the anchor and the refusal");

        // The receipt lands after the refusal, and the refused line stays
        // refused.
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        assert!(rig.handle.barrier().await);
        assert!(!rig.heard_any("too-late"), "the timed-out line never ran");
        rig.shut_down().await;
    }

    /// **16b cut B.5, the barrier rule.** A barrier that arrives behind a queued
    /// line waits for that line — and for the commit the line itself starts —
    /// so `barrier_result`'s "everything sent before has been processed" stays
    /// true.
    #[tokio::test]
    async fn a_barrier_behind_queued_lines_waits_for_them() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (first_serial, first_receipt) = rig.next_write().await;

        rig.line(2, "@create queued-orb").await;
        let barrier = tokio::spawn({
            let handle = rig.handle.clone();
            async move { handle.barrier().await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = rig.metrics.snapshot();
                if snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the line and the barrier behind it both wait");
        rig.nothing_timed_out_yet();
        first_receipt
            .send(Ok(first_serial))
            .expect("world still owns the first receipt");

        // The drained line is served and starts a commit of its own; the
        // barrier behind it is not finished after the FIRST receipt.
        let (second_serial, second_receipt) = rig.next_write().await;
        tokio::task::yield_now().await;
        assert!(
            !barrier.is_finished(),
            "the barrier finished before the queued line's own commit landed"
        );
        second_receipt
            .send(Ok(second_serial))
            .expect("world still owns the second receipt");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), barrier)
                .await
                .expect("the barrier completes after the second receipt")
                .expect("barrier task"),
            "the barrier resolves Ok"
        );
        // `name_free` probes the PLAYER index, which an `@create`d thing never
        // joins, so the object's own creation line is the witness that the
        // queued line ran — released only after its exact receipt.
        assert!(
            rig.heard_any("Created: queued-orb"),
            "the queued line ran and its object exists: {:?}",
            rig.heard()
        );
        assert_eq!(rig.metrics.snapshot().commit_pending_barriers, 0);
        rig.shut_down().await;
    }

    /// Wait (bounded) until the world has taken everything sent so far off its
    /// inbound channel — a lifecycle message parked in its slot included.
    async fn inbound_taken(handle: &WorldHandle) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while handle.tx.capacity() < handle.tx.max_capacity() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the world takes its inbound");
    }

    /// Wait (bounded) until `done` holds, WITHOUT answering any write: the
    /// held receipt stays held.
    async fn wait_until(what: &str, done: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !done() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// **16b cut B.5, the reads.** The motivating loss of cut AA's sweep was a
    /// world READ (`ConnectionReady`) that met another action's commit. The
    /// reads and a query behind a held receipt wait, and once it lands each is
    /// ANSWERED — not refused `Busy` — from the committed world.
    #[tokio::test]
    async fn a_read_and_a_query_behind_a_commit_are_answered_when_the_receipt_lands() {
        let mut rig = held_receipt_rig().await;
        rig.attach(1).await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        let name_free = tokio::spawn({
            let handle = rig.handle.clone();
            async move { handle.name_free("God").await }
        });
        let ready = tokio::spawn({
            let handle = rig.handle.clone();
            async move { handle.connection_ready(1, GOD).await }
        });
        let legal = tokio::spawn({
            let handle = rig.handle.clone();
            async move { handle.legal_actions(1, GOD, PerceivedRef::SelfActor).await }
        });
        assert!(
            rig.handle
                .inbound(Inbound::Query {
                    conn: 1,
                    actor: GOD,
                    id: 7,
                    name: "who".into(),
                    args: serde_json::json!({}),
                })
                .await
        );
        let metrics = Arc::clone(&rig.metrics);
        wait_until("the three reads and the query to wait", || {
            metrics.snapshot().command_deferred_total == 4
        })
        .await;
        assert!(
            !name_free.is_finished() && !ready.is_finished() && !legal.is_finished(),
            "a read behind a held receipt waits; it is not refused at once"
        );
        assert_eq!(rig.recorder.reply_text(7), None, "the query waits too");
        rig.nothing_timed_out_yet();
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");

        async fn answer<T>(join: tokio::task::JoinHandle<T>) -> T {
            tokio::time::timeout(Duration::from_secs(2), join)
                .await
                .expect("the read is answered after the receipt")
                .expect("read task")
        }
        assert_eq!(answer(name_free).await, Ok(false), "God's name is taken");
        assert_eq!(
            answer(ready).await,
            Ok(true),
            "the attached connection is ready"
        );
        // This rig composes no action provider, so the projection's own answer
        // is a typed failure of its own (`Unavailable` from the projector);
        // what matters is that the world ran the projection after the receipt
        // rather than refusing the read as busy.
        let legal = answer(legal).await;
        assert!(
            !matches!(legal, Err(LegalActionReadError::Busy)),
            "the legal-action read was answered, not refused: {legal:?}"
        );
        rig.serve_until("the query's reply", |rig| {
            rig.recorder.reply_text(7).is_some()
        })
        .await;
        let reply = rig.recorder.reply_text(7).unwrap_or_default();
        assert!(
            !reply.contains("committing another action") && !reply.contains("busy too long"),
            "the query was answered, not refused: {reply}"
        );
        let snapshot = rig.metrics.snapshot();
        assert_eq!(snapshot.command_deferred_total, 4);
        assert_eq!(snapshot.command_defer_timed_out_total, 0);
        rig.shut_down().await;
    }

    /// **16b cut B.5, queue before lifecycle.** A line typed before a `QUIT`
    /// runs before the `ConnDown` it is followed by, though that `ConnDown`
    /// reached the world while the line waited: the lifecycle slot is taken
    /// only once the queue is empty.
    #[tokio::test]
    async fn a_line_typed_before_a_disconnect_runs_before_it() {
        let mut rig = held_receipt_rig().await;
        rig.attach(1).await;
        rig.line(2, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        rig.line(1, "who").await;
        assert!(
            rig.handle
                .inbound(Inbound::ConnDown {
                    conn: 1,
                    actor: GOD,
                    reason: DisconnectKind::Quit,
                })
                .await
        );
        // The `ConnDown` is parked in the lifecycle slot before the receipt
        // lands, so the order below is the loop's, not the channel's.
        inbound_taken(&rig.handle).await;
        assert_eq!(rig.metrics.snapshot().command_deferred_total, 1);
        rig.nothing_timed_out_yet();
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");

        let who = |rig: &HeldReceiptRig| {
            rig.recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .find_map(|(_, event)| match event.as_ref() {
                    OutputEvent::Who(entries) => Some(
                        entries
                            .iter()
                            .map(|entry| entry.name.to_string())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
        };
        rig.serve_until("the WHO typed before the quit", |rig| who(rig).is_some())
            .await;
        // Run before the `ConnDown`, the WHO still finds God connected; run
        // after it (the lifecycle slot overtaking the queue) it lists nobody.
        assert_eq!(
            who(&rig).as_deref(),
            Some(&["God".to_owned()][..]),
            "the line typed before the quit ran before it"
        );
        assert_eq!(
            rig.handle.connection_ready(1, GOD).await,
            Ok(false),
            "and the disconnect ran after it"
        );
        assert!(
            !rig.heard_any("committing another action") && !rig.heard_any("busy too long"),
            "{:?}",
            rig.heard()
        );
        rig.shut_down().await;
    }

    /// **16b cut B.5, the shutdown arm.** A shutdown behind a held receipt
    /// refuses every queued line in the words it always used and keeps every
    /// queued barrier behind the receipt, as a retained barrier is kept; the
    /// world ends once the receipt lands.
    #[tokio::test]
    async fn a_shutdown_refuses_the_queued_lines_and_keeps_the_queued_barriers_behind_the_receipt()
    {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        rig.line(1, "think drained-at-shutdown").await;
        let barrier = tokio::spawn({
            let handle = rig.handle.clone();
            async move { handle.barrier_result().await }
        });
        let metrics = Arc::clone(&rig.metrics);
        wait_until("the line and the barrier behind it to wait", || {
            let snapshot = metrics.snapshot();
            snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1
        })
        .await;
        rig.nothing_timed_out_yet();

        // The shutdown watch, not the inbound channel.
        assert!(
            rig.handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        let recorder = Arc::clone(&rig.recorder);
        wait_until("the queued line's refusal", || {
            recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .any(|(_, event)| {
                    matches!(event.as_ref(), OutputEvent::Sys(text)
                        if text.to_plain_text().contains("committing another action"))
                })
        })
        .await;
        assert!(
            !barrier.is_finished(),
            "a queued barrier is kept behind the receipt, not answered at shutdown"
        );
        assert_eq!(
            rig.metrics.snapshot().commands_total,
            2,
            "the anchor and the refused line, once each"
        );

        let HeldReceiptRig {
            mut writer_rx,
            world_join,
            recorder,
            ..
        } = rig;
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), barrier)
                .await
                .expect("the barrier completes with the receipt")
                .expect("barrier task"),
            Ok(()),
            "a barrier kept behind the receipt completes with it"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !world_join.is_finished() {
                if let Ok(StoreJob::Write {
                    batch,
                    notify: Some(receipt),
                }) = writer_rx.try_recv()
                {
                    let _ = receipt.send(Ok(batch.serial));
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the world ends after the receipt");
        world_join.await.expect("world task panicked");
        assert!(
            !recorder
                .sends
                .lock()
                .expect("recording gateway poisoned")
                .iter()
                .any(|(_, event)| matches!(event.as_ref(), OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. }
                    if text.to_plain_text().contains("drained-at-shutdown"))),
            "a line refused at shutdown never ran"
        );
    }

    /// **16b cut B.5, the loop's exit.** When the world's inbound closes behind
    /// a held receipt it ends once the receipt lands, and whatever still waits
    /// in the queue is answered rather than dropped: a line hears the
    /// retryable `CommitPending` refusal, a barrier `Unavailable`.
    #[tokio::test]
    async fn what_still_waits_when_the_loop_ends_is_answered_not_dropped() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        rig.line(1, "think at-exit").await;
        let (reply, mut barrier) = oneshot::channel();
        rig.handle
            .tx
            .send(WorldMsg::Barrier { reply })
            .await
            .expect("world open");
        let metrics = Arc::clone(&rig.metrics);
        wait_until("the line and the barrier behind it to wait", || {
            let snapshot = metrics.snapshot();
            snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1
        })
        .await;
        rig.nothing_timed_out_yet();

        // Every sender goes: the world sees its inbound close with a commit
        // pending and waits for the receipt before it ends.
        let HeldReceiptRig {
            handle,
            mut writer_rx,
            world_join,
            recorder,
            ..
        } = rig;
        drop(handle);
        // Doc 18 cut IC14 row 49 (lead ruling L29, test-side): let the world see
        // its inbound close BEFORE the receipt exists. The test runs on tokio's
        // current-thread runtime, and with the close and the receipt both ready
        // the loop's `select!` picks either first: about one run in fifteen
        // (2 of 30 measured) it took the receipt first, completed the commit with
        // its inbound still open, and ran the queued line as an ordinary drain —
        // a legitimate path, but not the one this test proves. Yielding hands the
        // world its turns while only the close is ready, so its `Wake::Closed` arm
        // marks the pending commit `exit_after_ack` first; what the test asserts is
        // unchanged.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(!world_join.is_finished(), "the world waits for the receipt");
        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !world_join.is_finished() {
                if let Ok(StoreJob::Write {
                    batch,
                    notify: Some(receipt),
                }) = writer_rx.try_recv()
                {
                    let _ = receipt.send(Ok(batch.serial));
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the world ends after the receipt");
        world_join.await.expect("world task panicked");
        assert_eq!(
            barrier.try_recv(),
            Ok(Err(WorldReadError::Unavailable)),
            "a queued barrier hears that the world is gone"
        );
        let heard: Vec<String> = recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .filter_map(|(_, event)| match event.as_ref() {
                OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                    Some(text.to_plain_text())
                }
                _ => None,
            })
            .collect();
        assert!(
            heard
                .iter()
                .any(|line| line.contains("committing another action")),
            "the queued line is refused, not dropped: {heard:?}"
        );
        assert!(
            !heard.iter().any(|line| line.contains("at-exit")),
            "{heard:?}"
        );
    }

    /// **16b cut B.5, the barrier at the cap.** The queue is only ever
    /// non-empty behind a pending commit, so a barrier that meets a full queue
    /// is refused `Busy` — the retryable overload answer — never answered `Ok`
    /// ahead of the 64 entries and the receipt they wait on.
    #[tokio::test]
    async fn a_barrier_that_meets_a_full_defer_queue_is_refused_busy() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;

        let reads: Vec<_> = (0..COMMAND_DEFER_CAP)
            .map(|n| {
                let handle = rig.handle.clone();
                tokio::spawn(async move { handle.name_free(&format!("free-{n}")).await })
            })
            .collect();
        let metrics = Arc::clone(&rig.metrics);
        let cap = u64::try_from(COMMAND_DEFER_CAP).expect("small");
        wait_until("the queue to fill", || {
            metrics.snapshot().command_deferred_total == cap
        })
        .await;
        let refused = tokio::time::timeout(Duration::from_secs(2), rig.handle.barrier_result())
            .await
            .expect("a barrier at the cap is answered at once");
        rig.nothing_timed_out_yet();
        assert_eq!(refused, Err(WorldReadError::Busy));
        assert_eq!(
            rig.metrics.snapshot().commit_pending_barrier_refused_total,
            1
        );

        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        for read in reads {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), read)
                    .await
                    .expect("each queued read is answered")
                    .expect("read task"),
                Ok(true)
            );
        }
        rig.shut_down().await;
    }

    /// The pending-receipt custody guarantees of K0 hold with the defer queue in
    /// place (16b cut B.5 retitled and rewrote this: until then a command behind
    /// a held receipt was refused at once). A command typed behind the receipt
    /// waits in the queue; a thousand full reattach rosters behind it still
    /// coalesce into one pending snapshot, so the queue never head-of-line
    /// blocks operational traffic; the barrier sent after the command waits
    /// behind it; and once the receipt lands the command is SERVED — its own
    /// write reaches the writer — and the barrier completes only after that
    /// second receipt.
    #[tokio::test]
    async fn a_deferred_command_keeps_pending_reattach_live_and_holds_the_barrier_behind_it() {
        let mut rig = held_receipt_rig().await;
        rig.line(1, "@create receipt-anchor").await;
        let (serial, receipt) = rig.next_write().await;
        let metrics = Arc::clone(&rig.metrics);
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

        // A thousand full reattach rosters coalesce into one pending snapshot.
        // All but the last go BEFORE the command, so the command's 250 ms bound
        // (wall clock) covers only one reattach, the barrier and the poll —
        // never a thousand loop turns on a starved runner.
        let reattach = |conn: u64| {
            let handle = rig.handle.clone();
            async move {
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
        };
        for conn in 10_000..10_999_u64 {
            reattach(conn).await;
        }
        // Retryable input, not accepted semantic custody: it waits in the
        // bounded queue and leaves the FIFO available for the operational
        // messages behind it — the last reattach and the barrier.
        rig.line(2, "@create must-not-exist").await;
        reattach(10_999).await;
        let barrier = tokio::spawn({
            let handle = rig.handle.clone();
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
        .expect("operational traffic must remain live behind a deferred command");
        assert!(
            !barrier.is_finished(),
            "barrier must still wait for the receipt"
        );
        assert_eq!(metrics.snapshot().commit_pending_sends, baseline_sends);
        assert_eq!(metrics.snapshot().commit_pending_effects, baseline_effects);
        assert_eq!(metrics.snapshot().command_deferred_total, 1);
        rig.nothing_timed_out_yet();

        receipt
            .send(Ok(serial))
            .expect("world still owns the exact receipt");
        // The deferred command is served: its own write reaches the writer,
        // and the barrier sent after it waits for that write's receipt too.
        let (second_serial, second_receipt) = rig.next_write().await;
        tokio::task::yield_now().await;
        assert!(
            !barrier.is_finished(),
            "the barrier overtook the deferred command's own commit"
        );
        second_receipt
            .send(Ok(second_serial))
            .expect("world still owns the second receipt");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), barrier)
                .await
                .expect("barrier must complete after the second receipt")
                .expect("barrier task")
        );
        // The command ran and its object exists (`name_free` probes the player
        // index, which an `@create`d thing never joins, so the creation line —
        // released only after the second receipt — is the witness).
        assert!(
            rig.heard_any("Created: must-not-exist"),
            "the deferred command was served: {:?}",
            rig.heard()
        );
        assert!(
            !rig.heard_any("committing another action") && !rig.heard_any("busy too long"),
            "the deferred command was served, never refused: {:?}",
            rig.heard()
        );
        assert_eq!(metrics.snapshot().commit_pending_barriers, 0);
        assert_eq!(metrics.snapshot().commit_pending_reattach_records, 0);
        rig.shut_down().await;
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
            action: None,
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
            &test_effect_store(),
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
            _ctx: &mush_core::HookReadContext<'_>,
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
        assert!(!can_perceive(&reg, &world, alice, wraith, DETACHED));
        assert!(can_perceive(&reg, &world, bob, wraith, DETACHED));
        assert!(can_perceive(&reg, &world, alice, bob, DETACHED));
        assert!(can_perceive(&reg, &world, wraith, wraith, DETACHED));

        // D2(a): room_view drops the concealed occupant for Alice only.
        let names = |looker| -> Vec<String> {
            room_view(&reg, &world, looker, hall, DETACHED)
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
            DETACHED,
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
        dispatch(
            &reg,
            &mut world,
            bob,
            &format!("examine {hall}"),
            &mut out,
            DETACHED,
        );
        assert!(
            examine_contents(&out, bob).contains("Wraith"),
            "Bob's examine must still show Wraith"
        );

        // D2(b)+D3: the live Room fan-out. Wraith speaks; the event's subject
        // is Wraith, so Alice (who cannot perceive him) is dropped from the
        // resolved recipients while Bob and Wraith himself receive it.
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, wraith, "say boo", &mut out, DETACHED);
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
        assert!(can_perceive(&reg, &world, a, b, DETACHED));
        assert!(
            room_view(&reg, &world, a, hall, DETACHED)
                .unwrap()
                .contents
                .iter()
                .any(|r| r.name == "Bee")
        );

        // A say reaches every connected occupant (speaker included).
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, a, "say hi", &mut out, DETACHED);
        let recipients: std::collections::HashSet<ObjId> = resolve_sends(&reg, &world, &mut out)
            .iter()
            .filter_map(|(t, _)| match t {
                SendTarget::Char(ck) => Some(ObjId(u32::try_from(*ck).unwrap_or(u32::MAX))),
                _ => None,
            })
            .collect();
        assert!(recipients.contains(&a) && recipients.contains(&b));
    }

    /// `@emit` is the one verb that can put an unattributed line in front of
    /// another player, so it is a granted flag (`SPOOF`, Royalty+ implicit),
    /// and below Wizard the line still travels with its source: a `NOSPOOF`
    /// listener and staff receive it as `Spoof` (rendered `[Name] text`),
    /// everyone else as the bare `Emit`. Wizard narration is plain for all.
    #[test]
    fn spoofed_emits_carry_their_source_only_to_entitled_listeners() {
        use mush_core::{Flags, dispatch, register_core_commands};
        use mush_proto::NarrativeKind;

        let mut world = World::new_minimal();
        let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let ann = world
            .create(ObjKind::Player, "Ann", GOD, Some(hall))
            .unwrap();
        let bee = world
            .create(ObjKind::Player, "Bee", GOD, Some(hall))
            .unwrap();
        let cal = world
            .create(ObjKind::Player, "Cal", GOD, Some(hall))
            .unwrap();
        let dee = world
            .create(ObjKind::Player, "Dee", GOD, Some(hall))
            .unwrap();
        for p in [ann, bee, cal, dee] {
            world.set_connected(p, true);
        }
        world.set_flag(bee, Flags::NOSPOOF, true).unwrap();
        world.set_flag(cal, Flags::ROYALTY, true).unwrap();
        world.set_flag(dee, Flags::WIZARD, true).unwrap();
        let mut reg = Registry::new();
        register_core_commands(&mut reg).unwrap();

        // The narrative kind each listener received from one dispatch.
        let narratives = |world: &World, out: &mut Outbox| -> Vec<(ObjId, NarrativeKind)> {
            resolve_sends(&reg, world, out)
                .iter()
                .filter_map(|(t, ev)| match (t, ev.as_ref()) {
                    (SendTarget::Char(ck), OutputEvent::Narrative { kind, .. }) => {
                        Some((ObjId(u32::try_from(*ck).unwrap_or(u32::MAX)), *kind))
                    }
                    _ => None,
                })
                .collect()
        };

        // An ordinary player is refused and the room hears nothing.
        let mut out = Outbox::new();
        dispatch(
            &reg,
            &mut world,
            ann,
            "@emit A door slams.",
            &mut out,
            DETACHED,
        );
        assert!(narratives(&world, &mut out).is_empty());
        // They cannot grant themselves the licence; the observer preference
        // is theirs to set.
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, ann, "@set me=SPOOF", &mut out, DETACHED);
        assert!(!world.get(ann).unwrap().flags.contains(Flags::SPOOF));
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, ann, "@set me=NOSPOOF", &mut out, DETACHED);
        assert!(world.get(ann).unwrap().flags.contains(Flags::NOSPOOF));
        world.set_flag(ann, Flags::NOSPOOF, false).unwrap();

        // Granted the flag: bare for Ann herself, sourced for the NOSPOOF
        // listener and for staff.
        world.set_flag(ann, Flags::SPOOF, true).unwrap();
        let mut out = Outbox::new();
        dispatch(
            &reg,
            &mut world,
            ann,
            "@emit A door slams.",
            &mut out,
            DETACHED,
        );
        let got = narratives(&world, &mut out);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got.contains(&(ann, NarrativeKind::Emit)));
        assert!(got.contains(&(bee, NarrativeKind::Spoof)));
        assert!(got.contains(&(cal, NarrativeKind::Spoof)));
        assert!(got.contains(&(dee, NarrativeKind::Spoof)));

        // Wizard narration is plain for everyone, NOSPOOF or not.
        let mut out = Outbox::new();
        dispatch(&reg, &mut world, dee, "@emit Thunder.", &mut out, DETACHED);
        let got = narratives(&world, &mut out);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got.iter().all(|(_, k)| *k == NarrativeKind::Emit));
    }

    // ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
    //
    // 08j §5 (iii) verbatim: *"Settle the ingress shape before cut 2:
    // `CommandHandler`/`Ctx` is synchronous and cannot reach
    // `ActionCoordinator::gateway_ingress`, so the first `Authoritative`
    // registration needs an adapter seam that does not exist."* 08i §2 row 5:
    // *"There is no `Ctx`-level action submit."* These tests are the discharge.

    fn b1_world() -> mush_testkit::TestWorld {
        let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(
            mush_contract_tests::b1_seam::B1SeamModule,
        )]);
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x61; 32]).unwrap(), []).unwrap(),
        );
        world
    }

    fn b1_run(world: &mut mush_testkit::TestWorld, line: &str) -> Outbox {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        world.engine_mut().step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: line.to_owned(),
            },
            &mut out,
            &mut fx,
        );
        out
    }

    fn b1_lines(out: &Outbox) -> Vec<String> {
        out.events()
            .iter()
            .map(|(_, event)| mush_testkit::text_of(event))
            .collect()
    }

    /// The whole point of B1: a command classified `Authoritative` reaches
    /// `ActionCoordinator::prepare_and_stage_resolved`, and what it stages is an
    /// authoritative bundle — never the legacy lane.
    #[tokio::test]
    async fn e7c2_b1_a_command_reaches_prepare_and_stage_resolved() {
        let mut world = b1_world();
        let out = b1_run(&mut world, "@b1submit 3");
        assert!(
            world.engine().has_authoritative_submit(),
            "the handler's one submission must be queued"
        );
        assert_eq!(
            b1_lines(&out),
            vec!["Queued.".to_owned()],
            "the handler itself confirms nothing; the engine acknowledges RECEIPT \
             only (E7c2 fold R11)"
        );
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        let staged = match driver.drive_one(&mut engine, &store).await.unwrap() {
            CommandSubmitDrive::Staged { staged, actor, .. } => {
                assert_eq!(actor, GOD);
                staged
            }
            CommandSubmitDrive::Refused { detail, .. } => {
                panic!("the command submission was refused: {detail}")
            }
            _ => panic!("the command submission must stage an authoritative action"),
        };
        let (bundle, _installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        let bundle = *bundle;
        assert_eq!(bundle.token(), staged.token);
        assert_eq!(
            bundle.action().key().id.as_str(),
            mush_contract_tests::b1_seam::B1_ACTION
        );
        assert!(!engine.has_authoritative_submit());
    }

    /// A handler that fans out stages **nothing at all** — not one half.
    #[tokio::test]
    async fn e7c2_b1_a_handler_submitting_twice_refuses_and_stages_nothing() {
        let mut world = b1_world();
        let out = b1_run(&mut world, "@b1submit/twice 3");
        assert!(
            !world.engine().has_authoritative_submit(),
            "a fan-out voids the first submission too"
        );
        let lines = b1_lines(&out);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("more than one authoritative action")),
            "{lines:?}"
        );
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::Idle
        ));
        assert!(engine.take_staged_action().unwrap().is_none());
    }

    /// A submission that arrives beside legacy persistence work is **deferred,
    /// not dropped** — the same rule `DurableScheduleDriver::drive_one` follows,
    /// and the one `flush_prepared_action` enforces at the writer boundary.
    #[tokio::test]
    async fn e7c2_b1_a_submission_beside_legacy_work_is_deferred_not_dropped() {
        let mut world = b1_world();
        // Clear boot dirt first, so the only legacy work is the handler's own.
        let _ = world.drain_write_batch();
        let _ = b1_run(&mut world, "@b1submit/dirty 2");
        let store = mush_store::InMemoryStore::new();
        let mut engine = world.into_engine();
        assert!(engine.has_legacy_persistence_work());
        let mut driver = CommandSubmitDriver::new(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::DeferredByLegacyPredecessor
        ));
        assert!(
            engine.has_authoritative_submit(),
            "the deferred submission is retained, never dropped"
        );
        // The flush drains the legacy op; the very next drive stages it.
        let ops = engine.take_module_ops();
        assert_eq!(ops.len(), 1);
        assert!(!engine.has_legacy_persistence_work());
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::Staged { .. }
        ));
    }

    /// Two identical `@b1submit`s inside one epoch produce **one** commit: the
    /// second preflights as a HIT and its confirmation is projected from the
    /// durable receipt, with no provider and no second bundle.
    #[tokio::test]
    async fn e7c2_b1_two_identical_submissions_in_one_epoch_produce_one_commit() {
        let mut world = b1_world();
        let _ = b1_run(&mut world, "@b1submit 4");
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::Staged { .. }
        ));
        let (bundle, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();

        // The identical command again, in the same epoch.
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@b1submit 4".to_owned(),
            },
            &mut out,
            &mut fx,
        );
        match driver.drive_one(&mut engine, &store).await.unwrap() {
            CommandSubmitDrive::AlreadyFinished { actor, receipt, .. } => {
                assert_eq!(actor, GOD);
                assert_eq!(receipt.token(), bundle.token());
            }
            CommandSubmitDrive::Refused { detail, .. } => {
                panic!("the retyped identical command must be a preflight HIT: {detail}")
            }
            CommandSubmitDrive::Staged { .. } => panic!("a HIT must not stage a second commit"),
            _ => panic!("unexpected command-ingress outcome"),
        }
        assert!(
            engine.take_staged_action().unwrap().is_none(),
            "a HIT stages nothing"
        );
    }

    /// A retired idempotency epoch is recoverable for the command lane exactly
    /// as it is for the schedule lane: raise the watermark and retry.
    #[tokio::test]
    async fn e7c2_b1_a_retired_epoch_is_raised_and_the_retry_succeeds() {
        let mut world = b1_world();
        let _ = b1_run(&mut world, "@b1submit 5");
        let store = mush_store::InMemoryStore::new();
        let submit = {
            let mut probe = b1_world();
            let _ = b1_run(&mut probe, "@b1submit 5");
            let mut engine = probe.into_engine();
            *engine
                .take_authoritative_submit()
                .expect("probe submission")
        };
        let old = ActionCoordinator::command_ingress(&submit, 1).unwrap();
        store
            .retire_authoritative_epochs(old.idempotency().scope(), 9)
            .await
            .expect("seed a retired command epoch");
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::Staged { .. }
        ));
        assert!(
            driver.idempotency_epoch >= 9,
            "the retirement watermark must be adopted"
        );
    }

    /// §4's standing checklist item 1 — *no command can confirm before its
    /// durable acknowledgement*. The handler emits nothing; the seam's own
    /// confirmation is `OnCommit`, so the delivery boundary withholds it until
    /// this exact bundle's receipt resolves and suppresses it on abort.
    #[tokio::test]
    async fn e7c2_b1_the_confirmation_never_outruns_the_durable_ack() {
        let mut world = b1_world();
        let handler_out = b1_run(&mut world, "@b1submit 6");
        assert_eq!(
            b1_lines(&handler_out),
            vec!["Queued.".to_owned()],
            "an Authoritative handler emits no success presentation of its own; the \
             only line is the engine's BestEffort receipt acknowledgement"
        );
        assert_eq!(
            handler_out.classes(),
            [mush_core::DeliveryClass::BestEffort]
        );
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        let CommandSubmitDrive::Staged { staged, actor, .. } =
            driver.drive_one(&mut engine, &store).await.unwrap()
        else {
            panic!("staged")
        };
        // What the world loop then queues, and its delivery class.
        let mut out = Outbox::new();
        out.info(
            actor,
            staged.summary.clone().unwrap_or_else(|| "Done.".to_owned()),
        );
        assert_eq!(out.classes(), [mush_core::DeliveryClass::OnCommit]);
    }

    // ---- E7 cut 2 independent verify fold ----

    fn b1_run_as(world: &mut mush_testkit::TestWorld, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        world.engine_mut().step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.to_owned(),
            },
            &mut out,
            &mut fx,
        );
        out
    }

    /// `EconomyModule` + `InventoryModule` behind the real command seam, with a
    /// container-backed vault and a seeded pool: the smallest world in which a
    /// typed `@inv/move` can actually commit.
    fn inv_world() -> (
        mush_testkit::TestWorld,
        mush_inventory::InventoryFamilies,
        u64,
    ) {
        let mut world = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_economy::EconomyModule::new()),
            Box::new(mush_inventory::InventoryModule::new()),
        ]);
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x62; 32]).unwrap(), []).unwrap(),
        );
        let families = world
            .world()
            .module_state::<mush_inventory::InventoryHandles>()
            .expect("inventory handles")
            .0
            .clone();
        let vault = |world: &mut mush_testkit::TestWorld, name: &str| -> u64 {
            let created = world.run(GOD, &format!("@create {name}"));
            let object = created
                .iter()
                .find_map(|(_, event)| {
                    let text = mush_testkit::text_of(event);
                    let at = text.find("(#")?;
                    text[at + 2..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse::<u64>()
                        .ok()
                })
                .expect("created object");
            let out = world.run(GOD, &format!("@vault/create {name}=container:#{object}"));
            out.iter()
                .find_map(|(_, event)| {
                    let text = mush_testkit::text_of(event);
                    let at = text.find("vault #")?;
                    text[at + 7..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse::<u64>()
                        .ok()
                })
                .expect("vault id")
        };
        let from = vault(&mut world, "from");
        let _to = vault(&mut world, "to");
        {
            let engine = world.engine_mut();
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory transaction");
            families
                .credit(
                    &mut txn,
                    mush_inventory::EndpointRef(mush_economy::VaultId(from)),
                    &mush_core::CanonicalId::new("metals").expect("commodity"),
                    100,
                )
                .expect("seed");
            let prepared = txn.prepare().expect("prepared");
            let (_, installation) = prepared.into_parts();
            engine
                .install_composed_action(installation)
                .expect("the fixture installs");
        }
        (world, families, from)
    }

    fn inv_available(
        engine: &mut Engine,
        families: &mush_inventory::InventoryFamilies,
        vault: u64,
    ) -> u64 {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .available(
                &mut txn,
                mush_inventory::EndpointRef(mush_economy::VaultId(vault)),
                &mush_core::CanonicalId::new("metals").expect("commodity"),
            )
            .expect("available")
    }

    /// D1 as taken: the durable key derives from the request identity alone, so a
    /// replay after a reboot is a **HIT** with no second bundle. The epoch is a
    /// per-process random and does NOT survive a restart, and `InventoryVerb::Move`
    /// derives nothing and consumes nothing — so before the fold the second
    /// attempt moved the quantity twice and rewrote the same
    /// `inventory.transferred` marker over it.
    #[tokio::test]
    async fn vf_r1_a_replayed_move_across_an_epoch_change_moves_nothing_twice() {
        let (mut world, families, from) = inv_world();
        let to = from + 1;
        let _ = world.run(GOD, &format!("@inv/move {from}/metals:10={to}"));
        let submit = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("the command queued a submission");
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();

        // Attempt one, on this process's epoch.
        let first = ActionCoordinator::command_ingress(&submit, 1).unwrap();
        let ActionPreflight::Vacant(permit) =
            ActionCoordinator::preflight(&store, first).await.unwrap()
        else {
            panic!("the first attempt is vacant")
        };
        let submission = command_offer_submission(&engine, &submit).expect("offer");
        let (staged, _) = resolve_and_stage_offer(&mut engine, permit, submission).expect("staged");
        let (bundle, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        assert_eq!(bundle.token(), staged.token);
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        assert_eq!(inv_available(&mut engine, &families, from), 90);

        // The reboot: a fresh process epoch. The REQUEST identity survives it and
        // the durable key does not, so preflight is vacant all over again.
        let replay = ActionCoordinator::command_ingress(&submit, 2).unwrap();
        let epoch_free = ActionCoordinator::command_ingress(&submit, 1).unwrap();
        assert_eq!(
            replay.request(),
            epoch_free.request(),
            "the request identity is epoch-free"
        );
        assert_ne!(
            replay.idempotency().epoch(),
            epoch_free.idempotency().epoch(),
            "but the durable key it is looked up under is not"
        );
        let ActionPreflight::Vacant(permit) =
            ActionCoordinator::preflight(&store, replay).await.unwrap()
        else {
            panic!("the ingress cannot catch this; the mechanic must")
        };
        let submission = command_offer_submission(&engine, &submit).expect("offer");
        let refused = resolve_and_stage_offer(&mut engine, permit, submission)
            .expect_err("the applied marker refuses the replay");
        assert!(
            refused.contains("already been applied"),
            "the marker is the replay guard: {refused}"
        );
        assert_eq!(
            inv_available(&mut engine, &families, from),
            90,
            "and the quantity moved exactly once"
        );
    }

    /// **Doc 18 cut IC8: the stock clerk's first mint drives through the command
    /// ingress** (lead 2c-v). On the game roster with `[tacsim].enabled`, the first
    /// `tacsim.port` fire queues a submission whose actor is the Ceres stock clerk — a
    /// Thing, not a player and not GOD — and it lands exactly as a command's does:
    /// `command_ingress` → `preflight` Vacant → `command_offer_submission` →
    /// `resolve_and_stage_offer` → reserve, reconcile, install, checkpoint. The receipt
    /// and the lot exist; the same ingress replayed after an epoch change is refused by
    /// the receipt and mints nothing twice.
    #[tokio::test]
    async fn ic8_the_clerks_first_mint_drives_through_the_command_ingress() {
        let llm = Arc::new(
            mush_llm::LlmService::builder()
                .global_enabled(false)
                .build(),
        );
        let mut config = mush_config::config::Config::default();
        config.tacsim.enabled = true;
        let roster = crate::boot::game_modules(&config, &llm)
            .expect("an enabled config composes the game roster");
        let mut world = mush_testkit::TestWorld::with_modules(roster);
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x63; 32]).unwrap(), []).unwrap(),
        );
        let lane = mush_core::CanonicalId::new(tacsim::definitions::content::LANE_PACK).unwrap();
        let ceres = mush_core::CanonicalId::new("ceres_port").unwrap();
        let clerk = match world
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("the layer")
            .port_map(&lane, &ceres)
        {
            tacsim::port::PortMapState::Present(map) => {
                ObjId(map.clerk.as_ref().expect("a clerk").thing)
            }
            other => panic!("the Ceres map: {other:?}"),
        };
        let t = world.world().now().saturating_add(6);
        world.set_now(t);
        let _ = world.fire_due(t.saturating_mul(1_000));
        let submit = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("the first fire queued a submission");
        assert_eq!(submit.actor, clerk, "the Ceres stock clerk's");
        assert_ne!(submit.actor, GOD);
        assert_eq!(
            world.world().get(clerk).map(|o| o.kind),
            Some(mush_core::ObjKind::Thing)
        );
        assert_eq!(submit.action.key().id.as_str(), tacsim::port::STOCK_ACTION);
        let issue = tacsim::port::StockIssueId::parse(
            submit.parameters["issue"].as_str().expect("an issue key"),
        )
        .expect("an issue id");
        let handles = world
            .world()
            .module_state::<tacsim::port::PortHandles>()
            .cloned()
            .expect("the port handles");
        let inventory = handles.inventory.clone().expect("the filled cell");
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();

        let first = ActionCoordinator::command_ingress(&submit, 1).unwrap();
        let ActionPreflight::Vacant(permit) =
            ActionCoordinator::preflight(&store, first).await.unwrap()
        else {
            panic!("the first attempt is vacant")
        };
        let submission = command_offer_submission(&engine, &submit).expect("offer");
        let (staged, _) = resolve_and_stage_offer(&mut engine, permit, submission).expect("staged");
        let (bundle, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        assert_eq!(bundle.token(), staged.token);
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        engine
            .install_composed_action(installation)
            .expect("the mint installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        let receipt = {
            let mut txn = engine
                .action_transaction(&handles.families.port_stock)
                .expect("a tacsim transaction");
            txn.get(&handles.families.port_stock, &issue)
                .expect("the receipts read")
                .cloned()
                .expect("the receipt exists")
        };
        assert_eq!(receipt.clerk, clerk);
        let lot = {
            let mut txn = engine
                .action_transaction(&inventory.lots)
                .expect("an inventory transaction");
            txn.get(&inventory.lots, &mush_inventory::LotId(receipt.lot))
                .expect("the lots read")
                .cloned()
        };
        assert!(lot.is_some(), "the lot exists");

        // A fresh process epoch: the ingress is vacant again, and the receipt refuses.
        let replay = ActionCoordinator::command_ingress(&submit, 2).unwrap();
        let ActionPreflight::Vacant(permit) =
            ActionCoordinator::preflight(&store, replay).await.unwrap()
        else {
            panic!("the ingress cannot catch this; the receipt must")
        };
        let submission = command_offer_submission(&engine, &submit).expect("offer");
        let refused = resolve_and_stage_offer(&mut engine, permit, submission)
            .expect_err("the receipt refuses the replay");
        assert!(refused.contains("already minted"), "{refused}");
        let mut txn = engine
            .action_transaction(&inventory.lots)
            .expect("an inventory transaction");
        assert!(
            txn.get(&inventory.lots, &mush_inventory::LotId(receipt.lot))
                .expect("the lots read")
                .is_some(),
            "minted exactly once"
        );
    }

    /// The other half of D1: a deliberately **retyped** intent is a new request,
    /// as the player means it. The handler's `key` used to be a constant per-verb
    /// discriminator, so `@inv/move 1/metals:5=2` had one request identity for
    /// that actor for ever and "move five more" silently collapsed into a HIT.
    #[tokio::test]
    async fn vf_r1_two_deliberately_retyped_moves_move_twice() {
        let (mut world, _families, from) = inv_world();
        let to = from + 1;
        let line = format!("@inv/move {from}/metals:5={to}");
        let _ = world.run(GOD, &line);
        let first = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("first submission");
        let _ = world.run(GOD, &line);
        let second = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("second submission");
        assert_ne!(
            first.key.as_str(),
            second.key.as_str(),
            "two typed intents carry two discriminators"
        );
        let a = ActionCoordinator::command_ingress(&first, 1).unwrap();
        let b = ActionCoordinator::command_ingress(&second, 1).unwrap();
        assert_ne!(
            a.request(),
            b.request(),
            "and therefore two request identities, so both commit"
        );
        // While driving the SAME submission value twice is still one identity.
        let again = ActionCoordinator::command_ingress(&first, 1).unwrap();
        assert_eq!(a.request(), again.request());
    }

    // ---- E7 cut 3 independent verify fold: B2 (the @extract e2e gap) ----

    /// A test-only module that seeds GOD's extraction-licence account: the
    /// entitlement instrument (no `@instrument/register` builds one — it is
    /// hardcoded `Currency`) and an `Active` account in it. Composed only in this
    /// test's world, never in a production composition, so the allowlist tripwire
    /// (which enumerates the *production* registry) never sees `@seedlicence`.
    struct LicenceSetup;

    fn seed_licence(ctx: &mut mush_core::Ctx<'_>) -> Result<(), mush_core::CmdError> {
        let id = mush_economy::InstrumentId::new("tacsim.extraction").expect("instrument id");
        let _ = mush_economy::register_instrument(
            ctx,
            mush_economy::InstrumentDef {
                id: id.clone(),
                ruleset: SmolStr::new_static("tacsim"),
                code: SmolStr::new("tacsim.extraction"),
                kind: mush_economy::InstrumentKind::Entitlement,
                scale: 0,
                transferable: false,
                negative_policy: mush_economy::NegativePolicy::SystemOnly,
                status: mush_economy::InstrumentStatus::Active,
                display: None,
                revision: 1,
            },
        );
        let account_id = {
            ctx.w
                .module_state_mut::<mush_economy::EconomyStore>()
                .expect("economy store")
                .alloc_account()
                .expect("an account id")
        };
        mush_economy::open_account(
            ctx,
            mush_economy::Account {
                id: account_id,
                owner: mush_org::PrincipalId::Object(ctx.actor),
                instrument: id,
                scope: None,
                name: SmolStr::new("extraction licence"),
                status: mush_economy::AccountStatus::Active,
                posted_balance: mush_economy::Amount::ZERO,
                held_amount: mush_economy::Amount::ZERO,
                last_hold_seq: 0,
                credit: mush_economy::CreditPolicy::forbid(),
                last_journal_seq: 0,
                revision: 1,
                approval: None,
            },
        );
        Ok(())
    }

    impl mush_core::GameModule for LicenceSetup {
        fn name(&self) -> &'static str {
            "testlicencesetup"
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
            reg.command(
                mush_core::CommandSpec {
                    name: "@seedlicence".into(),
                    switches: Vec::new(),
                    shape: mush_core::ArgShape::EqPair,
                    ..mush_core::CommandSpec::default()
                },
                std::sync::Arc::new(|ctx: &mut mush_core::Ctx<'_>| seed_licence(ctx)),
            )?;
            Ok(())
        }
    }

    /// economy + inventory(delegates "industry") + industry(licence) + the licence
    /// seeder, behind the real command seam: the smallest world in which a typed
    /// `@extract/cycle` can actually commit a lot.
    fn extract_world() -> (mush_testkit::TestWorld, mush_industry::IndustryFamilies) {
        let cell: mush_inventory::InventoryHandleCell = std::sync::Arc::default();
        let licence = mush_economy::InstrumentId::new("tacsim.extraction").expect("id");
        let mut world = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_economy::EconomyModule::new()),
            Box::new(
                mush_inventory::InventoryModule::new()
                    .with_quantity_delegates(["industry"])
                    .with_handle_cell(std::sync::Arc::clone(&cell)),
            ),
            Box::new(
                mush_industry::IndustryModule::new()
                    .with_inventory(std::sync::Arc::clone(&cell))
                    .with_extraction_licence(licence),
            ),
            Box::new(LicenceSetup),
        ]);
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x71; 32]).unwrap(), []).unwrap(),
        );
        let families = world
            .world()
            .module_state::<mush_industry::IndustryHandles>()
            .expect("industry handles")
            .0
            .clone();
        (world, families)
    }

    fn apply_industry<T>(
        engine: &mut Engine,
        families: &mush_industry::IndustryFamilies,
        body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, mush_industry::IndustryError>,
    ) -> Result<T, mush_industry::IndustryError> {
        let (result, prepared) = {
            let mut txn = engine
                .action_transaction(&families.deposits)
                .expect("industry transaction");
            let result = body(&mut txn);
            match result {
                Ok(value) => (Ok(value), txn.prepare().ok()),
                Err(error) => (Err(error), None),
            }
        };
        if let Some(prepared) = prepared {
            let (_, installation) = prepared.into_parts();
            engine
                .install_composed_action(installation)
                .expect("the fixture installs");
        }
        result
    }

    fn make_gods_vault(world: &mut mush_testkit::TestWorld) -> u64 {
        let created = world.run(GOD, "@create facility");
        let object = created
            .iter()
            .find_map(|(_, event)| {
                let text = mush_testkit::text_of(event);
                let at = text.find("(#")?;
                text[at + 2..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u64>()
                    .ok()
            })
            .expect("created object");
        let out = world.run(GOD, &format!("@vault/create facility=container:#{object}"));
        out.iter()
            .find_map(|(_, event)| {
                let text = mush_testkit::text_of(event);
                let at = text.find("vault #")?;
                text[at + 7..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u64>()
                    .ok()
            })
            .expect("vault id")
    }

    /// Drive one queued authoritative submission through the coordinator to a
    /// durable checkpoint (or to the refusal `resolve_and_stage_offer` raises).
    async fn drive_submit(
        world: &mut mush_testkit::TestWorld,
        store: &mush_store::InMemoryStore,
        submit: &mush_core::AuthoritativeSubmit,
        epoch: u64,
    ) -> Result<String, String> {
        let ingress = ActionCoordinator::command_ingress(submit, epoch).unwrap();
        let permit = match ActionCoordinator::preflight(store, ingress).await.unwrap() {
            ActionPreflight::Vacant(permit) => permit,
            _ => return Err("preflight was not vacant".to_owned()),
        };
        let engine = world.engine_mut();
        let submission = command_offer_submission(engine, submit)?;
        let (staged, _) = resolve_and_stage_offer(engine, permit, submission)?;
        let summary = staged.summary.as_deref().unwrap_or_default().to_owned();
        let (bundle, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("a staged action")
            .into_parts();
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        Ok(summary)
    }

    // ---- E7 cut 5 independent verify fold: the driver-level C1 e2e ----

    /// A fixed character->account oracle for the sim focus seam.
    struct FoldMapOracle(std::collections::BTreeMap<ObjId, mush_core::AccountId>);
    impl mush_core::AccountIdentity for FoldMapOracle {
        fn account_of(&self, character: ObjId) -> Option<mush_core::AccountId> {
            self.0.get(&character).copied()
        }
    }

    /// The `(unit, station)` lease post-image carried in an applied bundle's
    /// replay writes (the driver never re-reads the internal family handle).
    fn bundle_lease(bundle: &AuthoritativeBundle) -> mush_core::ControlLeaseState {
        let value = bundle
            .writes()
            .iter()
            .find_map(|write| match write {
                mush_core::ReplayWrite::ModuleJsonSet { key, value, .. }
                    if key.as_str().contains("control_leases") =>
                {
                    Some(value.clone())
                }
                _ => None,
            })
            .expect("the bundle carries a control-lease post-image");
        let state = value.get("value").cloned().unwrap_or(value);
        serde_json::from_value(state).expect("a valid control-lease post-image")
    }

    /// Discover, prepare and COMMIT the one due scheduled control-handoff apply,
    /// returning its bundle so the seated lease can be read.
    async fn drive_scheduled_apply(
        world: &mut mush_testkit::TestWorld,
        store: &mush_store::InMemoryStore,
        at: i64,
    ) -> AuthoritativeBundle {
        world.world_mut().set_now(at);
        let candidate = world
            .engine()
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
            .expect("discovery")
            .candidates()[0]
            .clone();
        let ingress =
            ActionCoordinator::scheduled_ingress(candidate.action(), candidate.advance(), 1)
                .unwrap();
        let permit = match ActionCoordinator::preflight(store, ingress).await.unwrap() {
            ActionPreflight::Vacant(permit) => permit,
            _ => panic!("scheduled apply preflight was not vacant"),
        };
        ActionCoordinator::prepare_and_stage_scheduled(permit, world.engine_mut(), &candidate)
            .expect("the apply stages");
        let (bundle, installation) = world
            .engine_mut()
            .take_staged_action()
            .unwrap()
            .expect("a staged apply")
            .into_parts();
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        world
            .engine_mut()
            .install_composed_action(installation)
            .expect("the fixture installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        *bundle
    }

    /// C1 (HIGH), the "does the fallback actually seat" e2e the shipped H tests
    /// never wrote: a clean `+focus/release` fires the declared fallback, the
    /// real `command_offer_submission` + `resolve_and_stage_offer` resolve the
    /// deferred submit against the re-sourced relief offer (it is no longer
    /// self-staled by its own release), and the durable scheduled apply seats the
    /// lease on the declared fallback.
    #[tokio::test]
    async fn vf_c1_a_clean_release_fallback_actually_seats() {
        use mush_core::ObjKind;

        let mut world =
            mush_testkit::TestWorld::with_modules(vec![
                Box::new(mush_sim::SimulationModule::new()),
            ]);
        world
            .engine_mut()
            .set_offer_keys(OfferKeyRing::new(OfferSecret::new(3, [3; 32]).unwrap(), []).unwrap());
        let field = world
            .world_mut()
            .create(ObjKind::Room, "Field", GOD, None)
            .unwrap();
        let atlas = world
            .world_mut()
            .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
            .unwrap();
        let pilot = world.spawn_player("Pilot");
        let backup = world.spawn_player("Backup");
        for (obj, beh) in [(field, "sim.domain"), (atlas, "sim.participant")] {
            world
                .world_mut()
                .attach_behavior(
                    obj,
                    mush_core::BehaviorId::new(beh).unwrap(),
                    mush_core::BehaviorBinding {
                        schema_version: 1,
                        attached_by: GOD,
                        attached_at: 0,
                    },
                )
                .unwrap();
        }
        *world
            .world_mut()
            .module_state_mut::<mush_sim::AccountOracle>()
            .unwrap() = mush_sim::AccountOracle::new(Arc::new(FoldMapOracle(
            std::collections::BTreeMap::from([(
                pilot,
                mush_core::AccountId::mint("PilotAcct", 1).unwrap(),
            )]),
        )));
        let refused = |events: &[(mush_core::Audience, OutputEvent)]| {
            events.iter().any(|(_, event)| {
                matches!(event, OutputEvent::Sys(rich)
                    if rich.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
            })
        };
        for line in [
            format!("@domain/create #{}=sim.passive", field.0),
            format!("@spatial/place/domain #{}=0", atlas.0),
            format!(
                "@spatial/station #{}=pilot,pilot,channels:movement",
                atlas.0
            ),
            format!("@spatial/assign #{}=pilot,#{}", atlas.0, pilot.0),
        ] {
            assert!(
                !refused(&world.run(GOD, &line)),
                "setup line refused: {line}"
            );
        }
        world.world_mut().move_to(pilot, field).unwrap();
        world.world_mut().move_to(backup, field).unwrap();
        let _ = world.run(
            GOD,
            &format!("@station/fallback #{}/pilot=#{}", atlas.0, backup.0),
        );
        let _ = world.run(pilot, &format!("+focus #{}/pilot", atlas.0));
        let _ = world.run(pilot, "+focus/release");
        let fire = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("the clean release fired the fallback controller");

        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let boundary_at = world.world().world_ic_now().at;
        // Resolve + stage the fallback fire through the REAL command-submit path.
        drive_submit(&mut world, &store, &fire, 1)
            .await
            .expect("the fallback fire resolves and stages (not self-staled)");
        // Apply the scheduled handoff at its boundary: the seat FILLS with backup.
        let applied = bundle_lease(&drive_scheduled_apply(&mut world, &store, boundary_at).await);
        assert_eq!(
            applied.holder(),
            Some(&PrincipalId::Object { id: backup }),
            "the declared fallback actually seats through the full driver path"
        );
        assert!(applied.pending().is_none());
    }

    #[tokio::test]
    async fn vf_b2_extract_command_to_receipt_commits_one_lot() {
        let (mut world, families) = extract_world();
        let store = mush_store::InMemoryStore::new();
        // GOD's extraction licence (entitlement instrument + Active account).
        let _ = world.run(GOD, "@seedlicence");
        // GOD's own facility endpoint, and a body whose pool is seeded directly.
        let facility = make_gods_vault(&mut world);
        let body = mush_industry::OreDeposit::new(
            mush_industry::OreDepositId(0x0E7C3),
            ObjId(7),
            CanonicalId::new("metals").unwrap(),
            mush_inventory::Grade::new(60).unwrap(),
            mush_industry::Accessibility::new(100).unwrap(),
            mush_industry::BandId::new("grade_b").unwrap(),
            mush_industry::BandId::new("reserve_major").unwrap(),
            mush_industry::YieldCurve::new(vec![mush_industry::CurvePoint {
                cumulative: 0,
                per_cycle: 500,
            }])
            .unwrap(),
            mush_inventory::EndpointRef(mush_economy::VaultId(90_001)),
            40_000,
        )
        .unwrap();
        let now = world.world().world_ic_now();
        let claim = mush_industry::ClaimRecord::new(
            mush_industry::ClaimId(0x0C1A),
            mush_industry::ClaimSubject::Deposit { deposit: body.id() },
            mush_org::PrincipalId::Object(GOD),
            mush_org::PrincipalId::Object(GOD),
            ObjId(90_099),
            mush_industry::Exclusivity::Shared,
            1,
            mush_core::ClockStamp {
                clock: now.clock,
                at: now.at.saturating_add(1_000_000),
            },
            mush_industry::Transferability::Transferable,
        )
        .unwrap();
        {
            let engine = world.engine_mut();
            apply_industry(engine, &families, |txn| {
                families.open_deposit(txn, body.clone())
            })
            .expect("open the body");
            apply_industry(engine, &families, |txn| {
                families.file_claim(txn, claim.clone())
            })
            .expect("file the claim");
        }

        // @extract/start <body>=<facility>, driven to commit.
        let _ = world.run(GOD, &format!("@extract/start {}={}", body.id(), facility));
        let start_submit = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("@extract/start queued a submission");
        let _ = world.drain_write_batch();
        drive_submit(&mut world, &store, &start_submit, 1)
            .await
            .expect("the start commits");

        // @extract/cycle <body>, driven to commit.
        let _ = world.run(GOD, &format!("@extract/cycle {}", body.id()));
        let cycle_submit = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("@extract/cycle queued a submission");
        let _ = world.drain_write_batch();
        let summary = drive_submit(&mut world, &store, &cycle_submit, 1)
            .await
            .expect("the cycle commits");
        assert!(
            summary.contains("produced 500"),
            "the receipt names the yield: {summary}"
        );

        // Exactly one lot minted, at the OPERATOR's endpoint, and the run advanced.
        {
            let engine = world.engine_mut();
            let markers: Vec<mush_industry::AppliedMarker> = engine
                .action_state()
                .scan_family(&families.applied, None, 64)
                .expect("scan markers")
                .entries()
                .iter()
                .map(|entry| entry.value().clone())
                .collect();
            assert_eq!(
                markers.len(),
                1,
                "the committed bundle wrote exactly one marker"
            );
            assert_eq!(markers[0].deposit(), body.id());
            assert_eq!(markers[0].quantity(), 500);
            assert_eq!(
                markers[0].to(),
                mush_inventory::EndpointRef(mush_economy::VaultId(facility)),
                "the lot landed at the operator's own endpoint"
            );
            assert_eq!(markers[0].actor(), GOD);
            let run = engine
                .action_state()
                .get(&families.runs, &body.id())
                .expect("read run")
                .cloned()
                .expect("the run exists");
            assert_eq!(
                run.next_cycle(),
                1,
                "industry.run advanced exactly one cycle"
            );
        }

        // A retyped-identical submission is a marker HIT: a reboot (fresh epoch)
        // gets past the durable idempotency key, and the applied marker refuses
        // the replay, so nothing commits twice.
        let _ = world.drain_write_batch();
        let replay = drive_submit(&mut world, &store, &cycle_submit, 2).await;
        assert!(
            replay
                .as_ref()
                .err()
                .is_some_and(|error| error.contains("already been applied")),
            "the applied marker is the replay guard: {replay:?}"
        );
        // And still exactly one marker, the run still at cycle 1.
        {
            let engine = world.engine_mut();
            let count = engine
                .action_state()
                .scan_family(&families.applied, None, 64)
                .expect("scan markers")
                .entries()
                .len();
            assert_eq!(count, 1, "the replay committed no second marker");
            let run = engine
                .action_state()
                .get(&families.runs, &body.id())
                .expect("read run")
                .cloned()
                .expect("run");
            assert_eq!(run.next_cycle(), 1, "the run did not advance on the replay");
        }
    }

    // ---- E7c4 independent verify fold: R1 (the terminal-schedule freeze) ----

    /// Before the fold, a completed/cancelled work order left its repeating
    /// production `DurableSchedule` live; the next cadence tick discovered a
    /// candidate, `wo_advance` refused it with a **non-retryable**
    /// `ProviderRejected`, and the durable-schedule driver turned that into
    /// `apply_integrity_freeze` — a whole-world freeze that a reboot re-triggered.
    /// The terminal transition now tears the schedule down, and the adapter's
    /// `plan` refuses a terminal order's surviving schedule as **Stale** — which
    /// the driver DISCARDS. This asserts the classification the freeze fired on no
    /// longer arises: planning a terminal order's orphan schedule is a Stale plan
    /// refusal, not a hard prepare error.
    #[test]
    fn vf_r1_a_terminal_schedule_does_not_freeze_the_world() {
        let (mut world, families) = extract_world();
        let engine = world.engine_mut();
        let order = mush_industry::WorkOrderId(0x1234);
        let owner = mush_core::OwnerId::new("industry").unwrap();
        let local = CanonicalId::new(format!("production.{:016x}", order.0)).unwrap();
        let sid = mush_core::ScheduleId::derive(&owner, &local).unwrap();
        let ic = engine.world.world_ic_now();
        let sch = mush_core::DurableSchedule::new(mush_core::DurableScheduleDraft {
            id: sid,
            owner: owner.clone(),
            clock: ic.clock.clone(),
            next_due: mush_core::ClockStamp {
                clock: ic.clock.clone(),
                at: ic.at.saturating_sub(1),
            },
            cadence: Some(mush_industry::workorder::PRODUCTION_CADENCE_SECS),
            generation: 1,
            next_ordinal: 0,
            restart: mush_core::ScheduleRestartPolicy::Resume,
            catch_up_order: mush_core::ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: mush_industry::workorder::PRODUCTION_MAX_CATCH_UP,
        })
        .unwrap();
        let job = mush_industry::ProductionJob::new(
            sid,
            order,
            GOD,
            mush_industry::workorder::PRODUCTION_UNITS_PER_OCCURRENCE,
            1,
        )
        .unwrap();
        let recipe = mush_industry::RecipeRef::new(
            CanonicalId::new("tacsim.core").unwrap(),
            1,
            CanonicalId::new("alloy_smelt").unwrap(),
            mush_core::DefinitionDigest::from_bytes([0x11; 32]),
        )
        .unwrap();
        let pinned = mush_industry::PinnedRecipe::new(
            vec![mush_industry::RecipeLine::new(CanonicalId::new("metals").unwrap(), 4).unwrap()],
            vec![
                mush_industry::RecipeLine::new(CanonicalId::new("structural_alloys").unwrap(), 1)
                    .unwrap(),
            ],
            std::collections::BTreeSet::new(),
            3,
        )
        .unwrap();
        // A terminal (Completed) order whose schedule survived — the historical
        // orphan the terminal-bundle teardown never reached.
        let done = mush_industry::WorkOrder::draft(
            order,
            mush_org::PrincipalId::Object(GOD),
            mush_org::PrincipalId::Object(GOD),
            ObjId(7),
            2,
            1,
            1,
            recipe,
            pinned,
            1,
            mush_economy::AccountId(1),
            Some(mush_inventory::EndpointRef(mush_economy::VaultId(500))),
            mush_industry::DeliveryTarget::Endpoint {
                endpoint: mush_inventory::EndpointRef(mush_economy::VaultId(600)),
            },
            mush_economy::EconomicVisibility::Parties,
        )
        .unwrap()
        .with_state(mush_industry::WorkOrderState::Completed {
            commit: mush_core::CommitToken::from_bytes([9; 16]),
        })
        .unwrap();
        apply_industry(engine, &families, |txn| {
            families.open_work_order(txn, done)?;
            txn.insert(&families.schedules, &sid, sch)
                .map_err(mush_industry::IndustryError::from)?;
            txn.insert(&families.jobs, &sid, job)
                .map_err(mush_industry::IndustryError::from)?;
            Ok(())
        })
        .unwrap();
        // Discovery still finds the due schedule (discover reads only the cursor)…
        let page = engine
            .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
            .expect("discovery does not freeze");
        assert!(
            !page.candidates().is_empty(),
            "the orphan schedule is still discovered"
        );
        // …but planning it refuses as Stale (the adapter sees the terminal order),
        // which `scheduled_prepare_stale` classifies as DiscardedStale — the world
        // is not frozen.
        let candidate = page.candidates()[0].clone();
        let ingress =
            mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 0)
                .expect("ingress");
        match engine.plan_scheduled_action(&candidate, &ingress) {
            Err(mush_core::ScheduleActionPlanError::Adapter { source, .. }) => {
                assert_eq!(
                    source.code(),
                    mush_core::ActionErrorCode::Stale,
                    "a terminal order's schedule plans as Stale (discarded), never a freeze"
                );
            }
            other => panic!("expected a Stale plan refusal, got {other:?}"),
        }
    }

    /// E7c2 fold C4: the fan-out arm voided the ENGINE-wide slot, so a
    /// legitimate submission queued in an earlier step — possibly a different
    /// actor's — was silently destroyed and its owner was told nothing.
    #[tokio::test]
    async fn vf_c4_a_fan_out_never_voids_a_submission_from_an_earlier_step() {
        let mut world = b1_world();
        let _ = b1_run(&mut world, "@b1submit 3");
        assert!(world.engine().has_authoritative_submit());
        let out = b1_run(&mut world, "@b1submit/twice 7");
        assert!(
            world.engine().has_authoritative_submit(),
            "the incumbent from the earlier step survives the fan-out"
        );
        assert!(
            b1_lines(&out)
                .iter()
                .any(|line| line.contains("still committing")),
            "{:?}",
            b1_lines(&out)
        );
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        let CommandSubmitDrive::Staged { staged, .. } =
            driver.drive_one(&mut engine, &store).await.unwrap()
        else {
            panic!("the incumbent still drives")
        };
        assert_eq!(
            staged.summary.as_deref(),
            Some("The counter stands at 3."),
            "and it is the FIRST submission, not the fan-out's"
        );
    }

    /// D3 as taken: the slot is per actor, so a fan-out drops only the fanning
    /// actor's own — and it tells them. Nobody else's submission is touched and
    /// nobody else is told anything at all.
    #[tokio::test]
    async fn vf_c4_a_fan_out_tells_every_actor_it_dropped() {
        let mut world = b1_world();
        let bystander = world.spawn_player("bystander");
        let _ = b1_run_as(&mut world, bystander, "@b1submit 5");
        assert!(world.engine().has_authoritative_submit_for(bystander));
        let out = b1_run(&mut world, "@b1submit/twice 9");
        let dropped: Vec<String> = b1_lines(&out)
            .into_iter()
            .filter(|line| line.contains("more than one authoritative action"))
            .collect();
        assert_eq!(dropped.len(), 1, "one line per dropped actor: {dropped:?}");
        assert!(
            !world.engine().has_authoritative_submit_for(GOD),
            "the fanning actor stages nothing at all"
        );
        assert!(
            world.engine().has_authoritative_submit_for(bystander),
            "and the bystander's submission is untouched"
        );
    }

    /// E7c2 fold B7: one player's queued submission used to answer every other
    /// player's mutation with "An earlier action is still committing" — a
    /// cross-actor denial and a cross-actor tell. The durable one-intent rule
    /// (D-E7-39 / 08i answer 6) is about the STORE and is enforced downstream.
    #[tokio::test]
    async fn vf_b7_one_actors_submission_does_not_block_another() {
        let mut world = b1_world();
        let alice = world.spawn_player("alice");
        let bob = world.spawn_player("bob");
        let alice_out = b1_run_as(&mut world, alice, "@b1submit 3");
        let bob_out = b1_run_as(&mut world, bob, "@b1submit 4");
        assert!(
            !b1_lines(&bob_out)
                .iter()
                .any(|line| line.contains("still committing")),
            "bob is not refused because alice got there first: {:?}",
            b1_lines(&bob_out)
        );
        assert!(world.engine().has_authoritative_submit_for(alice));
        assert!(world.engine().has_authoritative_submit_for(bob));
        assert!(!b1_lines(&alice_out).iter().any(|line| line.contains("bob")));
        // Bob's OWN second submission, while his first is queued, is still
        // back-pressure — and the refusal names nobody else.
        let again = b1_run_as(&mut world, bob, "@b1submit 5");
        let refusal: Vec<String> = b1_lines(&again)
            .into_iter()
            .filter(|line| line.contains("still committing"))
            .collect();
        assert_eq!(refusal.len(), 1, "{refusal:?}");
        assert!(refusal[0].starts_with("Your earlier action"), "{refusal:?}");
        // Both drive, in successive steps.
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        for _ in 0..2 {
            match driver.drive_one(&mut engine, &store).await.unwrap() {
                CommandSubmitDrive::Staged { .. } => {}
                CommandSubmitDrive::Refused { detail, .. } => panic!("refused: {detail}"),
                _ => panic!("expected a staged submission"),
            }
            let (_, installation) = engine
                .take_staged_action()
                .unwrap()
                .expect("staged action")
                .into_parts();
            engine
                .abort_prepared_action(installation)
                .expect("the fixture aborts");
        }
        assert!(!engine.has_authoritative_submit());
    }

    /// E7c2 fold R8: a quarantined durable identity on the command lane used to
    /// be indistinguishable in the log from a typo. The actor's line is
    /// unchanged — that is the disclosure rule — but the drive result is not.
    #[tokio::test]
    async fn vf_r8_a_quarantined_preflight_is_an_integrity_event() {
        let mut world = b1_world();
        let _ = b1_run(&mut world, "@b1submit 3");
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await.unwrap(),
            CommandSubmitDrive::Staged { .. }
        ));
        let (bundle, _installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .discard_quarantined_authoritative_intent(
                bundle.token(),
                bundle.bundle_hash(),
                bundle.bundle_hash(),
            )
            .await
            .expect("quarantine the intent");
        // The identical command again: preflight now answers `Quarantined`.
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@b1submit 3".to_owned(),
            },
            &mut out,
            &mut fx,
        );
        match driver.drive_one(&mut engine, &store).await.unwrap() {
            CommandSubmitDrive::Refused {
                integrity, detail, ..
            } => {
                assert!(integrity, "a quarantine is an operator event: {detail}");
                assert!(detail.contains("quarantined"), "{detail}");
            }
            _ => panic!("expected an integrity refusal"),
        }
    }

    /// E7c2 fold R11: acceptance and loss used to be indistinguishable at the
    /// terminal — an `Authoritative` handler emits nothing and the only later
    /// line is the driver's, so a submission dropped by a crash or parked by a
    /// freeze produced no output whatever.
    #[tokio::test]
    async fn vf_r11_a_queued_submission_acknowledges_receipt_without_claiming_a_mutation() {
        let mut world = b1_world();
        let out = b1_run(&mut world, "@b1submit 3");
        assert_eq!(
            b1_lines(&out),
            vec!["Queued.".to_owned()],
            "exactly one non-committal line"
        );
        assert_eq!(
            out.classes(),
            [mush_core::DeliveryClass::BestEffort],
            "BestEffort, so the durable-ack gate never withholds it and it can \
             never be read as the confirmation"
        );
        assert!(world.engine().has_authoritative_submit());
    }

    /// E7c2 fold B12: the first commit rendered `deterministic_commit_token`'s
    /// internal binding digest raw into player output while the replay of the
    /// same act got the provider's own sentence. Both now read the same.
    #[tokio::test]
    async fn vf_b12_the_first_commit_and_the_hit_read_the_same() {
        let mut world = b1_world();
        let _ = b1_run(&mut world, "@b1submit 4");
        let store = mush_store::InMemoryStore::new();
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        let mut driver = CommandSubmitDriver::new(1);
        let CommandSubmitDrive::Staged { staged, .. } =
            driver.drive_one(&mut engine, &store).await.unwrap()
        else {
            panic!("staged")
        };
        let fresh = staged.summary.clone().expect("the provider's own summary");
        assert!(
            !fresh.contains(&staged.token.to_string()),
            "no raw commit token in player output: {fresh}"
        );
        let (bundle, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        store
            .reserve_authoritative_intent(*bundle.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        store
            .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
            .await
            .unwrap();
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@b1submit 4".to_owned(),
            },
            &mut out,
            &mut fx,
        );
        let CommandSubmitDrive::AlreadyFinished {
            actor,
            receipt,
            status,
        } = driver.drive_one(&mut engine, &store).await.unwrap()
        else {
            panic!("the retyped identical command is a HIT")
        };
        let projections = engine
            .registry
            .action_providers()
            .map(mush_core::ActionProviderRegistry::receipt_projection_table)
            .expect("projections");
        let hit = ActionCoordinator::project_finalized_receipt(
            &projections,
            &PrincipalId::Object { id: actor },
            status,
            &receipt,
        )
        .expect("projected")
        .result()
        .map(|result| result.summary().to_owned())
        .expect("summary");
        assert_eq!(fresh, hit, "the first commit and its HIT read the same");
    }

    /// The seam must not fork a second spelling of the five steps. Both
    /// ingresses call `resolve_and_stage_offer`, and it is the only site in this
    /// file that calls `prepare_and_stage_resolved`.
    #[test]
    fn e7c2_b1_both_ingresses_call_one_five_step_function() {
        // Only the production half of this file; the assertions below name the
        // very symbols they count.
        let source = include_str!("world_task.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a production half");
        let calls = source.matches("resolve_and_stage_offer(").count();
        assert!(
            calls >= 3,
            "expected the definition plus both call sites, found {calls}"
        );
        let staged = source
            .matches("ActionCoordinator::prepare_and_stage_resolved(")
            .count();
        assert_eq!(
            staged, 1,
            "prepare_and_stage_resolved must be reached through exactly one function"
        );
    }

    // ---- H2a: the production decision composition ----------------------

    /// A provider that always picks the prompt's `idle` offer, as a model
    /// would by calling the idle tool.
    struct IdleChoosingProvider {
        owner: OwnerId,
        calls: Arc<AtomicUsize>,
    }

    impl DecisionProviderAdapter for IdleChoosingProvider {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            lease: DecisionWorkerLease,
        ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
        {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            let prompt = lease.prompt().clone();
            Box::pin(async move {
                let idle = prompt
                    .actions
                    .iter()
                    .find(|a| a.presentation().verb == mush_npc::IDLE_VERB)
                    .ok_or_else(|| {
                        DecisionProviderFailure::new(
                            DecisionProviderFailureKind::Malformed,
                            "no idle offer",
                        )
                    })?;
                Ok(DecisionResponse {
                    token: prompt.token,
                    offer: idle.offer(),
                    targets: Vec::new(),
                    parameters: serde_json::json!({}),
                })
            })
        }
    }

    /// A world composed with the real `npc` module, keyed, with one NPC agent
    /// and one decision request queued for the tick — the capture already
    /// taken by the engine, ready for the loop.
    fn npc_decision_world(deadline_secs: i64, lease_secs: i64) -> (Engine, mush_core::ObjId) {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            mush_npc::NpcModule::new().with_decision_lease_secs(lease_secs),
        )]);
        tw.engine_mut().set_decision_keys(
            DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
        );
        tw.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap(),
        );
        let events = tw.run(GOD, "@agent/create Bob");
        let text: String = events
            .iter()
            .map(|(_, e)| mush_testkit::text_of(e))
            .collect::<Vec<_>>()
            .join("\n");
        let at = text.find("(#").expect("the create line names the id");
        let digits: String = text[at + 2..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let npc = mush_core::ObjId(digits.parse().unwrap());
        // The NPC object stays dirty: the world task persists it on its
        // first iteration, before it drives the capture, so a second boot
        // from the store alone still has the agent to project offers for.
        tw.world_mut()
            .module_state_mut::<mush_npc::NpcHostState>()
            .unwrap()
            .request_decision(mush_npc::NpcDecisionRequest {
                npc,
                cause_kind: CanonicalId::new("npc.test_turn").unwrap(),
                instance: IdempotencyKeyText::parse_canonical("turn-1").unwrap(),
                replaceable_key: None,
                context: serde_json::json!({ "room": "Limbo" }),
                deadline_secs,
                key: IdempotencyKeyText::parse_canonical("turn-1").unwrap(),
            });
        let _ = tw.fire_due(5_000);
        assert!(
            tw.engine().has_decision_capture(),
            "the tick issued no capture"
        );
        (tw.into_engine(), npc)
    }

    fn receipt_key(npc: mush_core::ObjId) -> String {
        format!("action-state/v1/npc_turn_receipts/npc_{}", npc.0)
    }

    async fn wait_for_receipt(
        store: &Arc<dyn ObjectStore>,
        npc: mush_core::ObjId,
        secs: u64,
    ) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(secs), async {
            loop {
                if let Some(value) = store
                    .module_json_get("npc", &receipt_key(npc))
                    .await
                    .expect("receipt read")
                {
                    break value;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the decision did not execute its action")
    }

    /// The whole production lifecycle through the production ingress: a
    /// queued request, the tick's capture, the loop builds, arms and submits
    /// `npc_decide`, the wake, the lease, the provider, the response, the
    /// coordinator executes `npc_idle`, and its turn receipt lands.
    #[tokio::test]
    async fn h2a_a_captured_npc_decision_runs_to_its_executed_action() {
        let (engine, npc) = npc_decision_world(60, 60);
        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(IdleChoosingProvider {
                owner: OwnerId::new("npc").unwrap(),
                calls: Arc::clone(&calls),
            }))
            .unwrap();
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            deps,
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        let receipt = wait_for_receipt(&store, npc, 5).await;
        assert_eq!(receipt["value"]["outcome"], "idle");
        assert_eq!(receipt["value"]["npc"], npc.0);
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1, "one lease, one call");
        // The pending record reached terminal: nothing is left to dispatch.
        let listed = store
            .module_json_list("npc")
            .await
            .expect("npc namespace listing");
        assert!(
            listed
                .iter()
                .any(|(key, value)| key.contains("npc_pending_decisions")
                    && value["value"]["status"]["state"] == "terminal"
                    && value["value"]["status"]["outcome"] == "executed"),
            "no terminal executed decision: {:?}",
            listed.iter().map(|(k, _)| k).collect::<Vec<_>>()
        );
        stop_test_server(handles).await;
    }

    /// With no LLM service the production adapter answers `Unavailable`
    /// every time, and the deadline hands the decision to `npc_idle`.
    #[tokio::test]
    async fn h2a_an_unavailable_provider_falls_back_to_idle_at_the_deadline() {
        let (engine, npc) = npc_decision_world(1, 60);
        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        let providers = production_decision_providers(&engine, None).unwrap();
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            deps,
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        let receipt = wait_for_receipt(&store, npc, 20).await;
        assert_eq!(receipt["value"]["outcome"], "idle");
        let listed = store.module_json_list("npc").await.unwrap();
        assert!(
            listed
                .iter()
                .any(|(key, value)| key.contains("npc_pending_decisions")
                    && value["value"]["status"]["state"] == "terminal"
                    && value["value"]["status"]["fallback_reason"].is_string()),
            "the terminal record does not name its fallback"
        );
        stop_test_server(handles).await;
    }

    /// A restart between the capture and the lease resumes from the pending
    /// family (K7.4): the second boot finds the record and finishes it.
    #[tokio::test]
    async fn h2a_a_restart_between_capture_and_lease_resumes_the_decision() {
        // A one-second lease, as the kernel's own restart test uses: the first
        // boot's abandoned lease must expire for the second to re-lease.
        let (engine, npc) = npc_decision_world(60, 1);
        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        // First boot: the capture commits; the provider can never answer.
        let providers = production_decision_providers(&engine, None).unwrap();
        let first = spawn_persist_with_gateway_and_decisions(
            engine,
            deps,
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let listed = store.module_json_list("npc").await.unwrap();
                if listed
                    .iter()
                    .any(|(key, _)| key.contains("npc_pending_decisions"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the capture did not commit");
        stop_test_server(first).await;
        assert!(
            store
                .module_json_get("npc", &receipt_key(npc))
                .await
                .unwrap()
                .is_none(),
            "nothing executed before the restart"
        );

        // Second boot from the store alone, with a provider that answers. Its
        // OWN lease is long: the abandoned lease expires on the first boot's
        // one-second terms whatever this boot would grant, and a one-second
        // lease here let a starved box (the workspace gate beside other
        // builds) expire the re-lease before the answer was applied and call
        // the provider a second time — the exactly-once assertion below is
        // about the restart, not about the box's scheduling.
        let world = crate::persist::load_or_seed_world(&*store).await.unwrap();
        let mut engine = Engine::with_modules(
            world,
            mush_core::Registry::new(),
            vec![Box::new(
                mush_npc::NpcModule::new().with_decision_lease_secs(60),
            )],
        )
        .unwrap();
        engine.set_decision_keys(
            DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
        );
        engine.set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap(),
        );
        crate::modules::boot_modules(&mut engine, &*store)
            .await
            .unwrap();
        assert_eq!(
            decision_census(&engine).nonterminal(),
            1,
            "the pending decision came back on boot"
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(IdleChoosingProvider {
                owner: OwnerId::new("npc").unwrap(),
                calls: Arc::clone(&calls),
            }))
            .unwrap();
        let second = spawn_persist_with_gateway_and_decisions(
            engine,
            decision_test_deps(Arc::clone(&store)),
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .unwrap();
        let receipt = wait_for_receipt(&store, npc, 10).await;
        assert_eq!(receipt["value"]["outcome"], "idle");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        stop_test_server(second).await;
    }

    /// The production spawn: a keyed composition starts the decision runtime
    /// over the LLM adapter; an unkeyed one starts the plain runtime; a
    /// worker limit past the ceiling refuses.
    #[tokio::test]
    async fn h2a_the_production_spawn_composes_the_npc_adapter() {
        let (engine, _) = npc_decision_world(60, 60);
        let providers = production_decision_providers(&engine, None).unwrap();
        assert!(providers.contains_owner(&OwnerId::new("npc").unwrap()));
        let handles = spawn_persist_with_gateway_production(
            engine,
            ServerDeps::in_memory(),
            None,
            DEFAULT_DECISION_PROVIDER_WORKERS,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("production spawn");
        stop_test_server(handles).await;

        let tw = mush_testkit::TestWorld::with_modules(vec![Box::new(mush_npc::NpcModule::new())]);
        let handles = spawn_persist_with_gateway_production(
            tw.into_engine(),
            ServerDeps::in_memory(),
            None,
            DEFAULT_DECISION_PROVIDER_WORKERS,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("an unkeyed composition still spawns");
        stop_test_server(handles).await;

        let (engine, _) = npc_decision_world(60, 60);
        let refused = spawn_persist_with_gateway_production(
            engine,
            ServerDeps::in_memory(),
            None,
            mush_core::MAX_DECISION_PROVIDER_WORKERS + 1,
            |_| Arc::new(BlackholeGateway),
        );
        assert!(refused.is_err(), "33 workers must refuse");
    }

    // ---- H2b: an NPC's scene turn as a durable decision ---------------

    /// `+timestop/next` hands the turn to an NPC; the NPC module requests a
    /// decision; the tick captures it; the loop runs it to `npc_idle` through
    /// the coordinator; and the scene stays exactly where it was — Active,
    /// the NPC still holding the turn — because nothing auto-advances.
    #[tokio::test]
    async fn h2b_an_npc_scene_turn_runs_to_its_action_and_leaves_the_scene_in_place() {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_scenes::ScenesModule::new()),
            Box::new(mush_npc::NpcModule::new()),
        ]);
        tw.engine_mut().set_decision_keys(
            DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
        );
        tw.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap(),
        );
        let room = tw
            .world_mut()
            .create(mush_core::ObjKind::Room, "The Bar", GOD, None)
            .unwrap();
        tw.world_mut().move_to(GOD, room).unwrap();
        let player = tw.spawn_player("Pat");
        tw.world_mut().move_to(player, room).unwrap();
        tw.world_mut().set_connected(player, true);
        tw.world_mut().set_live_conns(player, 1);
        let created: String = tw
            .run(GOD, "@agent/create Bob")
            .iter()
            .map(|(_, e)| mush_testkit::text_of(e))
            .collect::<Vec<_>>()
            .join("\n");
        let at = created.find("(#").unwrap();
        let digits: String = created[at + 2..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let npc = mush_core::ObjId(digits.parse().unwrap());
        let _ = tw.run(GOD, "+timestop");
        // Round 1 opens on Pat; the second advance hands the turn to Bob.
        let _ = tw.run(GOD, "+timestop/next");
        let _ = tw.run(GOD, "+timestop/next");
        let queued = tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .decision_requests
            .len();
        assert_eq!(queued, 1, "the NPC's turn requested one decision");
        let _ = tw.fire_due(5_000);
        assert!(tw.engine().has_decision_capture());
        let engine = tw.into_engine();

        let deps = ServerDeps::in_memory();
        let store = Arc::clone(&deps.store);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut providers = DecisionProviderRegistry::new();
        providers
            .register(Arc::new(IdleChoosingProvider {
                owner: OwnerId::new("npc").unwrap(),
                calls: Arc::clone(&calls),
            }))
            .unwrap();
        let handles = spawn_persist_with_gateway_and_decisions(
            engine,
            deps,
            DurableOutboxRegistry::new(),
            providers,
            1,
            |_| Arc::new(BlackholeGateway),
        )
        .expect("decision runtime startup");
        let receipt = wait_for_receipt(&store, npc, 5).await;
        assert_eq!(receipt["value"]["outcome"], "idle");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        // The scene did not move: still active, the NPC still on turn.
        let scenes = store.module_json_list("scenes").await.unwrap();
        let record = scenes
            .iter()
            .find(|(key, value)| key.starts_with("scene:") && value.get("participants").is_some())
            .map(|(_, value)| value.clone())
            .expect("the persisted scene record");
        assert_eq!(record["status"], "active", "{record}");
        let participants = record["participants"].as_array().unwrap();
        let turn_index = record["turn_index"].as_u64().unwrap() as usize;
        assert_eq!(participants[turn_index], npc.0, "{record}");
        stop_test_server(handles).await;
    }

    /// # E7 cut 8 independent verify fold — D7(a): the pulse, driven
    ///
    /// §3.8's deviation (e) recorded "`DurableScheduleDriver::drive_one` is a
    /// private `async fn` of `world_task.rs` with no test seam" and deferred the
    /// end-to-end tick replay and the slot-hold figure on that basis. The
    /// premise is false **across crates only**: this very module already drives
    /// `drive_one` a dozen times over `mush_store::InMemoryStore`, and
    /// `mush-sim` is an ordinary (non-dev) dependency of `mush-server` — so
    /// composing `SimulationModule` instead of the toy owner is the only new
    /// thing here. No new public API, no `#[ignore]`, no `*_gated` test and no
    /// CI job: `ci_gated_test_inventory` is untouched.
    ///
    /// Three of the fold's HIGHs (T1, T2, T3) were invisible to every test cut 8
    /// wrote for exactly one reason: none of them advanced the clock across two
    /// consecutive **driven** ticks. These do.
    mod e7c8_pulse {
        use super::*;
        use mush_core::{
            ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, CanonicalId,
            CommitToken, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef,
            IdempotencyKey, IdempotencyScope, ObjId, OwnerId, PerceivedRef, PrincipalId,
            RequestUid, ScopedIdempotencyKey,
        };
        use mush_sim::tick::{
            DomainTickCommit, MAX_RETAINED_TICK_COMMITS as RETAINED, SCHEDULE_SOURCE,
            TickCommitKey, TickHandles, pulse_clock, tick_schedule_id,
        };
        use mush_sim::{DomainId, SimulationModule, SpatialState};
        use serde_json::json;

        /// The world every row here starts from: one domain host, promoted, its
        /// activation applied, with the durable cursor armed through the REAL
        /// `domain_pulse_arm` authority.
        fn pulsing_world() -> (mush_core::Engine, mush_store::InMemoryStore, i64) {
            let mut world =
                mush_testkit::TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
            world.set_now(1_000);
            let host = world
                .world_mut()
                .create(mush_core::ObjKind::Room, "Landing Field", GOD, None)
                .unwrap();
            world
                .world_mut()
                .attach_behavior(
                    host,
                    mush_core::BehaviorId::new("sim.domain").unwrap(),
                    mush_core::BehaviorBinding {
                        schema_version: 1,
                        attached_by: GOD,
                        attached_at: 0,
                    },
                )
                .unwrap();
            let _ = world.run(GOD, &format!("@domain/create #{}=sim.passive", host.0));
            let _ = world.run(GOD, &format!("@domain/promote #{}=contact", host.0));
            let _ = world.run(GOD, &format!("@domain/tick #{}", host.0));
            let submit = *world
                .engine_mut()
                .take_authoritative_submit()
                .expect("@domain/tick stages the arm from the command handler");
            let _ = world.drain_write_batch();
            let mut engine = world.into_engine();
            // The production composition installs both key rings; the tick
            // resolves with recorded randomness and its window PINS the version
            // it draws under (fold T11).
            engine.set_resolution_keys(
                mush_core::ResolutionKeyRing::new(
                    mush_core::ResolutionSecret::new(3, [7; 32]).unwrap(),
                    [],
                )
                .unwrap(),
            );
            arm_directly(&mut engine, &submit);
            let store = mush_store::InMemoryStore::new();
            (engine, store, 1_000)
        }

        /// Apply the queued `domain_pulse_arm` submission straight through the
        /// registered provider. The staff gate is the fixed `armed_by`
        /// parameter, which the submission already carries by construction.
        fn arm_directly(engine: &mut mush_core::Engine, submit: &mush_core::AuthoritativeSubmit) {
            let mut parameters = submit.parameters.clone();
            parameters["armed_by"] = json!(submit.actor);
            let invocation = invocation_of(&submit.action, submit.actor, parameters, 21);
            let (prepared, _) = engine
                .prepare_registered_authoritative_action(
                    &submit.action,
                    &invocation,
                    CommitToken::from_bytes([21; 16]),
                )
                .expect("the pulse arm prepares");
            let (_ops, installation) = prepared.into_parts();
            engine
                .install_composed_action(installation)
                .expect("the fixture installs");
        }

        fn invocation_of(
            action: &DefinitionRef,
            actor: ObjId,
            parameters: serde_json::Value,
            token: u8,
        ) -> ActionInvocation {
            let principal = PrincipalId::Object { id: actor };
            ActionInvocation::new(ActionInvocationDraft {
                request: RequestUid::from_bytes([token; 16]),
                principal: principal.clone(),
                actor,
                offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
                subject: PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters,
                idempotency: ScopedIdempotencyKey::new(
                    IdempotencyScope {
                        principal,
                        owner: action.key().owner.clone(),
                        namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
                    },
                    1,
                    IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
                ),
            })
            .expect("a well-formed invocation")
        }

        /// Drive until a tick stages, completing it exactly as `world_task`
        /// does: reserve, reconcile, install.
        ///
        /// The driver's page cursor walks one record per call and resets when
        /// the page comes back empty, so a due occurrence takes at most a
        /// couple of polls to surface — exactly as the 250 ms world loop
        /// experiences it. Every intermediate outcome is asserted **not** to be
        /// an `Err`, because an `Err` here is `apply_integrity_freeze`.
        async fn drive_until_staged(
            driver: &mut DurableScheduleDriver,
            engine: &mut mush_core::Engine,
            store: &mush_store::InMemoryStore,
        ) -> Result<&'static str, String> {
            let mut last = "idle";
            for _ in 0..4 {
                last = drive_and_apply(driver, engine, store).await?;
                if last == "staged" {
                    return Ok(last);
                }
            }
            Ok(last)
        }

        /// Drive one scheduled action and, when it stages, complete it exactly
        /// as `world_task` does: reserve, reconcile, install.
        async fn drive_and_apply(
            driver: &mut DurableScheduleDriver,
            engine: &mut mush_core::Engine,
            store: &mush_store::InMemoryStore,
        ) -> Result<&'static str, String> {
            let outcome = driver.drive_one(engine, store).await?;
            Ok(match outcome {
                DurableScheduleDrive::Idle => "idle",
                DurableScheduleDrive::DeferredByLegacyPredecessor => "deferred",
                DurableScheduleDrive::DiscardedStale(_) => "stale",
                DurableScheduleDrive::Transient(_) => "transient",
                DurableScheduleDrive::Staged(_) => {
                    let staged = engine
                        .take_staged_action()
                        .expect("the staged tick is admitted")
                        .expect("a staged tick action");
                    let (bundle, installation) = staged.into_parts();
                    store
                        .reserve_authoritative_intent((*bundle).clone())
                        .await
                        .expect("reserve");
                    store
                        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                        .await
                        .expect("reconcile");
                    engine
                        .install_composed_action(installation)
                        .expect("the fixture installs");
                    "staged"
                }
            })
        }

        fn handles(engine: &mush_core::Engine) -> TickHandles {
            engine
                .world
                .module_state::<TickHandles>()
                .expect("the pulse registers its read handles")
                .clone()
        }

        fn generation(engine: &mush_core::Engine) -> u64 {
            engine
                .world
                .module_state::<SpatialState>()
                .and_then(|state| state.domains.get(&DomainId(0)).cloned())
                .and_then(|record| record.clock.generation())
                .expect("a pulsing domain")
        }

        fn commit_at(engine: &mush_core::Engine, tick: i64) -> Option<DomainTickCommit> {
            let handles = handles(engine);
            engine
                .action_state()
                .get(
                    &handles.commits,
                    &TickCommitKey {
                        domain: DomainId(0),
                        generation: generation(engine),
                        tick,
                    },
                )
                .ok()
                .flatten()
                .cloned()
        }

        /// **T1 (HIGH), the whole point of this module.** A promoted domain's
        /// SECOND consecutive tick **stages**; before the fold it returned
        /// `Err`, which `world_task.rs:4127` turns into `apply_integrity_freeze`
        /// — a promoted domain resolved exactly one tick and then wedged the
        /// server. The cause: the maintenance pass re-armed the live cursor on
        /// every fire and `PulseArmProvider` reset it to
        /// `(generation 1, ordinal 0, revision 1)`, so `advance_key` — which
        /// hashes `{schedule, owner, generation, next_ordinal, revision}` and
        /// **not** `next_due` — minted the ingress identity the previous tick
        /// had already finalized.
        #[tokio::test]
        async fn vf_t1a_a_second_consecutive_tick_stages_rather_than_freezing_the_world() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            // Tick one.
            engine.world.set_now(at + 10);
            let first = drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("the first tick drives");
            assert_eq!(first, "staged", "the first tick must stage");
            // Tick two, and tick three: the domain clock has moved on, so a
            // fresh occurrence is due each time.
            engine.world.set_now(at + 20);
            let second = drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("the SECOND tick must not return Err: that is the freeze");
            assert_eq!(
                second, "staged",
                "the second consecutive tick must stage, not freeze the world"
            );
            // And a third, so the property is not an artefact of two.
            engine.world.set_now(at + 30);
            let third = drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("the third tick drives");
            assert_eq!(third, "staged");
            // Two distinct commits exist, at consecutive ticks.
            let ticks: Vec<i64> = (0..20)
                .filter(|t| commit_at(&engine, *t).is_some())
                .collect();
            assert!(
                ticks.len() >= 2,
                "three driven ticks must leave more than one commit; got {ticks:?}"
            );
        }

        /// **T1(b).** Re-arming a live cursor leaves its ordinal and revision
        /// untouched — the authority is idempotent in TRUTH, not merely under
        /// its key.
        #[tokio::test]
        async fn vf_t1b_a_rearm_of_a_live_cursor_leaves_its_ordinal_and_revision_untouched() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            engine.world.set_now(at + 10);
            drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("a tick");
            let owner = OwnerId::new("sim").unwrap();
            let id = tick_schedule_id(&owner, DomainId(0)).expect("a cursor id");
            let before = engine
                .action_state()
                .get(&handles(&engine).schedules, &id)
                .expect("read")
                .expect("a live cursor")
                .clone();
            // Re-arm it at the same generation, exactly as the maintenance pass
            // would if it were still pushing every live domain onto `armed`.
            let action = engine
                .registry
                .definitions()
                .and_then(|definitions| {
                    definitions
                        .current(&DefinitionKey {
                            owner: owner.clone(),
                            kind: DefinitionKind::new("action").unwrap(),
                            id: DefinitionId::new("domain_pulse_arm").unwrap(),
                        })
                        .cloned()
                })
                .expect("the arm action is composed");
            let document = json!({
                "domain": 0,
                "generation": generation(&engine),
                "from_tick": 0,
                "arm": true,
                "clear_units": [],
            });
            let invocation = invocation_of(
                &action,
                GOD,
                json!({
                    "arm": serde_json::to_string(&document).unwrap(),
                    "armed_by": json!(GOD),
                }),
                44,
            );
            let (prepared, _) = engine
                .prepare_registered_authoritative_action(
                    &action,
                    &invocation,
                    CommitToken::from_bytes([44; 16]),
                )
                .expect("the re-arm prepares");
            let (_ops, installation) = prepared.into_parts();
            engine
                .install_composed_action(installation)
                .expect("the fixture installs");
            let after = engine
                .action_state()
                .get(&handles(&engine).schedules, &id)
                .expect("read")
                .expect("the cursor survives")
                .clone();
            assert_eq!(
                (
                    before.next_ordinal(),
                    before.revision(),
                    before.next_due().clone()
                ),
                (
                    after.next_ordinal(),
                    after.revision(),
                    after.next_due().clone()
                ),
                "a re-arm at the same generation must change NOTHING about the cursor"
            );
        }

        /// **The deferred replay row** (§3.8 deviation (e)). Drive a tick, drop
        /// the staged bundle, drive it again: the same `state_hash`. The RNG
        /// stream does not re-draw, because the window pins the resolution
        /// secret version the tick draws under (fold T11).
        #[tokio::test]
        async fn vf_d7b_a_dropped_staged_tick_redrives_to_the_same_state_hash() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            engine.world.set_now(at + 10);
            // First attempt: stage it and then DROP it, as a crash between
            // staging and reconciliation does.
            let outcome = driver.drive_one(&mut engine, &store).await.expect("drives");
            let DurableScheduleDrive::Staged(_) = outcome else {
                panic!("the tick must stage")
            };
            let staged = engine
                .take_staged_action()
                .expect("admitted")
                .expect("a staged tick");
            let (first, installation) = staged.into_parts();
            let first_hash = first.bundle_hash();
            engine
                .abort_prepared_action(installation)
                .expect("the fixture aborts");
            // Second attempt over the same clock: the same bundle hash.
            let mut driver = DurableScheduleDriver::new(1);
            let outcome = driver.drive_one(&mut engine, &store).await.expect("drives");
            let DurableScheduleDrive::Staged(_) = outcome else {
                panic!("the re-drive must stage")
            };
            let staged = engine
                .take_staged_action()
                .expect("admitted")
                .expect("a staged tick");
            let (second, installation) = staged.into_parts();
            assert_eq!(
                first_hash,
                second.bundle_hash(),
                "a re-drive after a dropped bundle must reproduce the same commit"
            );
            engine
                .abort_prepared_action(installation)
                .expect("the fixture aborts");
        }

        /// **T2/T3 (HIGH).** A stood-down domain whose cursor is still live
        /// drives `Idle` or `Transient` — **never** `Err`. Before the fold every
        /// `DomainClockProvider` failure was `ClockReadError::DomainProvider`,
        /// which neither the scan's pause arm nor `schedule_discovery_retryable`
        /// could match, so one stood-down domain aborted the whole shared
        /// discovery and took production, `control_handoff`, `npc` and cut 7's
        /// six `sensor_*` sources down with it inside one 250 ms poll.
        #[tokio::test]
        async fn vf_t2b_a_stood_down_domain_with_a_live_cursor_drives_idle_never_err() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            engine.world.set_now(at + 10);
            drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("a first tick");
            // Stand the domain down WITHOUT disarming: the exact window the old
            // code left open on every demote, and which T3 made permanent.
            if let Some(state) = engine.world.module_state_mut::<SpatialState>()
                && let Some(record) = state.domains.get_mut(&DomainId(0))
            {
                record.clock = mush_sim::data::DomainClock::Passive;
            }
            engine.world.set_now(at + 20);
            for _ in 0..3 {
                let outcome = driver
                    .drive_one(&mut engine, &store)
                    .await
                    .expect("a sick domain must never return Err from the shared scan");
                assert!(
                    matches!(
                        outcome,
                        DurableScheduleDrive::Idle | DurableScheduleDrive::Transient(_)
                    ),
                    "a stood-down domain with a live cursor did not pause its own entry"
                );
            }
            // The provider really cannot answer — the pause is doing the work.
            let clock =
                pulse_clock(&OwnerId::new("sim").unwrap(), DomainId(0), 2).expect("a domain clock");
            assert!(engine.clock_now(&clock).is_err());
        }

        /// **The slot-hold, measured** (§3.8's last unmeasured figure). How long
        /// the driver holds the durable schedule slot for one tick, over an
        /// in-memory store — the floor, not a production figure, but a real
        /// measurement rather than a deferral.
        #[tokio::test]
        async fn vf_d7c_the_measured_slot_hold_for_one_driven_tick() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            let mut worst = std::time::Duration::ZERO;
            let mut total = std::time::Duration::ZERO;
            let rounds = 5_u32;
            for round in 0..rounds {
                engine.world.set_now(at + 10 + 10 * i64::from(round));
                let started = std::time::Instant::now();
                let outcome = drive_until_staged(&mut driver, &mut engine, &store)
                    .await
                    .expect("every driven tick stages");
                let elapsed = started.elapsed();
                assert_eq!(outcome, "staged", "round {round}");
                total += elapsed;
                worst = worst.max(elapsed);
            }
            println!(
                "E7c8 fold D7(c) slot-hold over InMemoryStore: {} tick(s), mean {:?}, worst {:?}",
                rounds,
                total / rounds,
                worst
            );
            // The pin is a sanity ceiling, not a benchmark: a tick that holds
            // the slot for a whole second has stopped being a 2 s pulse.
            assert!(
                worst < std::time::Duration::from_millis(500),
                "one driven tick held the durable schedule slot for {worst:?}"
            );
        }

        /// **T8 (MEDIUM), end to end.** The consumed window is RETAINED for one
        /// tick and then reaped by the NEXT fire's bounded sweep — it is no
        /// longer deleted in the same bundle. Deleting it destroyed the kernel's
        /// own duplicate fence: `prepare_authoritative` treats absence as "open
        /// a fresh one", so an intent that raced the 250 ms driver poll against
        /// the 2 s pulse was accepted into a spent window, acknowledged to the
        /// player, and never consumed.
        #[tokio::test]
        async fn vf_t8b_the_consumed_window_is_retained_one_tick_and_then_reaped() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            engine.world.set_now(at + 10);
            drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("a first tick");
            let first_windows = window_count(&engine);
            assert!(
                first_windows > 0,
                "the consumed window is retained, not deleted in the same bundle"
            );
            // Several more ticks: the family stays flat rather than growing one
            // record per tick, which is what the ring is for.
            for round in 1..6 {
                engine.world.set_now(at + 10 + 10 * i64::from(round));
                drive_until_staged(&mut driver, &mut engine, &store)
                    .await
                    .expect("a later tick");
            }
            let later = window_count(&engine);
            assert!(
                later <= first_windows + 2,
                "the window family grew from {first_windows} to {later} across five                  driven ticks; the reaper is not bounding it"
            );
        }

        /// **B3 (HIGH), end to end.** Neither the window family nor the commit
        /// family grows without bound across a long run of driven ticks.
        #[tokio::test]
        async fn vf_b3c_the_window_and_commit_families_stay_flat_across_many_driven_ticks() {
            let (mut engine, store, at) = pulsing_world();
            let mut driver = DurableScheduleDriver::new(1);
            for round in 0..8 {
                engine.world.set_now(at + 10 + 10 * i64::from(round));
                drive_until_staged(&mut driver, &mut engine, &store)
                    .await
                    .expect("every tick stages");
            }
            assert!(
                window_count(&engine) <= 3,
                "the window family grew to {} across eight driven ticks",
                window_count(&engine)
            );
            assert!(
                commit_count(&engine) <= usize::try_from(RETAINED).unwrap_or(64) + 1,
                "the commit family grew past its retention ring"
            );
        }

        fn window_count(engine: &mush_core::Engine) -> usize {
            engine
                .action_state()
                .scan_family(&handles(engine).windows, None, 256)
                .map(|page| page.entries().len())
                .unwrap_or(0)
        }

        fn commit_count(engine: &mush_core::Engine) -> usize {
            engine
                .action_state()
                .scan_family(&handles(engine).commits, None, 256)
                .map(|page| page.entries().len())
                .unwrap_or(0)
        }

        /// The source is composed exactly once and under its own id, so nothing
        /// above depends on a second registration having appeared.
        #[test]
        fn vf_d7a_the_domaintick_source_is_composed_once_in_the_server_composition() {
            let world =
                mush_testkit::TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
            let sources = world
                .registry()
                .schedule_sources()
                .expect("a schedule source registry");
            assert!(!sources.is_empty());
            assert_eq!(SCHEDULE_SOURCE, "domaintick");
        }
    }

    #[tokio::test]
    async fn discord_unlink_and_ban_revoke_every_character_on_the_account() {
        for operation in ["self-unlink", "staff-unlink", "ban"] {
            let store = InMemoryStore::new();
            store
                .create_account(AccountRec {
                    id: None,
                    name: "Nick".into(),
                    argon2_hash: "fixture".into(),
                    created: 1,
                    characters: vec![],
                    ssh_keys: vec![],
                    discord: Some(mush_store::DiscordLinkRec {
                        user_id: 42,
                        linked_at: 1,
                        relay_character: None,
                        auto_follow: true,
                        dm_channel: None,
                        quiet: false,
                    }),
                })
                .await
                .unwrap();
            store
                .link_character("Nick", ObjId(7), "Nick")
                .await
                .unwrap();
            store.link_character("Nick", ObjId(8), "Alt").await.unwrap();
            store.discord_user_reserve(42, "Nick").await.unwrap();
            let recorder = Arc::new(RecordingGateway::default());
            let gateway: Gateway = recorder.clone();
            let result = if operation == "ban" {
                run_discord_ban(&gateway, &store, GOD, ObjId(7), "test").await
            } else {
                run_discord_unlink(&gateway, &store, ObjId(7), operation == "staff-unlink").await
            };
            assert!(!result.contains("failed"), "{result}");
            assert_eq!(
                *recorder.discord.lock().unwrap(),
                vec![(7, DiscordControlOp::Revoke), (8, DiscordControlOp::Revoke)],
                "{operation}: {result}"
            );
        }
    }
}
