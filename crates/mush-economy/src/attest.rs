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
//! ## The source is **opaque to the economy** (E6b independent verify fold, M10)
//!
//! `AttestorSet::Canon { kind, source }` compares `source` as a byte string and
//! interprets nothing about it. `run:{id}#{row}`, `scene:{uid}` and anything else are
//! all the same to [`canon_set_accepts`]: the economy asks only that the record's
//! source is *exactly* the one the frozen condition named, which is what makes the
//! seam game-neutral and what lets a ruleset invent its own naming without touching
//! this crate. A hand-built `@contract/condition canon:<kind>:<source>` therefore
//! accepts **any** source, deliberately.
//!
//! The rule that a run board's canon route may only name **its own** run is
//! consequently the *producer's* — `+run/post`'s allowlist in `ruleset-shadowrun`
//! (08f §4.6, D1) — and never this crate's. Two things follow: the economy is bounded
//! but not opinionated (D6 caps the source's *length*, and nothing else), and a
//! ruleset that ships a run board without that allowlist has a hole this seam cannot
//! close for it.
//!
//! All four of 08e answer 6's routes go through the one `TypedCondition` alphabet
//! and therefore through this seam or beside it: a **sealed scene outcome** is a
//! typed canon record with `source: run:{contract id}` — E6b's `scene.outcome`,
//! sealed by a **staff-conferred sealer** (never a self-granted facilitator: any
//! player becomes one by starting a scene) and registered as an outcome kind. A
//! `scene:{uid}` was never usable: the scene does not exist when the run is
//! offered, and the condition freezes then. It is matched by an
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
    let WorldEvent::CanonRecorded {
        canon_id,
        kind,
        entities,
        source,
    } = ev
    else {
        return;
    };
    // **Remembered first, and whatever happens next** (E4c verify fold, E4a). Canon
    // evidence is edge-triggered: this callback is the only moment the record passes
    // through the economy, and a contract that was not yet `Active` — or an economy
    // that was frozen — missed it with no way back. The tail is what
    // `@econ/unfreeze` and a contract's activation re-scan.
    let now = ctx.w.now();
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.note_canon(*canon_id, kind, source, entities, now);
    }
    // A frozen economy records nothing: every post refuses anyway, and writing an
    // attestation whose settlement can never be attempted would leave a contract
    // claiming evidence the world had stopped acting on.
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return;
    }
    record_and_settle(ctx, *canon_id, kind, source, Some(entities), now);
}

/// Re-scan the canon records this world has seen against every live `Canon`
/// condition (E4c verify fold, E4a; E6b DL-09).
///
/// Called from `@econ/unfreeze`, from a contract's activation, and once from the
/// first sweep after boot — the three moments a missed edge becomes recoverable. It
/// records nothing twice: a slot that is already filled is skipped, exactly as the
/// live path skips it.
///
/// **E6b (DL-09):** the tail now carries each record's `entities`, so the recovery
/// path runs the same party-intersection gate the live path runs and is no weaker
/// than it. The tail is still RAM-only and 64 deep, so [`rescan_canon_sources`]
/// follows it with the durable read seam.
pub(crate) fn rescan_canon_tail(ctx: &mut Ctx<'_>, trigger: RescanTrigger) {
    // E6b independent verify fold (R5): **the latch is taken here, one level up.**
    // It lived inside `rescan_canon_sources`, so the 64-deep RAM tail — which
    // settles, which emits, which an observer could in principle answer by
    // activating a contract that re-enters this function — was walked with nothing
    // guarding it. The re-entrant walk is now a no-op at its first line, which is
    // what "walks once" means.
    match ctx.w.module_state_mut::<EconomyStore>() {
        Some(store) if !store.canon_rescanning => store.canon_rescanning = true,
        _ => return,
    }
    let tail = match ctx.w.module_state::<EconomyStore>() {
        Some(store) if !store.is_frozen() => store.canon_tail(),
        _ => {
            clear_rescan_latch(ctx);
            return;
        }
    };
    for seen in tail {
        record_and_settle(
            ctx,
            seen.canon_id,
            &seen.kind,
            &seen.source,
            Some(&seen.entities),
            seen.at,
        );
    }
    rescan_sources_inner(ctx, trigger);
    clear_rescan_latch(ctx);
}

