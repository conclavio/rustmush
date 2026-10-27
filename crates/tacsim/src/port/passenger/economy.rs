//! Finite passenger purses and exact bilateral escrow. No model action moves money.
use super::{
    TravellerDef,
    store::{Booking, Phase},
};
use mush_core::{Ctx, ObjId, Registrar, World};
use mush_economy::{
    AcceptOutcome, Account, AccountId, AccountStatus, Amount, ConsentSurface, Consideration,
    ConsiderationSpec, ContractId, ContractRoles, ContractSpec, ContractState, ContractTerms,
    CreditPolicy, EconomicClock, EconomicDeadline, EconomicSchedule, EconomicVisibility,
    EconomyStore, EvidenceKind, Expiry, ExternalRef, GracePolicy, Hold, IdempotencyKey,
    IdempotencyRecord, InstrumentId, ObligationCause, ObligationSpec, OutcomeTerms, PayoutShare,
    Share, TransactionId, TypedCondition,
};
use mush_org::{PrincipalId, SystemPrincipalId};

#[derive(Clone, Default)]
struct Candidate(Option<Booking>);
pub(crate) fn register(reg: &mut Registrar<'_>) {
    reg.state(Candidate::default());
}
pub(crate) fn attachment_candidate(w: &World, actor: ObjId, id: ContractId) -> Option<Booking> {
    w.module_state::<Candidate>()?
        .0
        .as_ref()
        .filter(|r| r.captain == actor && r.contract == id.0)
        .cloned()
}
fn namespace(def: &TravellerDef) -> String {
    format!("tacsim.passenger.seed.{}", def.key)
}
fn account_name(def: &TravellerDef) -> String {
    format!("Passenger purse/{}", def.key)
}
fn treasury(w: &World, def: &TravellerDef) -> Option<(AccountId, SystemPrincipalId)> {
    let (_, port, map) = crate::port::market::ports(w)
        .into_iter()
        .find(|(_, port, _)| port.venue.as_str() == def.origin)?;
    let id = AccountId(map?.treasury?);
    let faucet = SystemPrincipalId::new(&port.treasury)?;
    let account = w.module_state::<EconomyStore>()?.snapshot().account(id)?;
    (account.owner == PrincipalId::System(faucet.clone())
        && account.status == AccountStatus::Active
        && account.instrument.as_str() == crate::economy::CREDIT)
        .then_some((id, faucet))
}
fn account_valid(account: &Account, npc: ObjId, def: &TravellerDef) -> bool {
    account.owner == PrincipalId::Object(npc)
        && account.instrument.as_str() == crate::economy::CREDIT
        && account.name.as_str() == account_name(def)
        && account.scope.is_none()
        && account.status == AccountStatus::Active
        && account.credit == CreditPolicy::forbid()
        && account.approval.is_none()
}
/// Discover the retained one-shot funding obligation, never infer a fresh seed from balance.
pub fn purse(w: &World, npc: ObjId, def: &TravellerDef) -> Option<AccountId> {
    let (debit, faucet) = treasury(w, def)?;
    let store = w.module_state::<EconomyStore>()?;
    let ns = namespace(def);
    let mut seeds = store.snapshot().obligations().filter(
        |o| matches!(&o.cause,ObligationCause::Ruleset{namespace} if namespace.as_str()==ns),
    );
    let seed = seeds.next()?;
    if seeds.next().is_some()
        || seed.debtor != PrincipalId::System(faucet.clone())
        || seed.minted_by != Some(PrincipalId::System(faucet))
        || seed.creditor != PrincipalId::Object(npc)
        || seed.debit_account != debit
        || seed.amount.minor() != def.funding_minor
        || seed.schedule.clock != EconomicClock::RealUtc
        || seed.schedule.cadence.is_some()
        || seed.schedule.maximum_catch_up != 1
        || seed.periods_settled > 1
        || seed.service_terms.is_some()
    {
        return None;
    }
    let account = store.snapshot().account(seed.credit_account)?;
    account_valid(account, npc, def).then_some(account.id)
}
/// Called by the ruleset tick only. The authored key has one lifetime seed, even after reuse.
pub fn ensure_purse(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    def: &TravellerDef,
) -> Result<AccountId, String> {
    if super::traveller(ctx.w, npc).as_ref() != Some(def) {
        return Err("passenger definition is not current".into());
    }
    if let Some(id) = purse(ctx.w, npc, def) {
        return Ok(id);
    }
    let (debit, faucet) = treasury(ctx.w, def).ok_or("passenger treasury unavailable")?;
    let existing = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or("economy unavailable")?;
        let ns = namespace(def);
        if store.snapshot().obligations().any(
            |o| matches!(&o.cause,ObligationCause::Ruleset{namespace} if namespace.as_str()==ns),
        ) {
            return Err(
                "passenger seed exists but is inconsistent; staff repair is required".into(),
            );
        }
        if def.funding_minor <= 0
            || store
                .config()
                .mint_cap(
                    store.snapshot(),
                    &InstrumentId::new(crate::economy::CREDIT).ok_or("credit unavailable")?,
                )
                .is_none_or(|cap| cap.minor() < def.funding_minor)
        {
            return Err("passenger seed exceeds the current mint limit".into());
        }
        let owner = PrincipalId::Object(npc);
        let mut accounts = store
            .snapshot()
            .accounts_of(&owner)
            .filter(|a| a.name.as_str() == account_name(def));
        let account = accounts.next();
        if accounts.next().is_some()
            || account.is_some_and(|a| {
                !account_valid(a, npc, def)
                    || !a.posted_balance.is_zero()
                    || !a.held_amount.is_zero()
                    || a.last_journal_seq != 0
                    || a.last_hold_seq != 0
            })
        {
            return Err("passenger purse is inconsistent; staff repair is required".into());
        }
        account.map(|a| a.id)
    };
    let id = match existing {
        Some(id) => id,
        None => {
            let id = ctx
                .w
                .module_state_mut::<EconomyStore>()
                .and_then(EconomyStore::alloc_account)
                .ok_or("passenger account allocator exhausted")?;
            mush_economy::open_account(
                ctx,
                Account {
                    id,
                    owner: PrincipalId::Object(npc),
                    instrument: InstrumentId::new(crate::economy::CREDIT)
                        .ok_or("credit unavailable")?,
                    scope: None,
                    name: account_name(def).into(),
                    status: AccountStatus::Active,
                    posted_balance: Amount::ZERO,
                    held_amount: Amount::ZERO,
                    credit: CreditPolicy::forbid(),
                    last_journal_seq: 0,
                    last_hold_seq: 0,
                    revision: 1,
                    approval: None,
                },
            );
            id
        }
    };
    let result = mush_economy::create_system_obligation(
        ctx,
        &faucet,
        ObligationSpec {
            debit_account: debit,
            credit_account: id,
            amount: Amount::from_minor(def.funding_minor),
            schedule: EconomicSchedule::new(EconomicClock::RealUtc, ctx.w.now(), None, 1),
            grace: GracePolicy::default(),
            notify_on_success: false,
            cause: ObligationCause::Ruleset {
                namespace: namespace(def).into(),
            },
        },
    );
    // If mint registration refuses, retain the harmless zero account for exact retry.
    if result.is_err() {
        ctx.out.pin_deferred();
    }
    result.map(|_| id).map_err(|e| e.to_string())
}
/// Narrow composition capability: only an authored traveller with its exact finite purse.
pub(crate) fn may_accept(w: &World, npc: ObjId) -> bool {
    super::traveller(w, npc).is_some_and(|d| purse(w, npc, &d).is_some())
}

