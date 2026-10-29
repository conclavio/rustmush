//! Write-through flush pipeline, audience resolution, and exact-receipt commit completion.

use super::*;

/// What became of one [`flush_dirty`] call. The distinction between
/// [`Clean`](FlushOutcome::Clean) and
/// [`WriterGone`](FlushOutcome::WriterGone) matters to ack-gated callers: a
/// clean flush has nothing to persist, while a gone writer means a real
/// batch was **dropped** and must never be confirmed as durable.
// One per flush, moved straight into a match; boxing the queued variant
// would allocate on every flush to shrink a short-lived local.
#[allow(clippy::large_enum_variant)]
pub(super) enum FlushOutcome {
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
pub(super) async fn flush_engine_step(
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
pub(super) async fn flush_prepared_action(
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

pub(super) fn legacy_reserved_action_state_violation(
    ops: &[mush_core::ModuleOp],
) -> Option<String> {
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
pub(super) async fn flush_dirty(
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
    if let Some(arenas) = engine.off_loop_simulation_host() {
        arenas.stamp_ledger_writes(*serial);
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
pub(super) fn resolve_sends(
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
pub(super) fn resolve_classified_sends(
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
pub(super) fn answers_an_abort(sends: &[ClassifiedSend]) -> bool {
    sends.iter().any(|(class, _, event)| {
        matches!(class, DeliveryClass::BestEffort | DeliveryClass::OnAbort)
            && !is_output_bracket(event)
    })
}

/// A connection's `OUTPUTPREFIX` or `OUTPUTSUFFIX` line (16b cut AP.4): a
/// `Sys` whose first span carries one of the two tags `Engine::step` gives it.
pub(super) fn is_output_bracket(event: &OutputEvent) -> bool {
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
pub(super) fn split_at_trailer(
    mut sends: Vec<ClassifiedSend>,
) -> (Vec<ClassifiedSend>, Vec<ClassifiedSend>) {
    let at = sends
        .iter()
        .position(|(class, _, _)| *class == DeliveryClass::Trailer)
        .unwrap_or(sends.len());
    let rest = sends.split_off(at);
    (sends, rest)
}

pub(super) fn release_classified_sends(
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

pub(super) fn release_classified_effects(
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
pub(super) fn release_best_effort_before_ack(
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
pub(super) fn release_independent_while_pending(
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
pub(super) fn complete_pending_ownership(
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

pub(super) fn fail_action_completion(
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
pub(super) fn complete_pending_commit(
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
pub(super) fn finish_pending_operational(
    engine: &mut Engine,
    gateway: &Gateway,
    completed: CompletedPending,
) {
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
pub(super) fn abort_without_receipt(
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
pub(super) fn refuse_before_submission(
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

pub(super) fn bounded_failure_detail(detail: &str) -> &str {
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

pub(super) fn send_commit_failure(gateway: &Gateway, actor: ObjId, detail: &str) {
    send_sys(
        gateway,
        actor,
        &format!(
            "The database rejected your last action ({}). Its success output and external effects were suppressed; the world is now integrity-frozen.",
            bounded_failure_detail(detail)
        ),
    );
}