/// Why a re-scan is running. **Recovery** (boot, `@econ/unfreeze`) is the case where
/// a source the seam cannot answer is a genuine loss worth paging about; an
/// **activation** re-scan is the ordinary case where the evidence simply has not been
/// produced yet (E6b independent verify fold, R3b).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum RescanTrigger {
    /// A contract just went `Active` and is catching up on missed edges.
    Activation,
    /// Boot's first sweep, or `@econ/unfreeze`.
    Recovery,
}

/// Drop the re-entrancy latch (E6b independent verify fold, R5).
fn clear_rescan_latch(ctx: &mut Ctx<'_>) {
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.canon_rescanning = false;
    }
}

/// Page the ladder about a live `Canon` condition whose source the durable seam
/// cannot answer at all (E6b independent verify fold, R3b).
///
/// The subject is the **contract**, so the ladder dedupes to one page per stuck
/// contract however often the re-scan runs, and the recommended command is the level-2
/// answer 08c §6.1 already documents for evidence that can no longer be produced.
fn page_missing_canon(ctx: &mut Ctx<'_>, id: ContractId, source: &str) {
    let now = ctx.w.now();
    tracing::warn!(
        contract = id.0,
        source,
        "economy: a live canon condition cites a source no record can be found for"
    );
    crate::sweep::page(
        ctx,
        crate::escalation::EscalationKind::EvidenceUnavailable,
        crate::escalation::EscalationSubject::Contract(id),
        None,
        format!(
            "contract #{id} waits on canon evidence from {source}, and no such record can be              found in the chronicle's readable window; if the outcome really was sealed, the              record is past recovery"
        ),
        vec![
            format!("@contract/show #{id}"),
            format!("@contract/resolve #{id}=settle"),
        ],
        now,
    );
}

/// One canon record as **evidence**, read back from the producer (E6b, DL-09).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonRecord {
    /// The record's durable id.
    pub canon_id: u64,
    /// Its kind.
    pub kind: SmolStr,
    /// Its source, compared as an opaque string.
    pub source: SmolStr,
    /// Who it names — the party-intersection gate's input.
    pub entities: Vec<ObjId>,
    /// When the record was made (wall clock, unix seconds).
    ///
    /// E6b independent verify fold (R4): the generation gate's input. A seal made
    /// against a *previous* generation of a consent surface must not be put back by
    /// a re-scan after an amendment cleared it, and the source (`run:{id}#{row}`)
    /// names no generation — so the record's own time is what makes the comparison
    /// possible. The live edge passes `ctx.w.now()`, which is the same instant.
    pub at: i64,
}

/// The **bounded canon read seam** (E6b, DL-09), wired at composition.
///
/// The economy's canon evidence is edge-triggered: `on_canon_recorded` runs once,
/// when the record lands, and a contract that was not yet `Active`, an economy that
/// was frozen, or a restart in between all miss the edge. The RAM tail
/// ([`crate::CANON_TAIL`]) recovers 64 records within one boot and nothing at all
/// across a restart, which is exactly the gap a settlement sealed before a reboot
/// falls into.
///
/// This seam closes it without a crate edge: `mush-economy` must not depend on
/// `mush-canon` (08c §6.1 deviation 1), so the composition root implements this over
/// `mush_canon::records_for_source` and hands it in with
/// `EconomyModule::with_canon_evidence`. Omitting it is the pre-E6b behaviour
/// exactly.
pub trait CanonEvidence: Send + Sync + std::fmt::Debug {
    /// The records naming `source`, newest first, at most `limit`.
    fn records_for_source(
        &self,
        w: &mush_core::World,
        source: &str,
        limit: usize,
    ) -> Vec<CanonRecord>;
}

