//! The **org inbox** (E6a step 1, DL-04; 08f §3.3 "who is told").
//!
//! The economy tells `PrincipalId::Object` parties what their contracts and
//! obligations did, and nobody else: an organization has no inbox, so an org payer
//! — E6a's design case — was told nothing when its run was crewed, settled, disputed
//! or refunded. This observer closes that from the composition root, where the
//! server already depends on both the economy and the org module:
//!
//! - it listens to the economy's **kinds-only** events (`ContractChanged`,
//!   `ObligationChanged` — ids and kinds, D-E4-13), and says exactly that much:
//!   "Contract #12 is active: every party has accepted." Never a term, an amount, an
//!   account, a party or a hold — the same discipline as the audience notice class;
//! - it says it to every **holder of `acts_for`** on the organization
//!   ([`mush_economy::principal_holders`]: `economy.account.post` on the root
//!   compartment, by grant), which is who may spend the org's money and see its
//!   escrow — the same predicate, so being told and being able to act never come
//!   apart;
//! - through [`mush_economy::deliver_notice`], so a connected holder reads it now and
//!   an absent one reads it at their next connect from a bounded, durable queue.
//!
//! A `System` principal is acted for by nobody (staff included), so a system party is
//! told nothing here; the staff channel (DL-05) is where its questions land. Object
//! parties are told by the economy itself and are not repeated here.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    ContractStateKind, EconomicEvent, EconomyObserver, EconomyStore, ObligationStatusKind,
    principal_key,
};
use mush_org::PrincipalId;

/// Whether one holder may be told about one record — the **view gate itself**, asked
/// per holder (E6a step 1 verify fold, D1).
///
/// `principal_holders` answers "who acts for this organization" (`economy.account.post`
/// on its root); the disclosure gates answer "who may look at this record". Those are
/// the same people today, and the D1 probe found the moment they were not: an
/// obligation's gate compared the debtor to the actor by identity, so an org's own
/// officer was told a line about a record `@obligation/show` then denied existed.
/// Rather than only fix the gate, the inbox now asks it: whatever either predicate
/// grows into, a line can never outrun what the reader may go and read.
enum Subject {
    Contract(mush_economy::ContractId),
    Obligation(mush_economy::ObligationId),
}

impl Subject {
    fn viewable_by(&self, w: &World, holder: ObjId) -> bool {
        match self {
            Subject::Contract(id) => mush_economy::may_view_contract(w, holder, *id),
            Subject::Obligation(id) => mush_economy::may_view_obligation(w, holder, *id),
        }
    }
}

/// The DL-04 observer. Wire one into
/// [`EconomyModule::with_observers`](mush_economy::EconomyModule::with_observers).
#[derive(Debug, Default, Clone, Copy)]
pub struct OrgInbox;

impl EconomyObserver for OrgInbox {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ContractChanged { id, state, .. } => {
                let Some(line) = contract_line(*id, *state) else {
                    return;
                };
                let orgs = contract_orgs(ctx, *id);
                if orgs.is_empty() {
                    return;
                }
                tell(ctx, &orgs, &Subject::Contract(*id), &line);
            }
            EconomicEvent::ObligationChanged { id, status_kind } => {
                let orgs = obligation_orgs(ctx, *id);
                if orgs.is_empty() {
                    return;
                }
                let line = obligation_line(ctx, *id, *status_kind);
                tell(ctx, &orgs, &Subject::Obligation(*id), &line);
            }
            // A raw transaction, a reservation, a title change and a staff page say
            // nothing an org's officers are owed here (the page is the staff
            // channel's, DL-05).
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::AssetTitleChanged { .. }
            | EconomicEvent::EscalationOpened { .. } => {}
        }
    }
}

/// The organization principals a contract names — its issuer and its parties —
/// deduplicated.
fn contract_orgs(ctx: &Ctx<'_>, id: mush_economy::ContractId) -> Vec<PrincipalId> {
    let Some(c) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(id).cloned())
    else {
        return Vec::new();
    };
    dedupe_orgs(std::iter::once(&c.issuer).chain(c.consent.parties.iter()))
}

/// The organization principals an obligation names — its debtor and creditor.
fn obligation_orgs(ctx: &Ctx<'_>, id: mush_economy::ObligationId) -> Vec<PrincipalId> {
    let Some(o) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id).cloned())
    else {
        return Vec::new();
    };
    dedupe_orgs([&o.debtor, &o.creditor].into_iter())
}

