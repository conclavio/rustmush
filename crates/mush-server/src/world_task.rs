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
    /// Gateway-retained connection reply delimiters.
    pub output_brackets: OutputBrackets,
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
    /// Fence the sequential staff executor after prior world delivery.
    StaffBarrier {
        reply: oneshot::Sender<Result<(), WorldReadError>>,
    },
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

mod commit_pipeline;
mod defer_queue;
mod effects_dispatch;
mod mode;
mod staff_ops;

use self::commit_pipeline::*;
use self::defer_queue::*;
use self::effects_dispatch::*;
pub use self::effects_dispatch::{AsyncEffectRegistry, AsyncEffectRunner};
use self::mode::*;
pub use self::mode::{
    CommitToken, STORE_READ_ONLY_ALLOWED, StoreHealth, WorldMode, WriterSignal,
    WriterSignalReceiver, WriterSignalSender, writer_signal_channel,
};
use self::staff_ops::*;
pub use self::staff_ops::{LlmStatsLine, StatsExtra, render_ps, render_stats};

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
        let result = rx.await.unwrap_or(Err(WorldReadError::Unavailable))?;
        if result.is_ok() {
            self.barrier_result().await?;
        }
        Ok(result)
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

    /// Typed reply completion: committed world delivery followed by all prior
    /// sequential staff work, including the store acknowledgment for a dump.
    pub async fn reply_barrier_result(&self) -> Result<(), WorldReadError> {
        self.barrier_result().await?;
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WorldMsg::StaffBarrier { reply })
            .await
            .map_err(|_| WorldReadError::Unavailable)?;
        rx.await.unwrap_or(Err(WorldReadError::Unavailable))
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
    pub arenas: Option<Arc<dyn mush_core::OffLoopSimulationHost>>,
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
/// (`production_durable_outbox`) plus the deployment's own
/// (`ServerDeps::outbox_adapters` — programme D5's on-call Discord DM adapter
/// among them). A duplicate `(owner, adapter)` is logged and skipped.
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
/// The placeholder tactics boundary-fact outbox (`TacticsBoundaryOutbox`) was
/// retired alongside the cut-T combat pulse resolution (`tacsim::arena` owns
/// combat facts). Deployment-specific adapters (such as Discord DM paging) are
/// registered through [`ServerDeps::outbox_adapters`].
#[must_use]
pub fn production_durable_outbox() -> DurableOutboxRegistry {
    DurableOutboxRegistry::new()
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
    mut engine: Engine,
    deps: ServerDeps,
    durable_outbox: Arc<DurableOutboxRegistry>,
    decision_runtime: Option<DecisionRuntime>,
    initial_mode: Option<WorldMode>,
    make_gateway: impl FnOnce(mpsc::WeakSender<WorldMsg>) -> Gateway,
) -> PersistHandles {
    engine.use_gateway_reply_framing();
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    // 16b cut T.2 (T1, T4): the game layer's shared off-loop simulation host,
    // registered in `register` and read back out of the engine's registry here. A
    // composition with no game layer leaves it `None` and the arena egress arm
    // disarmed.
    let arenas = engine.off_loop_simulation_host();
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
pub async fn arena_shutdown(arenas: Option<Arc<dyn mush_core::OffLoopSimulationHost>>) {
    if let Some(arenas) = arenas {
        arenas.shutdown().await;
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
    ArenaEgress(usize),
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

#[allow(clippy::too_many_lines)] // the world loop is one cohesive state machine
#[allow(clippy::too_many_arguments)] // the world loop threads its whole context
async fn run(
    mut engine: Engine,
    mut rx: mpsc::Receiver<WorldMsg>,
    tx: mpsc::WeakSender<WorldMsg>,
    writer: StoreWriter,
    deps: ServerDeps,
    staff_ops: mpsc::Sender<StaffWork>,
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
    arenas: Option<Arc<dyn mush_core::OffLoopSimulationHost>>,
) -> WorldOutcome {
    tracing::info!("world task running");
    // 16b cut T.2 (T4): the one receiver of every running match's egress. Taken
    // once, here, because the loop is its only legitimate owner — a second taker
    // would silently split the stream.
    let mut arena_egress = arenas.as_ref().and_then(|a| a.take_egress());
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
                    Some(rx) => rx.recv_and_stage().await,
                    None => std::future::pending().await,
                }
            }, if arena_egress_armed => match egress {
                Some(staged) => Wake::ArenaEgress(staged),
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
                                        let brackets = output_brackets_for_engine(&engine, &msg);
                                        refuse_world_msg(msg, reason, &gateway, brackets);
                                    }
                                },
                            }
                        } else {
                            if is_typed_line(&msg) {
                                metrics.command_processed();
                            }
                            let brackets = output_brackets_for_engine(&engine, &msg);
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
                        let brackets = output_brackets_for_engine(&engine, &msg);
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
                            } else if (req.dm_conn.is_none()
                                && req.text.chars().count() > MAX_RELAY_LINE_CHARS)
                                || req.text.len() > mush_core::MAX_INGRESS_LINE_BYTES
                            {
                                Err(mush_ipc::RelayError::TooLong)
                            } else if let Some(conn) = req.dm_conn {
                                if engine.actor_of(conn) != Some(req.actor) {
                                    Err(mush_ipc::RelayError::AccountMismatch)
                                } else if engine.world.input_depth(req.actor) > 0 {
                                    Err(mush_ipc::RelayError::NotChannelAlias)
                                } else {
                                    engine
                                        .relay_alias_identified(
                                            req.actor,
                                            &req.channel,
                                            req.kind,
                                            &req.text,
                                            req.message_id,
                                            &mut out,
                                        )
                                        .map_err(|e| match e {
                                            mush_core::RelayRefusal::NotChannelAlias => {
                                                mush_ipc::RelayError::NotChannelAlias
                                            }
                                            e => mush_ipc::RelayError::Refused(e.to_string()),
                                        })
                                }
                            } else {
                                engine
                                    .relay_speak_identified(
                                        req.actor,
                                        &req.channel,
                                        req.kind,
                                        &req.text,
                                        req.message_id,
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
                            engine.finish_external_step(&mut out, &mut fx);
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
                        WorldMsg::StaffBarrier { reply } => {
                            // Previous world work has crossed its exact receipt and
                            // the previous iteration dispatched its local/store ops.
                            if let Err(error) = staff_ops.try_send(StaffWork::Barrier(reply)) {
                                let result = if matches!(&error, mpsc::error::TrySendError::Full(_))
                                {
                                    WorldReadError::Busy
                                } else {
                                    WorldReadError::Unavailable
                                };
                                if let StaffWork::Barrier(reply) = error.into_inner() {
                                    let _ = reply.send(Err(result));
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
                    let brackets = output_brackets_for_engine(&engine, &msg);
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
            Wake::ArenaEgress(staged) => match arenas.as_ref().filter(|_| staged > 0) {
                None => StepOutcome::Continue,
                Some(arenas) => guarded!(
                    "engine.step/arena-egress",
                    engine.step(
                        Inbound::Apply(WorldCommand::ModuleAsyncDone {
                            module: SmolStr::new_static(arenas.module_name()),
                            actor: mush_core::GOD,
                            correlation: 0,
                            payload: arenas.egress_payload().to_owned(),
                        }),
                        &mut out,
                        &mut fx,
                    )
                ),
            },
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
                        out.best_effort_error(actor, mush_core::action::SUBMISSION_REFUSED_LINE);
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
                            let brackets = output_brackets_for_engine(&engine, &msg);
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
                        let brackets = output_brackets_for_engine(&engine, &blocked);
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
        // Publish settings validated by the command path, including clears.
        // This precedes any subsequent delivery/ReplyBarrier acknowledgement.
        for (conn, brackets) in engine.world.take_output_bracket_changes() {
            gateway.set_output_brackets(conn, brackets);
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
                let brackets = output_brackets_for_engine(&engine, &msg);
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
        engine
            .world
            .set_output_prefix(rec.conn, rec.output_brackets.prefix.clone());
        engine
            .world
            .set_output_suffix(rec.conn, rec.output_brackets.suffix.clone());
    }
}

#[cfg(test)]
mod tests;
