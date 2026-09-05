//! The **canon evidence bridge** (E4c, 08c §6.1 item 3; D-E4-12, review finding E15).
//!
//! The economy announces what it did as [`EconomicEvent`]s carrying ids and kinds
//! only. This is the one observer that turns some of them into world-visible
//! chronicle, and it lives here — in the composition root — for the reason the whole
//! seam exists: `mush-economy` must not depend on `mush-canon`, and `mush-server`
//! already depends on both.
//!
//! ## What reaches canon, and what never does
//!
//! **Only `Public`.** A contract's `visibility` is inside its consent surface, so a
//! `Public` contract is one every party *accepted publication of* before they funded
//! it. `Parties` (the default) is nobody's business; `Staff` is a staff channel; and
//! an **audience list** is emphatically not public — doc 10 T6's audience is a class
//! told that something is happening, not a broadcast, and republishing an operation
//! to the chronicle because its eligible defenders were told about it would invert
//! the whole point of the variant. `EconomicVisibility::is_public` is the predicate,
//! and it matches `Public` alone.
//!
//! **Existence-level detail only.** The line says a public contract settled (or that
//! an obligation it created went delinquent) and names the parties who consented to
//! being named. No amount, no account, no split, no description, no hold — the event
//! does not carry them, and this bridge deliberately does not go and look them up.
//!
//! ## Keyed on the landed transaction
//!
//! A settlement can be produced twice: the sweep retries a `ReadyToSettle` contract
//! whose state write was lost and its post resolves `AlreadyPosted`, and boot
//! finishes a half-done transition by reading the same deterministic key out of the
//! idempotency index. Both land on the **same transaction id**, which is why
//! `ContractChanged` carries it — the draft's `source` is `economy.tx:{id}`, and a
//! draft whose source is already in the chronicle is not committed again. (The check
//! reads canon's RAM tail, which is where recent events live; canon events are sparse
//! enough that a settlement's own line is still there for a very long time. A tail
//! that has rolled past it is the documented limit of this check — the same limit
//! canon's own id-allocator safety net has.)
//!
//! ## Re-entrancy: the bridge never calls back into the economy
//!
//! It commits canon and does nothing else. `mush_canon::commit` is documented as
//! **terminal**: it appends, then emits `CanonRecorded` synchronously, and the
//! economy's own attestation observer is subscribed to that — so a bridge that also
//! posted, settled, or wrote an economy document would be re-entering, from inside an
//! economy event callback, the machinery that produced the event. The chain that does
//! exist (settlement → event → canon → `CanonRecorded` → a *different* contract's
//! attestation → its settlement) is finite because every step consumes a distinct
//! contract and carries a new transaction id, and it is bounded anyway by the
//! economy's own [`mush_economy::MAX_OBSERVER_DEPTH`]. If this bridge ever needs the
//! economy to *do* something, that work belongs in the **next step** — a command, or
//! the economy sweep — because by the time an observer runs the post is installed and
//! its batch is closing, and nothing added here can be part of the decision it
//! records.

use mush_canon::{CanonDraft, CanonScope, CanonState};
use mush_core::{Ctx, ObjId};
use mush_economy::{
    EconomicEvent, EconomyObserver, EconomyStore, ObligationCause, ObligationStatusKind,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

/// The canon kind a settled public contract records.
pub const KIND_SETTLED: &str = "economy.settled";
/// The canon kind a public contract's delinquent obligation records.
pub const KIND_DELINQUENT: &str = "economy.delinquent";

/// The E4c evidence bridge. Wire one into
/// [`EconomyModule::with_observers`](mush_economy::EconomyModule::with_observers).
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonEvidenceBridge;

impl EconomyObserver for CanonEvidenceBridge {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ContractChanged {
                id,
                state,
                transaction,
            } => {
                if !matches!(state, mush_economy::ContractStateKind::Settled) {
                    return;
                }
                // No landed transaction means no key; without a key a replay would
                // publish twice, so nothing is published at all.
                let Some(tx) = transaction else { return };
                let Some(parties) = public_contract_parties(ctx, *id) else {
                    return;
                };
                // **The durable marker is the replay guard** (E4c verify fold,
                // V2+V3+V5). Canon's RAM tail rolls, and a settlement republished
                // after it had rolled would double the line; the contract document
                // carries the answer forever.
                let published = ctx
                    .w
                    .module_state::<EconomyStore>()
                    .and_then(|s| s.snapshot().contract(*id))
                    .and_then(|c| c.canon_published.clone());
                if let Some(canon_ref) = published {
                    tracing::debug!(contract = id.0, %canon_ref, "economy bridge: already chronicled");
                    return;
                }
                let Some(canon_id) = commit_once(
                    ctx,
                    KIND_SETTLED,
                    format!("economy.tx:{tx}"),
                    format!("A public contract (#{id}) settled."),
                    parties,
                    serde_json::json!({ "contract": id.0 }),
                ) else {
                    // A commit that failed leaves the marker UNSET on purpose: the
                    // economy sweep re-announces an unmarked public settlement a
                    // sweep period later, so the retry is a mechanism rather than a
                    // hope. Loudly, because a chronicle line went missing.
                    tracing::error!(
                        contract = id.0,
                        transaction = tx.0,
                        "economy bridge: a public settlement could not be committed to canon; the \
                         sweep will retry it"
                    );
                    return;
                };
                mush_economy::mark_canon_published(ctx, *id, &format!("canon:{}", canon_id.0));
            }
            EconomicEvent::ObligationChanged { id, status_kind } => {
                if *status_kind != ObligationStatusKind::Delinquent {
                    return;
                }
                // Only an obligation a **public contract** created: a lease's rent is
                // otherwise as private as the lease, and E4b1's obligations are not
                // consent surfaces at all.
                let Some((contract, debtor, missed)) = public_contract_of_obligation(ctx, *id)
                else {
                    return;
                };
                // **Once per missed period, not once ever** (E4c verify fold, V4).
                // The key was `economy.obligation:{id}`, so the *second* time a
                // tenant fell behind on the same lease the guard read its own first
                // line and published nothing: the chronicle said a lease went
                // delinquent in March and never that it went delinquent again in
                // April, which is exactly the fact a public lease's readers want.
                let _ = commit_once(
                    ctx,
                    KIND_DELINQUENT,
                    format!("economy.obligation:{id}:{missed}"),
                    format!("An obligation of a public contract (#{contract}) went delinquent."),
                    debtor,
                    serde_json::json!({ "contract": contract.0, "obligation": id.0 }),
                );
            }
            // A raw transaction, a reservation, a title change and a staff page say
            // nothing about consent to publication, so none of them is chronicle.
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::AssetTitleChanged { .. }
            | EconomicEvent::EscalationOpened { .. } => {}
        }
    }
}

