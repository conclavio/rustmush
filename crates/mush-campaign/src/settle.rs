//! Terminal settlement (E7 cut 9 B7; Q17A, D-E7-20, 10b §3.9's lens I).
//!
//! An operation ends by writing **one root fact**, and every consequence is a
//! separate, idempotent, causally linked consumer transition keyed off that root.
//! Two things make the fan-out crash-safe, and they are the whole argument:
//!
//! 1. **The required set is computed once, at entry to `Resolving`, from the frozen
//!    terms, and stored on the runtime.** It is never re-derived. A later build that
//!    adds a sixth consumer therefore leaves an already-stored set alone, and a
//!    `Resolved` operation never re-opens.
//! 2. **The root is written once**, as two things in one deferred batch: the
//!    `campaign.*` applied marker the `FactProvider` answers from (08c §6.2 — *an
//!    applied marker, never a status*), and one
//!    `mush_canon::commit(CanonDraft { kind: "campaign.operation.resolved", source:
//!    "op:{id}", .. })`. The commit-once check is canon's **own** durable ledger
//!    (`records_for_source`), not this crate's document, so a lost document write
//!    cannot produce a second root record.
//!
//! `mush_canon::commit` pins the deferred prefix on its way out, so **nothing
//! fallible follows the pin** (08j seam 3) — the operation document is queued
//! *before* the commit and rides inside the pinned prefix.
//!
//! ## What each consumer is
//!
//! | consumer | what discharges it |
//! |---|---|
//! | `CampaignOutcome` | the outcome row stamped on the runtime |
//! | `BondSettlement` | the E4 `Hold` released, or captured to the named beneficiary |
//! | `ContractOutcomes` | the root canon record, which a contract's `AttestorSet::Canon { kind: "campaign.operation.resolved", source: "op:{id}" }` attests from |
//! | `FrontControl` | the root canon record, which `mush-faction`'s own `on_canon_recorded` turns into a `Clock::advance` and an `on_complete_control` flip — **cut 9 writes no `ControlRecord` and takes no faction edge** |
//! | `ForceRetirement` | the task force dissolved and its units released |
//!
//! Three of the five are discharged by the ONE root record, which is exactly why
//! re-running a consumer writes nothing: there is nothing left for it to write.

use std::collections::BTreeSet;

use mush_canon::{CanonDraft, CanonScope};
use mush_core::Ctx;
use mush_economy::EconomicVisibility;
use smol_str::SmolStr;

use crate::bond;
use crate::ids::OperationId;
use crate::record::{
    ConsumerId, MAX_CONSUMER_ATTEMPTS, OperationDoc, OperationState, PauseReason, RootFact,
};
use crate::store::{CampaignStore, put_operation};

/// The canon kind one resolved operation records. Registered in the composition
/// root's `with_canon_outcome_kinds` beside `mush_scenes::KIND_SCENE_OUTCOME`, so
/// a contract may name it in an `AttestorSet::Canon` condition; every other
/// `canon:` attestor set is still refused at `@contract/offer`.
pub const KIND_OPERATION_RESOLVED: &str = "campaign.operation.resolved";

/// The canon kind an **abandoned** operation records (E7 cut 9 independent verify
/// fold, D7).
///
/// It is deliberately **not** registered in the composition root's
/// `with_canon_outcome_kinds`, so no contract may name it in an `AttestorSet::Canon`
/// and no settlement can be attested by one. Cut 9 committed the same kind and the
/// same source for an abandonment as for a completion, so a corrupted operation staff
/// had to abandon attested and settled a linked E4 contract exactly as a finished one
/// would — "abandoning is not settling" was true of the bond and false of every
/// consequence.
pub const KIND_OPERATION_ABANDONED: &str = "campaign.operation.abandoned";

/// The `source` every campaign root record names — the specific operation, never a
/// wildcard (E4c verify fold E2's rule).
#[must_use]
pub fn canon_source(operation: OperationId) -> String {
    format!("op:{}", operation.0)
}

