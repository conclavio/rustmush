//! World execution mode, writer-control signal mailbox, and structural ingress admission.

use super::*;

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

pub(super) const WRITER_SIGNAL_DETAIL_BYTES: usize = 4 * 1024;
pub(super) const MAX_WRITER_RESOLUTION_BYTES: usize = 16 * 1024 * 1024;

pub(super) type WriterSealIdentity = (SmolStr, SmolStr, String);

pub(super) struct BufferedSealResolution {
    pub(super) detail: String,
    pub(super) quarantined: bool,
}

#[derive(Default)]
pub(super) struct WriterSignalState {
    pub(super) store_status: Option<(bool, String)>,
    pub(super) deterministic_failure: Option<String>,
    pub(super) ledger_breach: Option<String>,
    pub(super) resolutions: BTreeMap<WriterSealIdentity, BufferedSealResolution>,
    pub(super) resolution_bytes: usize,
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

    pub(super) fn with_state<T>(&self, f: impl FnOnce(&mut WriterSignalState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }

    pub(super) fn wake(&self) {
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

    pub(super) fn drain_state(&mut self) {
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

pub(super) fn bound_writer_detail(mut detail: String) -> String {
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

pub(super) fn incident_id(domain: &str, detail: &str) -> IncidentId {
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
pub(super) enum IngressClass {
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
pub(super) enum RefusalReason {
    NotReady,
    CommitPending,
    /// **A deferred entry outlived [`COMMAND_DEFER_MS`] behind a commit that is
    /// still pending** (16b cut B.5, `B-13`). Only a typed line hears its own
    /// sentence; every other message is refused exactly as
    /// [`Self::CommitPending`] refuses it (see [`refuse_world_msg`]).
    CommitTimedOut,
    StoreUnavailable,
    IntegrityFrozen,
    OutageCountdown,
}

impl RefusalReason {
    pub(super) fn text(self) -> &'static str {
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
            Self::OutageCountdown => {
                "The world is in a post-outage recovery countdown; reconnects, inspection, and OOC communication are open while world actions and ticks remain paused."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    Allow,
    Refuse(RefusalReason),
    /// Store-read-only mode retains the legacy conservative command parser.
    /// Hard integrity modes never take this branch: every command is refused
    /// structurally before its verb or input-context contents are inspected.
    CheckStoreReadOnlyCommand,
}

pub(super) fn classify_inbound(msg: &Inbound) -> IngressClass {
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

pub(super) fn classify_world_msg(msg: &WorldMsg) -> IngressClass {
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
        | WorldMsg::LegalActions { .. }
        | WorldMsg::StaffBarrier { .. } => IngressClass::WorldRead,
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
pub(super) fn must_defer_behind_commit(msg: &WorldMsg) -> bool {
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
pub(super) fn waits_in_defer_queue(msg: &WorldMsg) -> bool {
    matches!(
        msg,
        WorldMsg::Inbound(Inbound::Cmd { .. } | Inbound::Query { .. } | Inbound::Invoke { .. })
            | WorldMsg::NameFree { .. }
            | WorldMsg::UnlinkPlayer { .. }
            | WorldMsg::InInputContext { .. }
            | WorldMsg::ConnectionReady { .. }
            | WorldMsg::LegalActions { .. }
            | WorldMsg::StaffBarrier { .. }
    )
}

/// Whether a message is a typed line: the one kind `command_processed` counts,
/// and the one kind [`COMMAND_DEFER_DEPTH`] bounds per connection.
pub(super) const fn is_typed_line(msg: &WorldMsg) -> bool {
    matches!(msg, WorldMsg::Inbound(Inbound::Cmd { .. }))
}

pub(super) fn classify_writer_signal(signal: &WriterSignal) -> IngressClass {
    match signal {
        WriterSignal::StoreStatus { .. }
        | WriterSignal::LedgerBreach { .. }
        | WriterSignal::DeterministicFailure { .. } => IngressClass::WriterRecovery,
    }
}

pub(super) fn classify_staff_op(kind: &StaffOpKind) -> IngressClass {
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

pub(super) const fn admission_for(mode: WorldMode, class: IngressClass) -> Admission {
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

pub(super) fn admit_world_msg(mode: WorldMode, world: &World, msg: &WorldMsg) -> Admission {
    let admission = match admission_for(mode, classify_world_msg(msg)) {
        Admission::CheckStoreReadOnlyCommand => resolve_store_read_only_command(world, msg),
        other => other,
    };
    if admission != Admission::Allow {
        return admission;
    }
    if let Some(freeze) = world.world_timeline().freeze()
        && let mush_core::WorldClockFreezeKind::OutageCountdown {
            unfreeze_at_real_unix,
            ..
        } = freeze.kind
        && world.now() < unfreeze_at_real_unix
    {
        return admit_outage_countdown_msg(world, msg);
    }
    Admission::Allow
}

const OUTAGE_COUNTDOWN_EXTRA_ALLOWED: &[&str] = &[
    "ooc",
    "+ooc",
    "page",
    "p",
    "+channel",
    "+com",
    "comlist",
    "comtitle",
    "addcom",
    "delcom",
    "time",
    "@time",
    "when",
    "@when",
    "score",
    "+sheet",
    "+status",
    "@stats",
    "@ps",
];

pub(super) fn admit_outage_countdown_msg(world: &World, msg: &WorldMsg) -> Admission {
    match msg {
        WorldMsg::Inbound(Inbound::Cmd { actor, line, .. }) => {
            if is_allowed_during_outage_countdown(world, *actor, line) {
                Admission::Allow
            } else {
                Admission::Refuse(RefusalReason::OutageCountdown)
            }
        }
        WorldMsg::Inbound(
            Inbound::ConnUp { .. }
            | Inbound::ConnDown { .. }
            | Inbound::SessionGone { .. }
            | Inbound::Query { .. }
            | Inbound::Composing { .. }
            | Inbound::Control(_),
        )
        | WorldMsg::ReAttach { .. }
        | WorldMsg::UnlinkPlayer { .. }
        | WorldMsg::NameFree { .. }
        | WorldMsg::InInputContext { .. }
        | WorldMsg::ConnectionReady { .. }
        | WorldMsg::LegalActions { .. }
        | WorldMsg::StaffBarrier { .. }
        | WorldMsg::Barrier { .. }
        | WorldMsg::RecheckPrivilege { .. }
        | WorldMsg::RelaySpeak { .. } => Admission::Allow,
        WorldMsg::Inbound(Inbound::Respond { .. } | Inbound::Invoke { .. } | Inbound::Apply(_))
        | WorldMsg::CreatePlayer { .. }
        | WorldMsg::AuthoritativeAction(_)
        | WorldMsg::DiscordInteraction { .. } => Admission::Refuse(RefusalReason::OutageCountdown),
    }
}

fn is_allowed_during_outage_countdown(world: &World, actor: ObjId, line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || world.input_depth(actor) > 0 {
        return true;
    }
    if find_exit(world, actor, t).is_some() {
        return false;
    }
    if matches!(t.chars().next(), Some('"' | ':' | ';' | '\\' | '=')) {
        return true;
    }
    let head = t.split_whitespace().next().unwrap_or("").to_lowercase();
    let (verb, switch) = match head.split_once('/') {
        Some((v, s)) => (v, Some(s)),
        None => (head.as_str(), None),
    };
    if STORE_READ_ONLY_ALLOWED.contains(&verb) || OUTAGE_COUNTDOWN_EXTRA_ALLOWED.contains(&verb) {
        return true;
    }
    if verb == "@tacsim" && matches!(switch, Some("time")) {
        return true;
    }
    if matches!(verb, "@contract" | "@econ" | "@hold" | "@vault" | "@asset")
        && matches!(switch, None | Some("show" | "list" | "history" | "status"))
    {
        return true;
    }
    false
}

/// The one admission that needs the message body: a command line typed while
/// the store is read-only is allowed only if the read-only allowlist clears it.
///
/// Only [`Inbound::Cmd`] classifies as [`IngressClass::Command`], so anything
/// else arriving here means the classifier and this resolver have drifted
/// apart. That is a wiring breach rather than something the player did — but
/// the caller is the single writer, so it fails closed with the refusal the
/// caller was already prepared to hear instead of ending the world task.
pub(super) fn resolve_store_read_only_command(world: &World, msg: &WorldMsg) -> Admission {
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

/// The `OUTPUTPREFIX`/`OUTPUTSUFFIX` row of the connection a typed line came
/// from (16b cut AP.4), cloned for [`refuse_world_msg`]; `None` for anything
/// that is not a typed line, and for a connection that set neither.
pub(super) fn output_brackets_for_engine(
    engine: &Engine,
    msg: &WorldMsg,
) -> Option<OutputBrackets> {
    if engine.gateway_frames_replies() {
        None
    } else {
        output_brackets_of(&engine.world, msg)
    }
}

pub(super) fn output_brackets_of(world: &World, msg: &WorldMsg) -> Option<OutputBrackets> {
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
pub(super) fn refuse_world_msg(
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
        WorldMsg::Barrier { reply } | WorldMsg::StaffBarrier { reply } => {
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
pub(super) fn refuse_store_read_only(world: &World, msg: &Inbound) -> Option<(ObjId, String)> {
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
pub(super) fn apply_store_status(
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
pub(super) fn release_prepared_installation(
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
pub(super) fn freeze_on_lost_installation(
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
pub(super) fn recover_from_world_fault(
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

pub(super) fn apply_integrity_freeze(
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
pub(super) fn freeze_orphan_commit_receipt(
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
pub(super) fn broadcast_wizards(world: &World, gateway: &Gateway, text: &str) {
    let ev = Arc::new(OutputEvent::Sys(RichText::tag("system", text)));
    for actor in connected_wizards(world) {
        gateway.deliver(SendTarget::Char(u64::from(actor.0)), Arc::clone(&ev));
    }
}

/// Send a Sys line to every connected player (`W-clock` / `FO-11` outage
/// countdown and unfreeze broadcasts).
pub(super) fn broadcast_connected(world: &World, gateway: &Gateway, text: &str) {
    let ev = Arc::new(OutputEvent::Sys(RichText::tag("system", text)));
    for actor in world.connected() {
        gateway.deliver(SendTarget::Char(u64::from(actor.0)), Arc::clone(&ev));
    }
}
