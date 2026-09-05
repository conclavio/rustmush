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

use mush_core::Ctx;
use mush_economy::{
    ContractStateKind, EconomicEvent, EconomyObserver, EconomyStore, ObligationStatusKind,
    principal_key,
};
use mush_org::PrincipalId;

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
                tell(ctx, &orgs, &line);
            }
            EconomicEvent::ObligationChanged { id, status_kind } => {
                let orgs = obligation_orgs(ctx, *id);
                if orgs.is_empty() {
                    return;
                }
                let line = obligation_line(*id, *status_kind);
                tell(ctx, &orgs, &line);
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
/// one contract hears it once.
fn tell(ctx: &mut Ctx<'_>, orgs: &[PrincipalId], line: &str) {
    let mut told: std::collections::BTreeSet<mush_core::ObjId> = std::collections::BTreeSet::new();
    for org in orgs {
        for holder in mush_economy::principal_holders(ctx.w, org) {
            if told.insert(holder) {
                mush_economy::deliver_notice(ctx, holder, line);
            }
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

/// The kinds-only line for an obligation transition.
fn obligation_line(id: mush_economy::ObligationId, status: ObligationStatusKind) -> String {
    match status {
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