/// The canon scope a resolution is recorded under, from the operation's own
/// visibility — `mush-faction`'s `Visibility → CanonScope` precedent, applied to
/// the economy's classes.
///
/// **A covert operation resolves `Secret`**, so `CanonEvent::visible_to` hides it
/// from every non-staff viewer. `Audience` is `Restricted` — the entities on the
/// record are the ones entitled to it — and only `Public` reaches the chronicle.
#[must_use]
pub fn scope_for(visibility: &EconomicVisibility) -> CanonScope {
    match visibility {
        EconomicVisibility::Public => CanonScope::Public,
        EconomicVisibility::Audience { .. } | EconomicVisibility::Parties => CanonScope::Restricted,
        EconomicVisibility::Staff => CanonScope::Secret,
    }
}

/// Whether this operation's root canon record exists. **The durable commit-once
/// check** (E7 cut 9 independent verify fold, D1/D6; decision E4(a)).
///
/// It reads `RootFact.canon` — the `CanonId` the commit itself returned, stamped on
/// the operation document and flushed in the same batch, and written **only** after a
/// successful commit (`settle_root`). Cut 9 asked `mush_canon::records_for_source`
/// instead, which 08f §4.6's R3 row calls a **bounded read seam** in its own words:
/// `CanonState::recent()` (the *unsealed* 16 KB chunk) plus `sealed_tail`, a
/// **boot-time** snapshot of two sealed chunks that is never refreshed at runtime. So
/// a campaign root record left the answer set after ordinary further canon traffic,
/// and the two consumers discharged BY that record — `ContractOutcomes` and
/// `FrontControl` — then refused for ever, three attempts apart, with `=retry`
/// re-refusing identically. The crate's own marker was durable all along and sat
/// unused.
#[must_use]
pub fn root_recorded(ctx: &Ctx<'_>, operation: OperationId) -> bool {
    ctx.w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(operation))
        .and_then(|doc| doc.runtime.root.as_ref())
        .is_some_and(|root| root.canon.is_some())
}

/// Page level 2 with the recommended command filled in, **once per subject** (08c
/// §4.6: resolve programmatically, page only for a genuine decision, stop only the
/// subject).
pub(crate) fn page(ctx: &mut Ctx<'_>, subject: SmolStr, what: &str, runbook: &str) {
    let first = ctx
        .w
        .module_state_mut::<CampaignStore>()
        .is_some_and(|s| s.paged.insert(subject.clone()));
    if !first {
        return;
    }
    tracing::error!(
        target: "campaign",
        subject = %subject,
        runbook = %runbook,
        "campaign: {what} (level 2)"
    );
}

/// Pause an operation, stopping **only** that operation, and page.
pub(crate) fn pause(ctx: &mut Ctx<'_>, id: OperationId, why: PauseReason, runbook: &str) {
    let Some(mut doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return;
    };
    if doc.runtime.paused.as_ref() == Some(&why) {
        return;
    }
    let label = why.label();
    doc.runtime.paused = Some(why);
    doc.runtime.revision = doc.runtime.revision.saturating_add(1);
    put_operation(ctx, doc);
    page(
        ctx,
        SmolStr::new(format!("op:{}:{label}", id.0)),
        &format!("operation {id} is paused ({label})"),
        runbook,
    );
}

/// **Enter `Resolving`**: compute the consumer set once, write the applied marker,
/// and commit the one root canon record.
///
/// Returns `false` and pages when the frozen surface no longer hashes — the
/// operation **disputes and pages rather than settling** (lens Z) — or when the
/// operation is not in a state that may resolve.
pub fn enter_resolving(ctx: &mut Ctx<'_>, id: OperationId, outcome: &str) -> bool {
    let Some(doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return false;
    };
    match doc.runtime.state {
        OperationState::Activated | OperationState::Running | OperationState::Declared => {}
        // Already resolving, resolved or aborted: never re-enter, never re-open.
        _ => return false,
    }
    if !doc.consent_intact() {
        pause(
            ctx,
            id,
            PauseReason::TermsBroken,
            &format!(
                "the frozen terms of {id} no longer hash, so nothing settles: it disputes \
                 rather than paying out. Inspect it with `@op/show/full {id}`; a STAFF \
                 `@op/resolve {id}=abort` is the one path past this, and it RELEASES the \
                 bond rather than capturing it"
            ),
        );
        return false;
    }
    settle_root(ctx, doc, outcome, false)
}