/// The wiring-frozen canon evidence seam (an `Arc`, cheap to clone).
pub type CanonEvidenceSeam = Option<std::sync::Arc<dyn CanonEvidence>>;

/// How many distinct `(kind, source)` pairs one re-scan looks up (E6b, DL-09).
pub const MAX_CANON_SOURCE_LOOKUPS: usize = 32;

/// How many records one source lookup asks for (E6b, DL-09).
pub const MAX_CANON_RECORDS_PER_SOURCE: usize = 8;

/// How many contracts one re-scan **visits** (E6b verify fold, D5).
///
/// D-LBK-5's rule: a bounded read bounds the records it *touches*, never the ones it
/// *matches*. `MAX_CANON_SOURCE_LOOKUPS` bounds the matches, and on a world where
/// few contracts carry a sourced `Canon` set — the normal case — that bound is never
/// reached, so the walk was O(contracts) on every activation.
pub const MAX_CANON_CONTRACTS_VISITED: usize = 256;

/// Ask the durable seam for the canon records every live `Canon` condition cites,
/// and record what they answer (E6b, DL-09).
///
/// Bounded twice over: at most [`MAX_CANON_SOURCE_LOOKUPS`] distinct sources, at
/// most [`MAX_CANON_RECORDS_PER_SOURCE`] records each. The sources come from the
/// contracts themselves — a condition's own `AttestorSet::Canon`, or the surface's
/// adjudicator when it has one, which is what actually binds (D-E6-8) — so the walk
/// is over evidence somebody consented to, never over the whole chronicle.
pub(crate) fn rescan_canon_sources(ctx: &mut Ctx<'_>, trigger: RescanTrigger) {
    // The latch belongs to whichever entry point is outermost (R5); boot's sweep
    // calls this one directly, so it takes the latch itself.
    match ctx.w.module_state_mut::<EconomyStore>() {
        Some(store) if !store.canon_rescanning => store.canon_rescanning = true,
        _ => return,
    }
    rescan_sources_inner(ctx, trigger);
    clear_rescan_latch(ctx);
}

