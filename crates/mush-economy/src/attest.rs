//! The canon **attestation observer** (E4c, 08c §6.1 item 2).
//!
//! The economy subscribes to core's [`WorldEventKind::CanonRecorded`] — never to
//! `mush-canon`, which it does not depend on and must not — and, in the same step
//! (hence the same `WriteBatch` as canon's own append), records an
//! [`Attestation`] on every live contract condition the record actually answers,
//! then tries to settle.
//!
//! ## What "actually answers" means
//!
//! **A producer-declared outcome kind, an exact source, and the right people**
//! (E4c verify fold, E1 + E2). `mush_canon` records *lifecycle* as readily as it
//! records outcomes: `scene.concluded` says a scene ended, and any participant can
//! end a scene — so a payee gated on one could manufacture their own settlement.
//! A canon attestor set must therefore name a kind the composition root registered
//! as an outcome ([`crate::CanonOutcomeKinds`], refused at `Offer` otherwise), must
//! name a specific source rather than wildcarding the kind, and the record must name
//! (or be named by somebody acting for) one of the contract's own parties.
//!
//! **Kind *and* source.** [`AttestorSet::Canon`] names both, and
//! [`crate::canon_set_accepts`] is the single predicate the observer and
//! [`crate::reverify`] share. E5's owner answer 20 is the reason: progress must cite
//! a specific front, opportunity or operation, so `scene.concluded` involving any
//! faction member is not a match key. A canon record with the right kind and the
//! wrong source attests nothing, and neither does one with no matching typed
//! condition at all — which is exactly 08e answer 6's *"prose alone cannot release
//! escrow"*: an LLM may narrate, facilitate and propose an outcome, and every word
//! of it can be committed to canon, but unless the consent surface named that kind
//! and that source before anybody accepted, the escrow does not move.
//!
//! All four of 08e answer 6's routes go through the one `TypedCondition` alphabet
//! and therefore through this seam or beside it: a **sealed scene outcome** is a
//! typed canon record with `source: scene:{uid}` — E6b's `scene.outcome`, sealed
//! under facilitator authority and registered as an outcome kind — matched by an
//! `Attestation { by: Canon { .. } }`
//! condition; an **authorized attestation** is `@contract/attest` under a
//! `Party`/`Capability`/`Staff` set; a **deterministic condition** is
//! `Fact`/`TitleHeld`/`DepositInto`/`Before`/`After`; a **composite** is several of
//! them on one contract, which is what the condition vector is.
//!
//! ## Infallible from dispatch's point of view
//!
//! `mush_canon::commit` is documented as **terminal**: it persists, then emits this
//! event synchronously, and if the command later returns `Err` the dispatch layer
//! truncates every deferred persist while leaving RAM mutated. So this observer must
//! never be a reason a canon-committing command fails. It contains no `?`, no
//! `unwrap`, no `expect`, no panicking index and no path that returns an error:
//! every internal refusal — a settlement that cannot re-verify, a plan that will not
//! build, an allocator at its ceiling — is logged and swallowed by the machinery it
//! calls (`promote_and_settle` pages through the ladder and leaves the contract
//! `ReadyToSettle` for the sweep). The **attestation still stands**, durably, on the
//! contract document, because the write that records it is queued before settlement
//! is attempted and is not conditional on it.

use mush_core::module::WorldEvent;
use mush_core::{Ctx, ObjId};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::clock::EconomicClock;
use crate::contract::{
    Attestation, AttestorRef, AttestorSet, ContractState, EconomicContract, TypedCondition,
    canon_set_accepts,
};
use crate::ids::ContractId;
use crate::module::EconomyStore;

/// How many contracts one canon record may **settle** in the observer's own step
/// (E4c verify fold, E10).
///
/// A record can answer a condition on any number of contracts, and each settlement
/// builds a plan, re-verifies, posts and emits — inside a synchronous observer, on
/// canon's own write batch. Past this many, the attestation is still recorded
/// (durably, on the document) and the contract is left for the sweep, which settles
/// it on the next fire with its own budget. Evidence is never dropped; only the
/// *timing* of the payout moves.
pub const MAX_OBSERVER_SETTLEMENTS_PER_EVENT: usize = 8;