pub(crate) fn fact_ref(row: &Booking, cancelled: bool) -> ExternalRef {
    // The fact attachment freezes the whole agreed trip, including non-money
    // terms. Names are outside economic consent and cannot provide this fence.
    let terms = serde_json::json!({
        "id":row.id,"npc":row.npc,"captain":row.captain,"hull":row.hull,
        "traveller_key":row.traveller_key,"origin":row.origin,"destination":row.destination,
        "purse":row.purse,"payee":row.payee,"fare_minor":row.fare_minor,
        "accepted_ic":row.accepted_ic,"deadline_ic":row.deadline_ic,"contract":row.contract
    });
    let digest = mush_core::CanonicalHash::for_canonical_value(
        "tacsim.passenger_terms.v1",
        1,
        &terms,
        MAX_TERMS_BYTES,
    )
    .map(|hash| hash.to_hex())
    .unwrap_or_else(|_| "unavailable".into());
    ExternalRef {
        provider: super::facts::PROVIDER.into(),
        kind: if cancelled {
            super::facts::CANCELLED
        } else {
            super::facts::DELIVERED
        }
        .into(),
        id: format!("{}/{digest}", row.id).into(),
        revision: Some(1),
    }
}
const MAX_TERMS_BYTES: usize = 4096;
pub(crate) fn contract_spec(row: &Booking) -> ContractSpec {
    ContractSpec {
        name: format!("Passenger #{} to {}", row.npc.0, row.destination).into(),
        scope: None,
        parties: vec![PrincipalId::Object(row.npc)],
        considerations: vec![ConsiderationSpec {
            account: AccountId(row.purse),
            amount: Amount::from_minor(row.fare_minor),
        }],
        outcomes: vec![
            OutcomeTerms {
                name: "passenger_delivered".into(),
                when: vec![0],
                payouts: vec![PayoutShare {
                    to: AccountId(row.payee),
                    share: Share::Exact(Amount::from_minor(row.fare_minor)),
                }],
                expense_treatment: mush_economy::ExpenseTreatment::None,
                asset_consequences: Vec::new(),
            },
            OutcomeTerms {
                name: "passenger_cancelled".into(),
                when: vec![1],
                payouts: vec![PayoutShare {
                    to: AccountId(row.purse),
                    share: Share::Exact(Amount::from_minor(row.fare_minor)),
                }],
                expense_treatment: mush_economy::ExpenseTreatment::None,
                asset_consequences: Vec::new(),
            },
        ],
        remainder_to: AccountId(row.purse),
        conditions: vec![
            TypedCondition::Fact {
                fact: fact_ref(row, false),
            },
            TypedCondition::Fact {
                fact: fact_ref(row, true),
            },
        ],
        adjudicator: None,
        evidence: vec![EvidenceKind::Fact],
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::WorldIc,
            at: row.deadline_ic.saturating_add(86400),
        }),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(row.captain)),
            payer: Some(PrincipalId::Object(row.npc)),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}