/// The root write, shared by `enter_resolving` and by [`abandon`].
///
/// **The commit's answer is the gate** (E7 cut 9 fold, D6; decision E4(a)). Cut 9
/// queued the applied marker and the state change *before* `mush_canon::commit` and
/// never inspected its `Option<CanonId>` except to backfill — so a commit that
/// produced nothing (no `CanonState`, a refused `prepare_append`, an exhausted
/// allocator) still answered the `campaign.operation.resolved` boundary fact as
/// **satisfied**, from an operation whose root record does not exist. The marker now
/// follows the commit and carries its id: a marker exists **if and only if** a record
/// does. The prefix is pinned again after the document write, so nothing fallible
/// follows the pin (D9).
fn settle_root(ctx: &mut Ctx<'_>, doc: OperationDoc, outcome: &str, abandoning: bool) -> bool {
    let id = doc.id();
    let required = doc.terms.required_consumers();
    if root_recorded(ctx, id) {
        // The record is already in canon and the marker already carries its id.
        // Re-running the entry — which is exactly what a reboot does — writes no
        // second record.
        return true;
    }
    // B9: the producer's cap and the consumer's must be one number. The campaign
    // built up to 128 participants plus 8 side principals and handed them to
    // `mush_canon::commit`, which `truncate`s at `MAX_ENTITIES_PER_EVENT` 64 in
    // SILENCE — so on a `Restricted` resolution every participant past the 64th lost
    // its `visible_to` entitlement to the record and nothing said so. The list is
    // capped here, deliberately, sides first, and the overflow is recorded in
    // `detail`, which makes the truncation a stated choice rather than a surprise.
    let sides = doc.terms.sides.iter().filter_map(|s| match &s.principal {
        mush_org::PrincipalId::Object(o) => Some(*o),
        _ => None,
    });
    let all: Vec<mush_core::ObjId> = sides
        .chain(doc.runtime.participants.iter().map(|p| p.unit.0))
        .collect();
    let overflow = all.len().saturating_sub(mush_canon::MAX_ENTITIES_PER_EVENT);
    let entities: Vec<mush_core::ObjId> = all
        .into_iter()
        .take(mush_canon::MAX_ENTITIES_PER_EVENT)
        .collect();
    let scope = scope_for(&doc.terms.visibility);
    let now = ctx.w.now();
    let visibility_label = match scope {
        CanonScope::Public => "public",
        CanonScope::Restricted => "restricted",
        CanonScope::Secret => "secret",
    };
    let kind = if abandoning {
        KIND_OPERATION_ABANDONED
    } else {
        KIND_OPERATION_RESOLVED
    };
    let canon = mush_canon::commit(
        ctx,
        CanonDraft {
            kind: SmolStr::new_static(kind),
            summary: format!("Operation {id} resolved: {outcome}."),
            entities,
            scope,
            ic_time: ctx.w.game_now_unix(),
            source: canon_source(id),
            detail: serde_json::json!({
                "operation": id.0,
                "outcome": outcome,
                "visibility": visibility_label,
                "entities_dropped": overflow,
            }),
        },
    );
    if canon.is_none() && !abandoning {
        // No record, so no marker, so no boundary fact: the operation stays exactly
        // where it was and a person is told why.
        //
        // **An abandonment is the exception, deliberately.** It causes no
        // consequence, answers no boundary fact (`CampaignFactProvider` refuses one)
        // and pays nobody, so refusing it for want of a chronicle line would leave a
        // player's bond reserved to an operation nothing could end — the very
        // stranding the escape hatch exists to prevent.
        pause(
            ctx,
            id,
            PauseReason::ConsumerFailed {
                consumer: ConsumerId::CampaignOutcome,
            },
            &format!(
                "operation {id} could not commit its root canon record, so it has NOT resolved \
                 and its boundary fact answers unknown. Check that canon is composed and its \
                 ledger is accepting appends, then re-run `@op/resolve {id}=<outcome>`; if it \
                 cannot go on, `@op/resolve {id}=abort` releases the bond"
            ),
        );
        return false;
    }
    if canon.is_none() {
        tracing::warn!(
            target: "campaign",
            operation = id.0,
            "campaign: an abandonment recorded no canon line; it settles nothing and pays nobody"
        );
    }
    let mut doc = doc;
    doc.runtime.required = required.clone();
    // Resolving IS the decision the page asked for, so it clears the pause: an
    // operation stopped for want of funds, or by a consumer that kept refusing,
    // must not stay stopped once a person has said how it ends.
    doc.runtime.paused = None;
    doc.runtime.attempts.clear();
    doc.runtime.abandoned = abandoning;
    doc.runtime.state = OperationState::Resolving { pending: required };
    doc.runtime.root = Some(RootFact {
        canon: canon.map(|c| c.0),
        at: now,
        outcome: SmolStr::new(outcome),
    });
    doc.runtime.revision = doc.runtime.revision.saturating_add(1);
    put_operation(ctx, doc);
    // D9: `commit` pinned the deferred prefix on its way out, and the document write
    // above is queued past that pin. Pin again, so nothing this transition wrote can
    // be dropped by a handler that errs afterwards.
    ctx.out.pin_deferred();
    true
}