/// The object parties of a contract whose consented visibility is `Public`, or
/// `None` for any other visibility (including an audience list) or no such contract.
fn public_contract_parties(ctx: &Ctx<'_>, id: mush_economy::ContractId) -> Option<Vec<ObjId>> {
    let c = ctx
        .w
        .module_state::<EconomyStore>()?
        .snapshot()
        .contract(id)?;
    if !c.consent.visibility.is_public() {
        return None;
    }
    Some(object_parties(&c.consent.parties))
}

/// The contract behind an obligation, when that contract is `Public` - with the
/// **debtor alone** and how many periods have been missed.
///
/// D7 (E4c verify fold): a delinquency line used to name every party of the contract
/// the obligation came from. "Who is behind on their rent" is a fact about the
/// debtor; the other runners on a job that happened to create the lease consented to
/// the *settlement* being public, not to being listed beside somebody else's
/// default. The debtor is resolved from the **debit account's owner**, which is the
/// journal's answer to who owes rather than the surface's list of who signed.
fn public_contract_of_obligation(
    ctx: &Ctx<'_>,
    id: mush_economy::ObligationId,
) -> Option<(mush_economy::ContractId, Vec<ObjId>, u32)> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let o = store.snapshot().obligation(id)?;
    let ObligationCause::Contract(contract) = o.cause else {
        return None;
    };
    let c = store.snapshot().contract(contract)?;
    if !c.consent.visibility.is_public() {
        return None;
    }
    let debtor = store
        .snapshot()
        .account(o.debit_account)
        .map(|a| object_parties(std::slice::from_ref(&a.owner)))
        .unwrap_or_default();
    Some((contract, debtor, o.missed_total))
}

/// Only the parties canon can name: `entities` is a list of world objects, so an
/// organization or system party contributes nothing rather than a fabricated id.
fn object_parties(parties: &[PrincipalId]) -> Vec<ObjId> {
    parties
        .iter()
        .filter_map(|p| match p {
            PrincipalId::Object(o) => Some(*o),
            _ => None,
        })
        .collect()
}

/// Commit one chronicle line unless the chronicle already has one with this
/// `source` — the replay guard described in this module's header.
fn commit_once(
    ctx: &mut Ctx<'_>,
    kind: &'static str,
    source: String,
    summary: String,
    entities: Vec<ObjId>,
    detail: serde_json::Value,
) -> Option<mush_canon::CanonId> {
    let already = ctx.w.module_state::<CanonState>().is_some_and(|st| {
        st.recent()
            .iter()
            .any(|e| e.kind == kind && e.source == source)
    });
    if already {
        tracing::debug!(
            kind,
            source,
            "economy bridge: that consequence is already canon"
        );
        return None;
    }
    let ic_time = ctx.w.game_now_unix();
    mush_canon::commit(
        ctx,
        CanonDraft {
            kind: SmolStr::new(kind),
            summary,
            entities,
            // The contract said `Public`; the chronicle line says the same.
            scope: CanonScope::Public,
            ic_time,
            source,
            detail,
        },
    )
}
