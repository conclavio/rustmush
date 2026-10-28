//! Pending commit custody, exact receipt tracking, and the bounded command defer queue.

use super::*;

pub(super) enum PendingReceiptEvent {
    Owned(Result<IntentReceipt, AuthoritativeCommitFailure>),
    Final(Result<(), AuthoritativeCommitFailure>),
}

pub(super) type ClassifiedSend = (DeliveryClass, SendTarget, Arc<OutputEvent>);

pub(super) struct CreateCompletion {
    pub(super) id: ObjId,
    pub(super) reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
}

pub(super) struct ActionCompletion {
    pub(super) staged: StagedAuthoritativeAction,
    pub(super) disposition: Option<submission_disposition::StagedAttempt>,
    pub(super) owned:
        Option<oneshot::Sender<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>>,
    pub(super) finished:
        Option<oneshot::Sender<Result<StagedAuthoritativeAction, AuthoritativeCommitFailure>>>,
    pub(super) ownership_valid: bool,
    pub(super) post_commit_decision: Option<DecisionPostCommit>,
    pub(super) post_commit_decision_response: Option<DecisionResponsePostCommit>,
    pub(super) post_commit_decision_terminal: Option<DecisionTerminalPostCommit>,
}

pub(super) struct DecisionPostCommit {
    pub(super) reservation: DecisionDispatchReservation,
    pub(super) worker: mush_core::DecisionWorkerLease,
}

pub(super) struct DecisionResponsePostCommit {
    pub(super) fence: mush_core::DecisionCompletionFence,
    pub(super) binding: mush_core::DecisionActionBinding,
    pub(super) outcome: mush_core::DecisionTerminalOutcome,
}

pub(super) struct DecisionTerminalPostCommit {
    pub(super) fence: mush_core::DecisionCompletionFence,
    pub(super) binding: mush_core::DecisionActionBinding,
    pub(super) outcome: mush_core::DecisionTerminalOutcome,
}

/// One authoritative batch whose presentation and external work remain held
/// until the writer returns its matching legacy serial or K3 token/hash receipt.
pub(super) struct PendingCommit {
    pub(super) token: CommitToken,
    pub(super) receipt: PendingReceipt,
    pub(super) sends: Vec<ClassifiedSend>,
    pub(super) effects: Vec<(DeliveryClass, Effect)>,
    pub(super) failure_actor: Option<ObjId>,
    pub(super) create: Option<CreateCompletion>,
    pub(super) action: Option<ActionCompletion>,
    /// Typed action-state images installed only after this exact receipt.
    pub(super) prepared_installation: Option<PreparedInstallation>,
    pub(super) barriers: Vec<oneshot::Sender<Result<(), WorldReadError>>>,
    /// Gateway reannounce is a full replaceable snapshot. Retain only the
    /// newest bounded snapshot while the exact receipt is unresolved; applying
    /// each one immediately would append another rendered room view to
    /// `sends` and make transport churn an unbounded custody side channel.
    pub(super) coalesced_reattach: Option<Box<[ReAttachRec]>>,
    pub(super) exit_after_ack: bool,
}