/// **Abandon an operation whose frozen surface no longer hashes** — the one path
/// past [`enter_resolving`]'s intactness gate, and it is staff's alone
/// (`@op/resolve <id>=abort`).
///
/// Without it a corrupted document would strand a player's money forever: the
/// operation could never settle, so the `BondSettlement` consumer could never run,
/// so the `Hold` would stand against an account nobody could spend from — the exact
/// shape of E4b2's M2/M3 findings ("a refused unwind left a terminal record holding
/// live escrow that no command could reach, and the failure was logged rather than
/// paged", 08c §5.2). **Abandoning is not settling**: it records the resolution as
/// `abort`, which *releases* the bond rather than capturing it, and every
/// consequence rides the same closed consumer set a successful resolution would.
pub fn abandon(ctx: &mut Ctx<'_>, id: OperationId) -> bool {
    let Some(doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return false;
    };
    match doc.runtime.state {
        OperationState::Activated | OperationState::Running | OperationState::Declared => {
            settle_root(ctx, doc, "abort", true)
        }
        // **The exit from `Resolving`** (E7 cut 9 independent verify fold, C4;
        // decision E3(a)). Cut 9 accepted only the three pre-terminal states, so the
        // one documented escape hatch was unreachable from the only state a consumer
        // can strand a bond in — and two of this module's own level-2 runbooks print
        // `@op/resolve {id}=abort` from exactly there, where it answered "That
        // operation cannot resolve right now." Every arm that gets there is ordinary:
        // a `bond.forfeit` account that does not exist, a beneficiary account closed
        // mid-drain, an instrument mismatch, an unknown stored consumer, a surface
        // broken after entry, and the bond dispute arm below.
        //
        // It re-stamps the recorded outcome as `abort` — which RELEASES the bond,
        // since `outcome_forfeits("abort")` is false — sets the abandonment flag,
        // clears the pause and the attempts, and leaves `pending` exactly as it is so
        // the fan-out drains past the broken surface. It does **not** re-commit
        // canon: the root record is already written and `root_recorded` is durable.
        OperationState::Resolving { .. } => {
            let mut doc = doc;
            let at = doc.runtime.root.as_ref().map_or(ctx.w.now(), |r| r.at);
            doc.runtime.root = Some(RootFact {
                canon: doc.runtime.root.as_ref().and_then(|r| r.canon),
                at,
                outcome: SmolStr::new_static("abort"),
            });
            doc.runtime.abandoned = true;
            doc.runtime.paused = None;
            doc.runtime.attempts.clear();
            doc.runtime.revision = doc.runtime.revision.saturating_add(1);
            put_operation(ctx, doc)
        }
        // Terminal: never re-opened, either way.
        OperationState::Resolved | OperationState::Aborted => false,
    }
}