fn dedupe_orgs<'a>(principals: impl Iterator<Item = &'a PrincipalId>) -> Vec<PrincipalId> {
    let mut out: Vec<PrincipalId> = Vec::new();
    for p in principals {
        if !matches!(p, PrincipalId::Organization(_)) {
            continue;
        }
        if out.iter().any(|q| principal_key(q) == principal_key(p)) {
            continue;
        }
        out.push(p.clone());
    }
    out
}

/// One line to every holder of every org, once each — an officer of two orgs on
/// one contract hears it once — and **only to a holder who may view the record**
/// (D1). `principal_holders` is itself bounded ([`mush_economy::MAX_NOTICE_HOLDERS`]).
fn tell(ctx: &mut Ctx<'_>, orgs: &[PrincipalId], subject: &Subject, line: &str) {
    let mut told: std::collections::BTreeSet<ObjId> = std::collections::BTreeSet::new();
    let audience: Vec<(ObjId, PrincipalId)> = orgs
        .iter()
        .flat_map(|org| {
            mush_economy::principal_holders(ctx.w, org)
                .into_iter()
                .map(move |h| (h, org.clone()))
        })
        .filter(|(holder, _)| told.insert(*holder))
        .filter(|(holder, _)| subject.viewable_by(ctx.w, *holder))
        .collect();
    for (holder, org) in audience {
        // D6: the queue records whose business the line was, so a queued copy is
        // re-gated against live standing at the reader's next connect.
        mush_economy::deliver_notice_for(ctx, holder, &org, line);
        // Interface plan I11: a holder who is live right now also gets the
        // line on the notification surface; an offline holder's queued copy
        // is summarised there at their next connect instead (notice.rs), so
        // nobody is told twice.
        if ctx.w.has_live_conns(holder) {
            ctx.notice_info(holder, "inbox", line, None);
        }
    }
}

/// The kinds-only line for a contract transition, or `None` for one that is not an
/// officer's business: a draft being written (creation and an A′ amendment both
/// land there, and the re-offer is the line that matters) and the transient
/// `ReadyToSettle` step on the way to a settlement.
fn contract_line(id: mush_economy::ContractId, state: ContractStateKind) -> Option<String> {
    Some(match state {
        ContractStateKind::Draft | ContractStateKind::ReadyToSettle => return None,
        ContractStateKind::Offered => {
            format!("Contract #{id} has been offered to your organization (@contract/show #{id}).")
        }
        ContractStateKind::Active => {
            format!("Contract #{id} is active: every party has accepted.")
        }
        ContractStateKind::Settled => format!("Contract #{id} has settled."),
        ContractStateKind::Disputed => {
            format!("Contract #{id} is disputed: its escrow is frozen until it is resolved.")
        }
        ContractStateKind::Cancelled => format!("Contract #{id} ended; its escrow was released."),
        ContractStateKind::Expired => format!("Contract #{id} lapsed."),
    })
}

/// Whether obligation `id` reads as **brand new** rather than resumed (E6a step 1
/// verify fold, D9).
///
/// `ObligationChanged` carries a status kind and no previous status, so `Active`
/// covers both "created" and "a suspension lifted". Every creation path builds the
/// record with an empty history and zero counters, and every route back into `Active`
/// leaves a `Resumed`/`Held`/`Escalated` event behind it — so an untouched tail is
/// the created case. It costs a wrong word at worst, never a disclosure: the line is
/// kinds-only either way.
fn is_new_obligation(ctx: &Ctx<'_>, id: mush_economy::ObligationId) -> bool {
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id))
        .is_some_and(|o| {
            o.history.is_empty()
                && o.periods_settled == 0
                && o.settled_total == 0
                && o.missed_total == 0
                && o.forgiven_total == 0
        })
}

/// The kinds-only line for an obligation transition.
fn obligation_line(
    ctx: &Ctx<'_>,
    id: mush_economy::ObligationId,
    status: ObligationStatusKind,
) -> String {
    match status {
        ObligationStatusKind::Active if is_new_obligation(ctx, id) => {
            format!("Obligation #{id} has been created (@obligation/show #{id}).")
        }
        ObligationStatusKind::Active => format!("Obligation #{id} is back on schedule."),
        ObligationStatusKind::Grace => {
            format!("Obligation #{id} missed a period and is in grace.")
        }
        ObligationStatusKind::Delinquent => format!("Obligation #{id} is delinquent."),
        ObligationStatusKind::Suspended => {
            format!("Obligation #{id} is suspended pending a decision.")
        }
        ObligationStatusKind::Satisfied => format!("Obligation #{id} is satisfied."),
        ObligationStatusKind::Cancelled => format!("Obligation #{id} is cancelled."),
    }
}