pub(crate) fn expected_consent(row: &Booking) -> ConsentSurface {
    let spec = contract_spec(row);
    ConsentSurface {
        fact_consent: None,
        managed: None,
        parties: vec![
            PrincipalId::Object(row.captain),
            PrincipalId::Object(row.npc),
        ],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: PrincipalId::Object(row.npc),
                account: AccountId(row.purse),
                amount: Amount::from_minor(row.fare_minor),
            }],
            deliverables: spec.deliverables,
            split: Vec::new(),
            remainder_to: spec.remainder_to,
            on_expiry: spec.on_expiry,
            outcomes: spec.outcomes,
        },
        conditions: spec.conditions,
        deadline: spec.deadline,
        visibility: spec.visibility,
        scope: spec.scope,
        adjudicator: spec.adjudicator,
        evidence: spec.evidence,
    }
}
pub(crate) fn valid(w: &World, row: &Booking) -> bool {
    if row.check().is_err() {
        return false;
    }
    let Some(c) = w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(ContractId(row.contract)))
    else {
        return false;
    };
    let mut consent = c.consent.clone();
    consent.fact_consent = None;
    c.issuer == PrincipalId::Object(row.captain)
        && c.amendments == 0
        && c.consent.fact_consent.is_some()
        && c.consent_intact()
        && consent == expected_consent(row)
        && c.acceptances
            .iter()
            .any(|a| a.party == c.issuer && a.consent_hash == c.consent_hash)
}
pub(crate) fn offered(w: &World, row: &Booking) -> bool {
    valid(w, row)
        && w.module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(ContractId(row.contract)))
            .is_some_and(|c| {
                c.state == ContractState::Offered && c.pending.is_none() && c.escrow.is_empty()
            })
}
pub fn active(w: &World, row: &Booking) -> bool {
    if !valid(w, row) {
        return false;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    c.state.is_live()
        && c.pending.is_none()
        && c.acceptances
            .iter()
            .any(|a| a.party == PrincipalId::Object(row.npc) && a.consent_hash == c.consent_hash)
        && c.escrow.iter().try_fold(0_i64, |n, id| {
            let h = store.snapshot().hold(*id)?;
            if h.account != AccountId(row.purse) || !h.status.is_held() {
                return None;
            }
            n.checked_add(h.amount.minor())
        }) == Some(row.fare_minor)
}
pub fn settled(w: &World, row: &Booking) -> bool {
    if !valid(w, row) {
        return false;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    let ContractState::Settled { .. } = c.state else {
        return false;
    };
    c.pending.is_none()
        && c.acceptances
            .iter()
            .any(|a| a.party == PrincipalId::Object(row.npc) && a.consent_hash == c.consent_hash)
        && terminal_receipt(store, c.id, &c.state).is_some()
}

fn terminal_key(
    id: ContractId,
    state: &ContractState,
) -> Option<(IdempotencyKey, Option<TransactionId>)> {
    match state {
        ContractState::Settled { transaction } => {
            Some((mush_economy::settle_key(id), Some(*transaction)))
        }
        ContractState::Cancelled | ContractState::Expired => {
            Some((mush_economy::refund_key(id), None))
        }
        _ => None,
    }
}

/// Warm exact current terminal receipts through the owner API. The read does
/// not post money; world-only admission remains closed until its answer lands.
pub(super) fn refresh_receipt(ctx: &mut Ctx<'_>, row: &Booking) {
    if !valid(ctx.w, row) {
        return;
    }
    let key = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().contract(ContractId(row.contract)))
        .filter(|contract| contract.pending.is_none() && !contract.escrow.is_empty())
        .and_then(|contract| terminal_key(contract.id, &contract.state))
        .map(|(key, _)| key);
    if let Some(key) = key {
        let _ = mush_economy::resolve_idempotency(ctx, &key);
    }
}

/// A terminal contract plus its exact, unreversed journal receipt proves the
/// unwind. A missing hold alone proves nothing: terminal hold projections are
/// deliberately collected, while the idempotency document is retained forever.
fn terminal_receipt<'a>(
    store: &'a EconomyStore,
    id: ContractId,
    state: &ContractState,
) -> Option<&'a IdempotencyRecord> {
    let (key, expected_transaction) = terminal_key(id, state)?;
    let receipt = store
        .snapshot()
        .idempotency(&key)
        .or_else(|| store.cold_key(&key)?.record.as_ref())?;
    (receipt.idempotency == key
        && expected_transaction.is_none_or(|tx| tx == receipt.transaction)
        && store.id_allocated(receipt.transaction)
        && !store.snapshot().is_reversed(receipt.transaction))
    .then_some(receipt)
}