/// Advance **at most one** consumer for one operation. Returns the consumer it
/// discharged, or `None` when there was nothing to do (which includes every
/// already-applied replay: a re-run past the applied point is a no-op).
pub fn advance_one(ctx: &mut Ctx<'_>, id: OperationId) -> Option<ConsumerId> {
    let doc = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())?;
    let OperationState::Resolving { ref pending } = doc.runtime.state else {
        return None;
    };
    // **Fails closed.** A stored id this build does not know is kept, never
    // dropped: the operation pauses and pages rather than "completing" a set it
    // cannot enumerate.
    if !doc.runtime.unknown_consumers.is_empty() {
        let unknown: Vec<&str> = doc
            .runtime
            .unknown_consumers
            .iter()
            .map(SmolStr::as_str)
            .collect();
        let runbook = format!(
            "operation {id} stores consumer id(s) {} that this build does not know; it cannot \
             reach Resolved. Run the build that knows them, or abandon with `@op/resolve \
             {id}=abort`, which drains the consumers this build DOES know and releases the bond",
            unknown.join(", ")
        );
        // C4: the unknown-consumer arm returned **before any consumer ran**, so the
        // stop was unconditional and the live bond behind it was unreachable by every
        // command — while the runbook's own recommended command was refused from this
        // very state. A staff abandonment now drains past it: the operation still
        // cannot claim to have reached `Resolved` (an id nobody can discharge is kept,
        // never dropped, and still blocks the terminal state), but the money goes back.
        if doc.runtime.abandoned {
            page(
                ctx,
                SmolStr::new(format!("op:{}:unknown-consumer", id.0)),
                &format!("operation {id} was abandoned carrying consumer ids it cannot discharge"),
                &runbook,
            );
        } else {
            pause(ctx, id, PauseReason::UnknownConsumer, &runbook);
            return None;
        }
    }
    if doc.runtime.paused.is_some() {
        return None;
    }
    // The frozen surface is re-checked at EVERY transition, not only at entry —
    // **unless** the recorded resolution is an explicit staff abandonment, which is
    // the one outcome that pays nobody and therefore cannot be what a mutated
    // surface was mutated to obtain.
    //
    // C7: the flag is set by the TRANSITION that took it (`abandon`), never read off
    // the outcome string. Cut 9 asked `outcome_abandons(&root.outcome)`, and the
    // staff test at the command chose only *which function ran* — so a non-staff
    // `Lens::Plan` holder passing the word `abort` recorded the same outcome and
    // switched the mid-drain intactness re-check off for the whole remaining fan-out.
    let abandoning = doc.runtime.abandoned;
    if !abandoning && !doc.consent_intact() {
        pause(
            ctx,
            id,
            PauseReason::TermsBroken,
            &format!(
                "the frozen terms of {id} no longer hash mid-resolution, so the fan-out \
                 is stopped. Inspect it with `@op/show/full {id}`; a STAFF \
                 `@op/resolve {id}=abort` re-enters the settlement and releases the bond"
            ),
        );
        return None;
    }
    let next = ConsumerId::ALL.into_iter().find(|c| pending.contains(c))?;
    let outcome = doc
        .runtime
        .root
        .as_ref()
        .map(|r| r.outcome.clone())
        .unwrap_or_default();
    match run_consumer(ctx, &doc, next, &outcome) {
        Ok(()) => {
            acknowledge(ctx, id, next);
            Some(next)
        }
        // D8: **a wait is not a failure.** `sweep.rs` classifies exactly
        // `JournalUnavailable | EconomyFrozen | RetryShortly` as
        // `Disposition::Transient` ("the next fire is the retry") and doc 13 §2's
        // driver rule says a settlement whose post answers `RetryShortly` is
        // `NotReady`, **no attempt counted**. Cut 9's settlement was the one driver
        // in the tree that counted a wait: three of them paused the operation and
        // demanded a human for something the next fire would have fixed.
        Err(Refusal::Transient(reason)) => {
            tracing::debug!(
                target: "campaign",
                operation = id.0,
                consumer = next.as_str(),
                %reason,
                "campaign: a consumer deferred; the next fire is the retry"
            );
            None
        }
        Err(Refusal::Failed(reason)) => {
            record_failure(ctx, id, next, &reason);
            None
        }
    }
}