pub(super) enum PendingReceipt {
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
    pub(super) const fn authoritative_identity(
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

    pub(super) const fn token(&self) -> CommitToken {
        match self {
            Self::Legacy {
                expected_serial, ..
            } => CommitToken::Legacy(*expected_serial),
            Self::Authoritative { expected_token, .. } => {
                CommitToken::Authoritative(*expected_token)
            }
        }
    }

    pub(super) async fn receive(&mut self) -> Result<(), AuthoritativeCommitFailure> {
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

    pub(super) async fn receive_ownership(
        &mut self,
    ) -> Result<IntentReceipt, AuthoritativeCommitFailure> {
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

    pub(super) const fn expects_ownership(&self) -> bool {
        matches!(
            self,
            Self::Authoritative {
                ownership: Some(_),
                ..
            }
        )
    }

    pub(super) async fn receive_next(&mut self) -> PendingReceiptEvent {
        if self.expects_ownership() {
            PendingReceiptEvent::Owned(self.receive_ownership().await)
        } else {
            PendingReceiptEvent::Final(self.receive().await)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PendingCustody {
    pub(super) barriers: usize,
    pub(super) reattach_records: usize,
    pub(super) operational_bytes: usize,
    pub(super) sends: usize,
    pub(super) effects: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReattachOverload {
    pub(super) records: usize,
    pub(super) logical_bytes: usize,
}

pub(super) struct CompletedPending {
    pub(super) outcome: DeliveryOutcome,
    pub(super) barriers: Vec<oneshot::Sender<Result<(), WorldReadError>>>,
    pub(super) coalesced_reattach: Option<Box<[ReAttachRec]>>,
    pub(super) decision_response: Option<DecisionResponsePostCommit>,
    pub(super) decision_terminal: Option<DecisionTerminalPostCommit>,
}

impl PendingCommit {
    pub(super) fn retain_barrier(
        &mut self,
        reply: oneshot::Sender<Result<(), WorldReadError>>,
    ) -> bool {
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
    pub(super) fn coalesce_reattach(
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

    pub(super) fn custody(&self) -> PendingCustody {
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
pub(super) fn publish_pending_custody(
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
pub(super) fn settle_barrier(
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
pub(super) type DeferInstant = tokio::time::Instant;

/// **One entry of the [`DeferQueue`]**: what waits, and since when (16b cut
/// B.5). `since` is the instant the entry first joined the queue, and it
/// survives a return to the front ([`DeferQueue::push_front_kept`]), so the
/// [`COMMAND_DEFER_MS`] bound is always measured from the player's keystroke.
pub(super) struct Deferred {
    pub(super) since: DeferInstant,
    pub(super) entry: DeferEntry,
}

/// What a [`Deferred`] holds.
pub(super) enum DeferEntry {
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
pub(super) struct DeferQueue {
    pub(super) entries: VecDeque<Deferred>,
}

impl DeferQueue {
    /// Queue a message refused `CommitPending`, or hand it back when its
    /// connection already has [`COMMAND_DEFER_DEPTH`] typed lines waiting (a
    /// typed line only) or the queue holds [`COMMAND_DEFER_CAP`] entries.
    #[allow(clippy::result_large_err)] // Refusal returns the exact inbound message without allocating on a full queue.
    pub(super) fn push_msg(&mut self, msg: WorldMsg, now: DeferInstant) -> Result<(), WorldMsg> {
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
    pub(super) fn push_barrier(
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
    pub(super) fn push_front_kept(&mut self, deferred: Deferred) {
        self.entries.push_front(deferred);
    }

    /// Take every message that has waited [`COMMAND_DEFER_MS`] or longer, front
    /// to back. A barrier never expires and keeps its place among the rest.
    pub(super) fn expire(&mut self, now: DeferInstant) -> Vec<WorldMsg> {
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
    pub(super) fn oldest_deadline(&self) -> Option<DeferInstant> {
        self.entries
            .iter()
            .filter(|held| matches!(held.entry, DeferEntry::Msg(_)))
            .map(|held| held.since.checked_add(COMMAND_DEFER).unwrap_or(held.since))
            .min()
    }

    pub(super) fn pop_front(&mut self) -> Option<Deferred> {
        self.entries.pop_front()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Everything, front to back, leaving the queue empty (shutdown and the
    /// loop's exit).
    pub(super) fn drain_all(&mut self) -> Vec<Deferred> {
        self.entries.drain(..).collect()
    }

    /// How many barrier waiters are queued (the custody gauge).
    pub(super) fn barriers(&self) -> usize {
        self.entries
            .iter()
            .filter(|held| matches!(held.entry, DeferEntry::Barrier(_)))
            .count()
    }
}