/// The re-scan proper — the caller holds the latch.
fn rescan_sources_inner(ctx: &mut Ctx<'_>, trigger: RescanTrigger) {
    let (seam, wanted) = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        if store.is_frozen() {
            return;
        }
        let Some(seam) = store.canon_evidence() else {
            return;
        };
        // E6b independent verify fold (R3b): the map remembers **which contract**
        // first cited each source, so a source the seam answers with nothing can be
        // paged against that contract rather than dropped on the floor.
        let mut wanted: std::collections::BTreeMap<SmolStr, ContractId> =
            std::collections::BTreeMap::new();
        // E6b independent verify fold (R1): **filter, then take.** The bound sat
        // ahead of the live filter, and `contracts` is a `BTreeMap` in ascending id
        // whose terminal entries are never garbage-collected — so on any world with
        // more than `MAX_CANON_CONTRACTS_VISITED` contracts the whole budget was
        // spent on the oldest *dead* ones and the live run at the end was never
        // visited at all. DL-09's recovery then stopped silently: a seal committed
        // while the economy was frozen, or before a reboot, was never found, and the
        // escrow sat until its deadline with nothing saying why. This is the rule for
        // every bounded walk in the cut — bound what you *consider*, not what you
        // *skip*.
        'contracts: for c in store
            .snapshot()
            .contracts()
            .filter(|c| {
                matches!(
                    c.state,
                    ContractState::Active | ContractState::ReadyToSettle | ContractState::Disputed
                )
            })
            .take(MAX_CANON_CONTRACTS_VISITED)
        {
            for set in c
                .consent
                .conditions
                .iter()
                .filter_map(|cond| match cond {
                    TypedCondition::Attestation { by } => Some(by),
                    _ => None,
                })
                .chain(c.consent.adjudicator.iter())
            {
                if let AttestorSet::Canon {
                    source: Some(source),
                    ..
                } = set
                {
                    wanted.entry(source.clone()).or_insert(c.id);
                    if wanted.len() >= MAX_CANON_SOURCE_LOOKUPS {
                        break 'contracts;
                    }
                }
            }
        }
        (seam, wanted)
    };
    for (source, contract) in wanted {
        let records = seam.records_for_source(ctx.w, &source, MAX_CANON_RECORDS_PER_SOURCE);
        // E6b independent verify fold (R3b): **a source with no records anywhere is
        // said out loud.** The seam answers from the open tail and the last sealed
        // chunks; past that, the evidence a live condition consented to is simply
        // unrecoverable, and returning silently left the escrow frozen until its
        // deadline with nothing anywhere saying why. It is paged only from the
        // recovery triggers (boot and `@econ/unfreeze`) and never from an
        // activation — a freshly offered run whose scene has not been played yet is
        // not a stuck one — and the ladder's per-subject dedupe makes it one page per
        // contract however many times the re-scan runs.
        if records.is_empty() && trigger == RescanTrigger::Recovery {
            page_missing_canon(ctx, contract, &source);
            continue;
        }
        for record in records {
            // E6b verify fold (R5): **do not remember what the seam can find again.**
            // `note_canon` evicts the front of a 64-deep tail, and one re-scan can
            // return up to 32 × 8 records — so remembering them replaced the tail
            // with exactly the records that are already recoverable, evicting the
            // ones that are not: records whose `source` no live condition cites
            // *yet*, which a contract offered five minutes later would have found.
            record_and_settle(
                ctx,
                record.canon_id,
                &record.kind,
                &record.source,
                Some(&record.entities),
                record.at,
            );
        }
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
    recorded_at: i64,
) {
    let matched = matching_contracts(ctx, kind, source, entities);
    if matched.is_empty() {
        return;
    }
    let canon = SmolStr::new(format!("canon:{canon_id}"));
    let now = ctx.w.now();
    let mut settled = 0usize;
    for (id, slots) in matched {
        let Some(mut c) = live_contract(ctx, id) else {
            continue;
        };
        // E6b independent verify fold (R4): **evidence for the surface they agreed
        // to.** `amended_draft` clears `satisfied`, but a canon source names the run
        // and the row and no generation, so the next re-scan found the same record
        // and put the same attestation straight back — a seal made before the crew
        // joined would have paid the crew. A record older than the generation it
        // would attest is skipped, loudly, and the seal made *after* the amendment is
        // the one that counts.
        if recorded_at < c.generation_started_at {
            tracing::info!(
                contract = id.0,
                canon = canon_id,
                recorded_at,
                generation_started_at = c.generation_started_at,
                "economy: canon evidence predates the contract's current consent surface; skipped"
            );
            continue;
        }
        if c.satisfied.len() < c.consent.conditions.len() {
            c.satisfied.resize(c.consent.conditions.len(), None);
        }
        let mut wrote = false;
        for i in slots {
            let Some(slot) = c.satisfied.get_mut(i) else {
                continue;
            };
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
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return Vec::new();
    };
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
        c.consent
            .parties
            .iter()
            .any(|p| p == &me || auth.acts_for(&me, p))
    })
}

/// Whether this canon record answers `cond` on `c`.
///
/// A named **adjudicator replaces** each condition's own attestor set (08e answer 6,
/// §5.1 deviation 4), so when the surface has one it is the adjudicator that must be
/// a matching canon set — the same rule `@contract/attest` applies to a person.
fn answers(c: &EconomicContract, cond: &TypedCondition, kind: &str, source: &str) -> bool {
    let TypedCondition::Attestation { by } = cond else {
        return false;
    };
    let effective: &AttestorSet = c.consent.adjudicator.as_ref().unwrap_or(by);
    canon_set_accepts(effective, kind, source)
}

/// The contract as it stands right now (a clone, so nothing borrows the store).
fn live_contract(ctx: &Ctx<'_>, id: ContractId) -> Option<EconomicContract> {
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .contract(id)
        .cloned()
}