/// Why one consumer did not discharge. The two arms are the whole of D8's fix: a
/// **wait** defers without counting an attempt; a **failure** counts one, and three
/// of them pause this operation and page.
enum Refusal {
    /// The economy asked to be retried. The next fire is the retry.
    Transient(String),
    /// A genuine refusal.
    Failed(String),
}

/// Whether an outcome forfeits the bond. **One word, one rule: only `forfeit`
/// takes a player's money; every other outcome releases it**, `abort` included —
/// abandoning an operation is not settling it, and an unwind must never be a way to
/// capture. Stated here rather than inferred anywhere else, so two paths cannot
/// disagree about whose money moves.
#[must_use]
pub fn outcome_forfeits(outcome: &str) -> bool {
    outcome.trim() == "forfeit"
}

/// Whether a recorded outcome is an explicit **abandonment** rather than a
/// settlement. An abandonment is the one resolution allowed to drain past a broken
/// consent surface: it pays nobody, releases the bond to the account it was
/// reserved on, and frees the units — which is the only alternative to leaving a
/// player's money reserved to a record nothing can ever settle.
#[must_use]
pub fn outcome_abandons(outcome: &str) -> bool {
    outcome.trim() == "abort"
}

fn run_consumer(
    ctx: &mut Ctx<'_>,
    doc: &OperationDoc,
    consumer: ConsumerId,
    outcome: &str,
) -> Result<(), Refusal> {
    let id = doc.id();
    match consumer {
        // The outcome row is the root marker itself: entering `Resolving` stamped
        // it, so this consumer is discharged by the root's existence and writes
        // nothing a re-run could duplicate.
        ConsumerId::CampaignOutcome => match &doc.runtime.root {
            Some(_) => Ok(()),
            None => Err(Refusal::Failed("the root marker is missing".to_owned())),
        },
        // Discharged by the ONE root canon record: a contract naming
        // `AttestorSet::Canon { kind: campaign.operation.resolved, source: op:{id} }`
        // attests from it, and `mush-faction`'s `on_canon_recorded` observer turns
        // it into a `Clock::advance` and an `on_complete_control` flip. Cut 9
        // writes neither an attestation nor a `ControlRecord`, so re-running this
        // consumer writes nothing at all — which is the property, not a hope.
        ConsumerId::ContractOutcomes | ConsumerId::FrontControl => {
            // An abandonment causes no consequence at all — it commits its own kind
            // (`campaign.operation.abandoned`, which no contract may name), answers
            // no boundary fact and moves no front — so the consumers that exist only
            // to note a consequence have nothing to wait for.
            if doc.runtime.abandoned || root_recorded(ctx, id) {
                Ok(())
            } else {
                Err(Refusal::Failed(
                    "the root canon record is not in the ledger".to_owned(),
                ))
            }
        }
        ConsumerId::BondSettlement => settle_bond(ctx, doc, outcome),
        ConsumerId::ForceRetirement => {
            let Some(mut doc) = ctx
                .w
                .module_state::<CampaignStore>()
                .and_then(|s| s.operation(id).cloned())
            else {
                return Err(Refusal::Failed("the operation vanished".to_owned()));
            };
            if doc.runtime.participants.is_empty() && doc.runtime.force.formations.is_empty() {
                // Already retired: a re-run past the applied point is a no-op.
                return Ok(());
            }
            // C3: through the bounded, deduplicated audit tail, so a full roster's
            // retirement cannot write a document the next boot would refuse and hold.
            for p in std::mem::take(&mut doc.runtime.participants) {
                doc.runtime.record_withdrawal(p.unit);
            }
            doc.runtime.force.formations.clear();
            doc.runtime.revision = doc.runtime.revision.saturating_add(1);
            if !put_operation(ctx, doc) {
                return Err(Refusal::Failed(
                    "the retired roster would not store".to_owned(),
                ));
            }
            Ok(())
        }
    }
}

