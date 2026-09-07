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

use mush_canon::{CanonDraft, CanonScope, CanonState};
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

/// Whether canon already holds this operation's root record. **The durable
/// commit-once check**: canon's own ledger, not a document this crate may have
/// failed to write.
#[must_use]
pub fn root_recorded(ctx: &Ctx<'_>, operation: OperationId) -> bool {
    ctx.w.module_state::<CanonState>().is_some_and(|state| {
        !mush_canon::records_for_source(state, &canon_source(operation), 1).is_empty()
    })
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
    settle_root(ctx, doc, outcome)
}

/// The root write, shared by `enter_resolving` and by boot's heal (the same code
/// path re-run is what makes a lost document converge).
fn settle_root(ctx: &mut Ctx<'_>, doc: OperationDoc, outcome: &str) -> bool {
    let id = doc.id();
    let required = doc.terms.required_consumers();
    let already = root_recorded(ctx, id);
    let entities: Vec<mush_core::ObjId> = doc
        .runtime
        .participants
        .iter()
        .map(|p| p.unit.0)
        .chain(doc.terms.sides.iter().filter_map(|s| match &s.principal {
            mush_org::PrincipalId::Object(o) => Some(*o),
            _ => None,
        }))
        .collect();
    let scope = scope_for(&doc.terms.visibility);
    let now = ctx.w.now();

    let mut doc = doc;
    doc.runtime.required = required.clone();
    // Resolving IS the decision the page asked for, so it clears the pause: an
    // operation stopped for want of funds, or by a consumer that kept refusing,
    // must not stay stopped once a person has said how it ends.
    doc.runtime.paused = None;
    doc.runtime.attempts.clear();
    doc.runtime.state = OperationState::Resolving { pending: required };
    doc.runtime.root = Some(RootFact {
        canon: None,
        at: now,
        outcome: SmolStr::new(outcome),
    });
    doc.runtime.revision = doc.runtime.revision.saturating_add(1);
    let visibility_label = match scope {
        CanonScope::Public => "public",
        CanonScope::Restricted => "restricted",
        CanonScope::Secret => "secret",
    };
    // Queue the document FIRST: `mush_canon::commit` pins the deferred prefix on
    // its way out, so everything queued before it is pinned against dispatch's
    // abort path and nothing fallible follows the pin (08j seam 3).
    put_operation(ctx, doc.clone());
    if already {
        // The root record is already in canon's own ledger. Re-running the entry —
        // which is exactly what a reboot does — writes no second record.
        return true;
    }
    let canon = mush_canon::commit(
        ctx,
        CanonDraft {
            kind: SmolStr::new_static(KIND_OPERATION_RESOLVED),
            summary: format!("Operation {id} resolved: {outcome}."),
            entities,
            scope,
            ic_time: ctx.w.game_now_unix(),
            source: canon_source(id),
            detail: serde_json::json!({
                "operation": id.0,
                "outcome": outcome,
                "visibility": visibility_label,
            }),
        },
    );
    if let Some(canon) = canon
        && let Some(root) = doc.runtime.root.as_mut()
    {
        root.canon = Some(canon.0);
        put_operation(ctx, doc);
    }
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
        OperationState::Activated | OperationState::Running | OperationState::Declared => {}
        _ => return false,
    }
    settle_root(ctx, doc, "abort")
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
        pause(
            ctx,
            id,
            PauseReason::UnknownConsumer,
            &format!(
                "operation {id} stores consumer id(s) {} that this build does not know; it \
                 cannot reach Resolved. Run the build that knows them, or abandon with \
                 `@op/resolve {id}=abort`",
                unknown.join(", ")
            ),
        );
        return None;
    }
    if doc.runtime.paused.is_some() {
        return None;
    }
    // The frozen surface is re-checked at EVERY transition, not only at entry —
    // **unless** the recorded resolution is an explicit staff abandonment, which is
    // the one outcome that pays nobody and therefore cannot be what a mutated
    // surface was mutated to obtain.
    let abandoning = doc
        .runtime
        .root
        .as_ref()
        .is_some_and(|r| outcome_abandons(&r.outcome));
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
        Err(reason) => {
            record_failure(ctx, id, next, &reason);
            None
        }
    }
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
) -> Result<(), String> {
    let id = doc.id();
    match consumer {
        // The outcome row is the root marker itself: entering `Resolving` stamped
        // it, so this consumer is discharged by the root's existence and writes
        // nothing a re-run could duplicate.
        ConsumerId::CampaignOutcome => match &doc.runtime.root {
            Some(_) => Ok(()),
            None => Err("the root marker is missing".to_owned()),
        },
        // Discharged by the ONE root canon record: a contract naming
        // `AttestorSet::Canon { kind: campaign.operation.resolved, source: op:{id} }`
        // attests from it, and `mush-faction`'s `on_canon_recorded` observer turns
        // it into a `Clock::advance` and an `on_complete_control` flip. Cut 9
        // writes neither an attestation nor a `ControlRecord`, so re-running this
        // consumer writes nothing at all — which is the property, not a hope.
        ConsumerId::ContractOutcomes | ConsumerId::FrontControl => {
            if root_recorded(ctx, id) {
                Ok(())
            } else {
                Err("the root canon record is not in the ledger".to_owned())
            }
        }
        ConsumerId::BondSettlement => settle_bond(ctx, doc, outcome),
        ConsumerId::ForceRetirement => {
            let Some(mut doc) = ctx
                .w
                .module_state::<CampaignStore>()
                .and_then(|s| s.operation(id).cloned())
            else {
                return Err("the operation vanished".to_owned());
            };
            if doc.runtime.participants.is_empty() && doc.runtime.force.formations.is_empty() {
                // Already retired: a re-run past the applied point is a no-op.
                return Ok(());
            }
            for p in std::mem::take(&mut doc.runtime.participants) {
                doc.runtime.withdrawals.push(p.unit);
            }
            doc.runtime.force.formations.clear();
            doc.runtime.revision = doc.runtime.revision.saturating_add(1);
            put_operation(ctx, doc);
            Ok(())
        }
    }
}

fn settle_bond(ctx: &mut Ctx<'_>, doc: &OperationDoc, outcome: &str) -> Result<(), String> {
    let id = doc.id();
    let Some(bond_terms) = doc.terms.bond.clone() else {
        return Ok(());
    };
    // The **durable** idempotency evidence: a hold that is no longer `Held` has
    // already been settled by this key, whatever this crate's document says. A
    // release twice across a restart is therefore unreachable rather than unlikely.
    let Some(hold) = bond::live_hold(ctx, id) else {
        return match bond::any_hold(ctx, id) {
            // Settled already — the replay HIT.
            Some(_) => Ok(()),
            // Never placed (the operation resolved from `Declared`): nothing owed.
            None => Ok(()),
        };
    };
    let terms = doc.terms.clone();
    if outcome_forfeits(outcome)
        && let Some(to) = bond_terms.forfeit_to.clone()
    {
        return bond::capture(ctx, &terms, &hold, &to).map_err(|e| e.to_string());
    }
    bond::release(ctx, &terms, &hold).map_err(|e| e.to_string())
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
        OperationState::Resolved
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