/// The `CanonRecorded` observer.
pub(crate) fn on_canon_recorded(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::CanonRecorded { canon_id, kind, entities, source } = ev else { return };
    // **Remembered first, and whatever happens next** (E4c verify fold, E4a). Canon
    // evidence is edge-triggered: this callback is the only moment the record passes
    // through the economy, and a contract that was not yet `Active` — or an economy
    // that was frozen — missed it with no way back. The tail is what
    // `@econ/unfreeze` and a contract's activation re-scan.
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.note_canon(*canon_id, kind, source);
    }
    // A frozen economy records nothing: every post refuses anyway, and writing an
    // attestation whose settlement can never be attempted would leave a contract
    // claiming evidence the world had stopped acting on.
    if ctx.w.module_state::<EconomyStore>().is_none_or(EconomyStore::is_frozen) {
        return;
    }
    record_and_settle(ctx, *canon_id, kind, source, Some(entities));
}

/// Re-scan the canon records this world has seen against every live `Canon`
/// condition (E4c verify fold, E4a).
///
/// Called from `@econ/unfreeze` and from a contract's activation — the two moments a
/// missed edge becomes recoverable. It records nothing twice: a slot that is already
/// filled is skipped, exactly as the live path skips it.
pub(crate) fn rescan_canon_tail(ctx: &mut Ctx<'_>) {
    let tail = match ctx.w.module_state::<EconomyStore>() {
        Some(store) if !store.is_frozen() => store.canon_tail(),
        _ => return,
    };
    for (canon_id, kind, source) in tail {
        // The tail carries no entity list — it is a record of what the world
        // announced, not a copy of canon — so the party-intersection check does not
        // run here. `source` is still exact, which is the load-bearing half: a
        // re-scan can only reach a condition that named this specific front.
        record_and_settle(ctx, canon_id, &kind, &source, None);
    }
}

/// Record this canon record against every live condition it answers, and settle what
/// that makes ready (bounded by [`MAX_OBSERVER_SETTLEMENTS_PER_EVENT`]).
fn record_and_settle(
    ctx: &mut Ctx<'_>,
    canon_id: u64,
    kind: &str,
    source: &str,
    entities: Option<&[ObjId]>,
) {
    let matched = matching_contracts(ctx, kind, source, entities);
    if matched.is_empty() {
        return;
    }
    let canon = SmolStr::new(format!("canon:{canon_id}"));
    let now = ctx.w.now();
    let mut settled = 0usize;
    for (id, slots) in matched {
        let Some(mut c) = live_contract(ctx, id) else { continue };
        if c.satisfied.len() < c.consent.conditions.len() {
            c.satisfied.resize(c.consent.conditions.len(), None);
        }
        let mut wrote = false;
        for i in slots {
            let Some(slot) = c.satisfied.get_mut(i) else { continue };
            // **Written once** (E4b2 verify fold, M5): re-stamping `at` moves the
            // anchor an `After` condition counts its delay from, so a front that
            // records canon every hour could defer a consented settlement forever.
            if slot.is_some() {
                continue;
            }
            *slot = Some(Attestation {
                by: AttestorRef::Canon {
                    canon: canon.clone(),
                    kind: SmolStr::new(kind),
                    source: SmolStr::new(source),
                },
                at: now,
                clock: EconomicClock::RealUtc,
                source: canon.clone(),
            });
            wrote = true;
        }
        if !wrote {
            continue;
        }
        c.revision = c.revision.saturating_add(1);
        tracing::info!(
            contract = id.0,
            canon = canon_id,
            kind,
            source,
            "economy: a canon record attested a contract condition"
        );
        // Durable either way: the record is installed and queued here, so a
        // settlement that refuses (or a contract that is not ready) still leaves the
        // attestation on the document.
        crate::api::put_contract(ctx, c.clone());
        // E4c verify fold, E10: past the per-event settlement cap the evidence still
        // stands — it is already installed and queued above — and the contract is
        // left for the sweep, which has its own budget and its own batch.
        if settled >= MAX_OBSERVER_SETTLEMENTS_PER_EVENT {
            tracing::warn!(
                contract = id.0,
                canon = canon_id,
                "economy: one canon record answered more contracts than an observer settles in \
                 one step; the attestation stands and the sweep will settle it"
            );
            continue;
        }
        settled = settled.saturating_add(1);
        // …and, when settlement is attempted in the same step, the same record rides
        // the settlement post's own batch (`post_with_companions`), so the evidence
        // and the money it released are one durable write rather than two.
        let _ = crate::contract_live::promote_and_settle_with(ctx, id, true, vec![c]);
    }
}