fn settle_bond(ctx: &mut Ctx<'_>, doc: &OperationDoc, outcome: &str) -> Result<(), Refusal> {
    let id = doc.id();
    let Some(bond_terms) = doc.terms.bond.clone() else {
        return Ok(());
    };
    // The **durable** idempotency evidence: a hold that is no longer `Held` was
    // settled before this fire, whatever this crate's document says. A release twice
    // across a restart is therefore unreachable rather than unlikely.
    //
    // C2: but it must say **by whom**. Cut 9 read `any_hold(..) { Some(_) => Ok(()) }`
    // — "settled already, the replay HIT" — which could not tell "our own settlement
    // consumed it" from "somebody took it out of band", so a bond the beneficiary had
    // captured through `@hold/capture` (or the attacker had released through
    // `@hold/release`) was acknowledged as `BondSettlement`, the operation reached
    // `Resolved`, and nothing paused and nothing paged. E2(a) closed the out-of-band
    // routes; this is the settlement's own half of the same fix, and it stays
    // authoritative on the hold's status rather than on any key.
    let Some(hold) = bond::live_hold(ctx, id) else {
        let Some(settled) = bond::any_hold(ctx, id) else {
            // Never placed (the operation resolved from `Declared`): nothing owed.
            return Ok(());
        };
        return match settled.status {
            // A capture posted: only this settlement can post one, so it is ours.
            mush_economy::HoldStatus::Consumed { .. } => Ok(()),
            // Held is unreachable here (`live_hold` would have found it).
            mush_economy::HoldStatus::Held => Ok(()),
            // Released or expired: ours if our own settlement key resolved to a
            // transaction, and a **dispute** otherwise.
            mush_economy::HoldStatus::Released | mush_economy::HoldStatus::Expired => {
                if mush_economy::idempotency_resolved(
                    ctx,
                    &bond::consumer_key(id, ConsumerId::BondSettlement),
                ) {
                    Ok(())
                } else {
                    pause(
                        ctx,
                        id,
                        PauseReason::Unfunded,
                        &format!(
                            "operation {id}'s bond is no longer held and this settlement did not \
                             end it, so nothing is paid and nothing is acknowledged. Inspect the \
                             reservation and the journal; when the dispute is decided, \
                             `@op/resolve {id}=abort` closes the operation without paying"
                        ),
                    );
                    Err(Refusal::Failed(
                        "the bond was settled by something other than this operation".to_owned(),
                    ))
                }
            }
        };
    };
    let terms = doc.terms.clone();
    let outcome = if outcome_forfeits(outcome)
        && let Some(to) = bond_terms.forfeit_to.clone()
    {
        bond::capture(ctx, &terms, &hold, &to)
    } else {
        bond::release(ctx, &terms, &hold)
    };
    outcome.map_err(|e| {
        if e.is_transient() {
            Refusal::Transient(e.to_string())
        } else {
            Refusal::Failed(e.to_string())
        }
    })
}