fn released_hold(
    hold: Option<&Hold>,
    row: &Booking,
    receipt: &IdempotencyRecord,
    now: i64,
) -> bool {
    match hold {
        Some(hold) => {
            hold.account == AccountId(row.purse)
                && hold.amount.minor() == row.fare_minor
                && matches!(hold.purpose, mush_economy::HoldPurpose::Escrow { contract } if contract.0 == row.contract)
                && match hold.status {
                    mush_economy::HoldStatus::Held => false,
                    mush_economy::HoldStatus::Consumed { transaction, .. } => {
                        transaction == receipt.transaction
                    }
                    mush_economy::HoldStatus::Released | mush_economy::HoldStatus::Expired => true,
                }
        }
        None => now.saturating_sub(receipt.posted_at) > mush_economy::HOLD_GC_AFTER_SECS,
    }
}

pub fn replaceable(w: &World, row: &Booking) -> bool {
    if !valid(w, row) {
        return false;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    if !c.state.is_terminal() || c.pending.is_some() {
        return false;
    }
    // An unfunded offer has no monetary unwind to prove. Once the traveller
    // accepts, this fixed one-consideration contract must have exactly one hold.
    if c.escrow.is_empty() {
        return matches!(c.state, ContractState::Cancelled | ContractState::Expired)
            && !c
                .acceptances
                .iter()
                .any(|a| a.party == PrincipalId::Object(row.npc));
    }
    let Some(receipt) = terminal_receipt(store, c.id, &c.state) else {
        return false;
    };
    c.escrow.len() == 1
        && c.escrow
            .iter()
            .all(|id| released_hold(store.snapshot().hold(*id), row, receipt, w.now()))
}
/// Funded cancellations use a typed fact; an unfunded offer may be closed by
/// its actual captain. Retain the economy write even if later placement fails.
pub fn prepare_cancellation(ctx: &mut Ctx<'_>, row: &Booking) -> Result<(), String> {
    if ctx.actor != row.captain
        || super::store::get(ctx, row.id)?.as_ref() != Some(row)
        || !matches!(row.phase, Phase::Booked | Phase::Aboard)
        || !valid(ctx.w, row)
    {
        return Err("this passenger cancellation is not authorized".into());
    }
    if active(ctx.w, row) || replaceable(ctx.w, row) {
        return Ok(());
    }
    if !offered(ctx.w, row) {
        return Err("the passenger contract has an unsettled operation; retry shortly".into());
    }
    mush_economy::cancel_contract(ctx, ContractId(row.contract)).map_err(|e| e.to_string())?;
    ctx.out.pin_deferred();
    if !replaceable(ctx.w, row) {
        return Err("the passenger offer has not released; retry shortly".into());
    }
    Ok(())
}
pub fn offer(ctx: &mut Ctx<'_>, row: &Booking) -> Result<(), String> {
    row.check()?;
    let def = super::traveller(ctx.w, row.npc).ok_or("passenger unavailable")?;
    let payee = crate::port::credit::own_account(ctx.w, row.captain)
        .ok_or("captain credit account unavailable")?;
    if ctx.actor != row.captain
        || row.phase != Phase::Booked
        || row.revision != 1
        || row.journey.is_some()
        || row.traveller_key != def.key
        || row.origin != def.origin
        || row.destination != def.destination
        || row.fare_minor > def.max_fare_minor
        || purse(ctx.w, row.npc, &def) != Some(AccountId(row.purse))
        || payee.id != AccountId(row.payee)
        || ctx
            .w
            .module_state::<EconomyStore>()
            .is_none_or(|s| s.peek_next_contract() != row.contract)
    {
        return Err("passenger offer does not match its accepted quote".into());
    }
    ctx.w
        .module_state_mut::<Candidate>()
        .ok_or("passenger facts unavailable")?
        .0 = Some(row.clone());
    let offered = mush_economy::offer_contract(ctx, contract_spec(row));
    if let Some(c) = ctx.w.module_state_mut::<Candidate>() {
        c.0 = None;
    }
    let id = offered.map_err(|e| e.to_string())?;
    if id.0 != row.contract
        || !matches!(
            mush_economy::accept_contract(ctx, id),
            Ok(AcceptOutcome::Accepted { .. })
        )
    {
        let _ = mush_economy::cancel_contract(ctx, id);
        ctx.out.pin_deferred();
        return Err("passenger acceptance failed".into());
    }
    Ok(())
}
pub fn activate(ctx: &mut Ctx<'_>, row: &Booking) -> Result<bool, String> {
    if super::store::get(ctx, row.id)?.as_ref() != Some(row)
        || row.phase != Phase::Booked
        || ctx.w.world_ic_now().at > row.deadline_ic
        || !offered(ctx.w, row)
    {
        return Ok(false);
    }
    mush_economy::accept_contract_as(ctx, ContractId(row.contract), &PrincipalId::Object(row.npc))
        .map(|result| matches!(result, AcceptOutcome::Accepted { .. }))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn booking() -> Booking {
        Booking {
            schema: 1,
            id: 0,
            revision: 1,
            npc: ObjId(10),
            captain: ObjId(11),
            hull: ObjId(12),
            traveller_key: "pack/passenger.leila".into(),
            origin: "stickney".into(),
            destination: "ceres".into(),
            purse: 0,
            payee: 1,
            fare_minor: 50000,
            accepted_ic: 100,
            deadline_ic: 200,
            contract: 0,
            phase: Phase::Booked,
            journey: None,
            delivered: None,
            cancelled: None,
        }
    }

    #[test]
    fn consent_freezes_trip_identity_but_does_not_change_when_the_passenger_boards() -> TestResult {
        let row = booking();
        let original = expected_consent(&row);
        let mut changed = row.clone();
        changed.phase = Phase::Aboard;
        changed.revision += 1;
        check_eq!(expected_consent(&changed), original);
        changed.destination = "luna".into();
        check!(expected_consent(&changed) != original);
        changed = row;
        changed.hull = ObjId(13);
        check!(expected_consent(&changed) != original);
        check_eq!(original.visibility, EconomicVisibility::Parties);
        check!(original.conditions.len() == 2);
        Ok(())
    }

    #[test]
    fn collected_escrow_requires_an_old_terminal_receipt_and_never_hides_a_live_hold() -> TestResult
    {
        let row = booking();
        let receipt = IdempotencyRecord {
            idempotency: mush_economy::refund_key(ContractId(row.contract)),
            plan_hash: [1; 32],
            transaction: TransactionId(7),
            posted_at: 100,
        };
        let cutoff = receipt.posted_at + mush_economy::HOLD_GC_AFTER_SECS;
        check!(!released_hold(None, &row, &receipt, cutoff));
        check!(released_hold(None, &row, &receipt, cutoff + 1));
        let mut hold = Hold {
            id: mush_economy::HoldId(2),
            account: AccountId(row.purse),
            amount: Amount::from_minor(row.fare_minor),
            beneficiary: None,
            purpose: mush_economy::HoldPurpose::Escrow {
                contract: ContractId(row.contract),
            },
            placed_by: PrincipalId::Object(row.npc),
            holder: IdempotencyKey::new("contract", "0/accept"),
            expires: None,
            status: mush_economy::HoldStatus::Held,
            last_journal_seq: 1,
            revision: 1,
        };
        check!(!released_hold(Some(&hold), &row, &receipt, cutoff + 1));
        hold.status = mush_economy::HoldStatus::Released;
        check!(released_hold(Some(&hold), &row, &receipt, cutoff + 1));
        hold.account = AccountId(row.payee);
        check!(!released_hold(Some(&hold), &row, &receipt, cutoff + 1));
        check!(terminal_key(ContractId(row.contract), &ContractState::Active).is_none());
        check_eq!(
            terminal_key(ContractId(row.contract), &ContractState::Expired),
            Some((receipt.idempotency.clone(), None))
        );
        check_eq!(
            terminal_key(
                ContractId(row.contract),
                &ContractState::Settled {
                    transaction: receipt.transaction
                }
            ),
            Some((
                mush_economy::settle_key(ContractId(row.contract)),
                Some(receipt.transaction)
            ))
        );
        Ok(())
    }
}