/// Every live contract, and which of its condition indexes this canon record
/// answers. Read-only: the world is not touched until the caller writes.
fn matching_contracts(
    ctx: &Ctx<'_>,
    kind: &str,
    source: &str,
    entities: Option<&[ObjId]>,
) -> Vec<(ContractId, Vec<usize>)> {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else { return Vec::new() };
    let mut out: Vec<(ContractId, Vec<usize>)> = Vec::new();
    for c in store.snapshot().contracts() {
        // `Disputed` is included for the same reason `@contract/attest` accepts it:
        // evidence usually arrives *because* somebody disputed, and recording it is
        // what lets a resolution settle. It settles nothing — `promote_and_settle`
        // only ever moves an `Active`/`ReadyToSettle` contract.
        if !matches!(
            c.state,
            ContractState::Active | ContractState::ReadyToSettle | ContractState::Disputed
        ) {
            continue;
        }
        // **The record has to be about these people** (E4c verify fold, E1). A
        // producer-declared outcome kind and an exact source are the first two gates;
        // this is the third, and it is what stops a correctly-typed outcome sealed
        // about somebody else's operation from discharging this contract. A record
        // naming nobody names nobody here either.
        if let Some(entities) = entities
            && !names_a_party(ctx, entities, c)
        {
            continue;
        }
        let slots: Vec<usize> = c
            .consent
            .conditions
            .iter()
            .enumerate()
            .filter(|(_, cond)| matches!(cond, TypedCondition::Attestation { .. }))
            .filter(|(_, cond)| answers(c, cond, kind, source))
            .map(|(i, _)| i)
            .collect();
        if !slots.is_empty() {
            out.push((c.id, slots));
        }
    }
    out
}

/// Whether any of the record's entities is (or acts for) one of the contract's
/// parties (E4c verify fold, E1).
///
/// `acts_for` rather than an identity match, for the same reason `may_view_hold`
/// uses it: a contract whose party is `org:Ironhold` is answered by an outcome naming
/// one of Ironhold's officers, and an identity match would silently never fire.
fn names_a_party(ctx: &Ctx<'_>, entities: &[ObjId], c: &EconomicContract) -> bool {
    let auth = crate::api::WorldAuth { w: ctx.w };
    entities.iter().any(|o| {
        let me = PrincipalId::Object(*o);
        c.consent.parties.iter().any(|p| p == &me || auth.acts_for(&me, p))
    })
}

/// Whether this canon record answers `cond` on `c`.
///
/// A named **adjudicator replaces** each condition's own attestor set (08e answer 6,
/// §5.1 deviation 4), so when the surface has one it is the adjudicator that must be
/// a matching canon set — the same rule `@contract/attest` applies to a person.
fn answers(c: &EconomicContract, cond: &TypedCondition, kind: &str, source: &str) -> bool {
    let TypedCondition::Attestation { by } = cond else { return false };
    let effective: &AttestorSet = c.consent.adjudicator.as_ref().unwrap_or(by);
    canon_set_accepts(effective, kind, source)
}

/// The contract as it stands right now (a clone, so nothing borrows the store).
fn live_contract(ctx: &Ctx<'_>, id: ContractId) -> Option<EconomicContract> {
    ctx.w.module_state::<EconomyStore>()?.snapshot().contract(id).cloned()
}