fn acknowledge(ctx: &mut Ctx<'_>, id: OperationId, consumer: ConsumerId) {
    let Some(mut doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return;
    };
    let OperationState::Resolving { mut pending } = doc.runtime.state.clone() else {
        return;
    };
    if !pending.remove(&consumer) {
        return;
    }
    doc.runtime.attempts.remove(consumer.as_str());
    doc.runtime.state = if pending.is_empty() && doc.runtime.unknown_consumers.is_empty() {
        // D7: an abandonment really reaches `Aborted`. Before the fold that variant
        // was never constructed anywhere in the tree, so `@op/show` reported
        // "resolved" for an operation staff had abandoned past a corrupt surface and
        // `operation_holding`'s exclusion of it was dead in the write direction.
        if doc.runtime.abandoned {
            OperationState::Aborted
        } else {
            OperationState::Resolved
        }
    } else {
        OperationState::Resolving { pending }
    };
    doc.runtime.revision = doc.runtime.revision.saturating_add(1);
    put_operation(ctx, doc);
}

fn record_failure(ctx: &mut Ctx<'_>, id: OperationId, consumer: ConsumerId, reason: &str) {
    let Some(mut doc) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|s| s.operation(id).cloned())
    else {
        return;
    };
    let slot = doc
        .runtime
        .attempts
        .entry(SmolStr::new(consumer.as_str()))
        .or_insert(0);
    *slot = slot.saturating_add(1);
    let attempts = *slot;
    doc.runtime.revision = doc.runtime.revision.saturating_add(1);
    put_operation(ctx, doc);
    tracing::warn!(
        target: "campaign",
        operation = id.0,
        consumer = consumer.as_str(),
        attempts,
        %reason,
        "campaign: a consumer refused"
    );
    if attempts >= MAX_CONSUMER_ATTEMPTS {
        pause(
            ctx,
            id,
            PauseReason::ConsumerFailed { consumer },
            &format!(
                "operation {id}'s {} consumer refused {attempts} times ({reason}); only this \
                 operation is stopped. Fix the cause and clear it with `@op/resolve {id}=retry`, \
                 or abandon it with `@op/resolve {id}=abort`",
                consumer.as_str()
            ),
        );
    }
}

/// The consumers one operation still owes, for `@op/consumers` (staff).
#[must_use]
pub fn outstanding(doc: &OperationDoc) -> BTreeSet<ConsumerId> {
    doc.runtime.pending()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7c9_b7_the_canon_scope_follows_the_operations_visibility_on_every_arm() {
        assert_eq!(scope_for(&EconomicVisibility::Public), CanonScope::Public);
        assert_eq!(
            scope_for(&EconomicVisibility::Audience {
                members: Vec::new(),
                announce_issuer: false
            }),
            CanonScope::Restricted
        );
        assert_eq!(
            scope_for(&EconomicVisibility::Parties),
            CanonScope::Restricted
        );
        // A covert operation resolves SECRET, so `CanonEvent::visible_to` hides it
        // from every non-staff viewer.
        assert_eq!(scope_for(&EconomicVisibility::Staff), CanonScope::Secret);
    }

    #[test]
    fn e7c9_b7_only_forfeit_takes_a_players_money_and_only_abort_abandons() {
        assert!(outcome_forfeits("forfeit"));
        assert!(outcome_forfeits(" forfeit "));
        // Every other arm — an abandonment included — RELEASES.
        for outcome in ["success", "failure", "partial", "retry", "abort", ""] {
            assert!(
                !outcome_forfeits(outcome),
                "{outcome} took a player's money"
            );
        }
        assert!(outcome_abandons("abort"));
        assert!(outcome_abandons(" abort "));
        for outcome in ["success", "failure", "partial", "forfeit", ""] {
            assert!(!outcome_abandons(outcome), "{outcome}");
        }
    }

    #[test]
    fn e7c9_b7_the_canon_source_names_the_specific_operation_never_a_wildcard() {
        assert_eq!(canon_source(OperationId(12)), "op:12");
        assert_ne!(canon_source(OperationId(1)), canon_source(OperationId(2)));
    }
}
