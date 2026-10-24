//! R8a economy adapter. Protected reservations and ordinary journal postings;
//! no payer impersonation, duplicate balance or independently writable entitlement.
use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicCause, EconomyPlan,
    EconomyStore, Hold, HoldId, HoldOp, HoldPurpose, HoldStatus, IdempotencyKey, PostOutcome,
    RequestedLine,
};
use mush_org::PrincipalId;
use mush_propagation::commerce::{Purchase, Tariff};

pub const PURPOSE: &str = "relay.message";
const ACCOUNT: &str = "Malapert Exchange/operating";
fn purpose(p: &Purchase) -> HoldPurpose {
    HoldPurpose::Reservation {
        kind: PURPOSE.into(),
        id: p.id,
    }
}
fn key(kind: &str, id: u64) -> IdempotencyKey {
    IdempotencyKey::new(kind, id.to_string())
}
fn cause(id: u64) -> EconomicCause {
    EconomicCause::Internal {
        kind: PURPOSE.into(),
        id,
    }
}
fn held<'a>(w: &'a World, p: &Purchase) -> Option<&'a Hold> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .hold(HoldId(p.hold?))
        .filter(|h| {
            h.purpose == purpose(p)
                && h.account == AccountId(p.payer)
                && h.amount.minor() == p.total_minor
                && h.beneficiary == Some(PrincipalId::Organization(mush_org::OrgId(p.operator)))
                && h.placed_by == PrincipalId::Object(p.issuer)
        })
}
pub fn reserved(w: &World, p: &Purchase) -> bool {
    w.module_state::<EconomyStore>()
        .is_some_and(|s| !s.is_frozen())
        && held(w, p).is_some_and(|h| h.status == HoldStatus::Held)
}
pub fn payer(w: &World, actor: ObjId, id: u64) -> Result<(), String> {
    let account = AccountId(id);
    let valid = mush_economy::may_view_account(w, actor, account)
        && w.module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(account))
            .is_some_and(|a| {
                a.status == AccountStatus::Active
                    && a.instrument.as_str() == crate::economy::CREDIT
                    && !matches!(a.owner, PrincipalId::System(_))
            });
    if !valid {
        return Err("No usable payer account.".into());
    }
    Ok(())
}
pub fn reserve(ctx: &mut Ctx<'_>, p: &Purchase) -> Result<u64, String> {
    if ctx.actor != p.issuer {
        return Err("No such quote.".into());
    }
    payer(ctx.w, ctx.actor, p.payer)?;
    let expected = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or("Economy unavailable")?
        .peek_next_hold();
    let mut plan = EconomyPlan::new(
        key("relay.reserve", p.id),
        PrincipalId::Object(ctx.actor),
        cause(p.id),
        vec![],
    );
    plan.description = format!(
        "relay quote {}: {} units {:?}, tariff v{}",
        p.id, p.units, p.class, p.tariff
    );
    plan.hold_ops.push(HoldOp::Place {
        account: AccountId(p.payer),
        amount: Amount::from_minor(p.total_minor),
        beneficiary: Some(PrincipalId::Organization(mush_org::OrgId(p.operator))),
        purpose: purpose(p),
        expires: None,
    });
    match mush_economy::post(ctx,&plan).map_err(|e|format!("Reservation refused: {e}"))? {
        PostOutcome::Posted(_)=>Ok(expected),
        PostOutcome::AlreadyPosted(_)=>Err("Reservation already journaled but purchase missing; restore committed transport state.".into()),
        PostOutcome::NeedsApproval(id)=>Err(format!("Account approval #{id} required; approve and retry this quote.")),
    }
}
pub fn settle(ctx: &mut Ctx<'_>, p: &Purchase, capture: bool) -> Result<u64, String> {
    let h = held(ctx.w, p)
        .cloned()
        .ok_or("Reservation missing or inconsistent; restore committed records")?;
    match h.status {
        HoldStatus::Consumed { transaction, .. } if capture => return Ok(transaction.0),
        HoldStatus::Held => {}
        _ => return Err("Reservation is not held.".into()),
    }
    let lines = if capture {
        vec![
            RequestedLine {
                account: h.account,
                delta: Amount::from_minor(-p.total_minor),
                memo: PURPOSE.into(),
            },
            RequestedLine {
                account: AccountId(p.payee),
                delta: h.amount,
                memo: PURPOSE.into(),
            },
        ]
    } else {
        vec![]
    };
    let mut plan = EconomyPlan::new(
        key("relay.settle", p.id),
        h.placed_by.clone(),
        cause(p.id),
        lines,
    );
    plan.description = format!(
        "relay quote {} {}",
        p.id,
        if capture {
            "consumed at first Exchange transmission"
        } else {
            "released unconsumed"
        }
    );
    plan.hold_ops.push(if capture {
        HoldOp::Capture {
            hold: h.id,
            amount: h.amount,
        }
    } else {
        HoldOp::Release { hold: h.id }
    });
    match mush_economy::settle_engine_reservation(
        ctx,
        &plan,
        h.id,
        capture.then_some(AccountId(p.payee)),
    )
    .map_err(|e| format!("Settlement refused: {e}"))?
    {
        PostOutcome::Posted(tx) | PostOutcome::AlreadyPosted(tx) => Ok(tx.0),
        PostOutcome::NeedsApproval(id) => Err(format!("Settlement approval #{id} required")),
    }
}
/// Called after production relay content and economy load. The market document
/// pins the account id; losing it never authorizes a second capitalization.
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(registry) = mush_propagation::routing::registry(ctx.w) else {
        return Ok(());
    };
    let operator = registry.operator;
    let owner = PrincipalId::Organization(operator);
    let scope = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().org(operator))
        .ok_or("Exchange organization missing")?
        .root_scope;
    let tariff: Tariff = serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(crate::relay_service::JSON)
            .map_err(|e| e.to_string())?["retail"]
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    tariff.validate()?;
    let store = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or("Exchange economy missing")?;
    // Reverse audit as well: a lost purchase must not strand an engine-owned
    // hold. This derived purpose index avoids scanning unrelated economy data.
    let live: Vec<_> = store
        .snapshot()
        .held_reservations(PURPOSE)
        .take(mush_propagation::commerce::MAX_PURCHASES + 1)
        .collect();
    if live.len() > mush_propagation::commerce::MAX_PURCHASES {
        return Err("relay live reservations exceed purchase bound".into());
    }
    for h in live {
        let HoldPurpose::Reservation { id, .. } = &h.purpose else {
            return Err("relay reservation index identity drift".into());
        };
        let p = mush_propagation::commerce::current(ctx.w).and_then(|m| m.purchases.get(id));
        if !p.is_some_and(|p| {
            p.phase == mush_propagation::commerce::Phase::Reserved
                && held(ctx.w, p).is_some_and(|matched| matched.id == h.id)
        }) {
            return Err("orphaned relay reservation; restore committed purchase records".into());
        }
    }
    let currency = mush_economy::InstrumentId::new(crate::economy::CREDIT).ok_or("credit id")?;
    let found = if let Some(m) = mush_propagation::commerce::current(ctx.w) {
        store.snapshot().account(AccountId(m.payee)).cloned()
    } else {
        let accounts: Vec<_> = store.snapshot().accounts_of(&owner).take(65).collect();
        if accounts.len() > 64 {
            return Err("Exchange account inventory exceeds initialization bound".into());
        }
        accounts.into_iter().find(|a| a.name == ACCOUNT).cloned()
    };
    if let Some(market) = mush_propagation::commerce::current(ctx.w) {
        let a = found
            .as_ref()
            .ok_or("committed Exchange operating account missing")?;
        if a.id.0 != market.payee
            || a.owner != owner
            || a.instrument != currency
            || a.scope != Some(scope)
        {
            return Err("Exchange account identity drift".into());
        }
        for p in market.purchases.values() {
            match p.phase {
                mush_propagation::commerce::Phase::Reserved
                    if !held(ctx.w, p).is_some_and(|h| h.status == HoldStatus::Held) =>
                {
                    return Err("relay purchase reservation missing or inconsistent".into());
                }
                mush_propagation::commerce::Phase::Consumed => {
                    if let Some(h) = held(ctx.w, p) {
                        if !matches!(h.status,HoldStatus::Consumed{transaction,amount} if Some(transaction.0)==p.transaction && amount.minor()==p.total_minor)
                        {
                            return Err("relay purchase settlement disagrees with journal".into());
                        }
                    } else if store
                        .snapshot()
                        .hold(HoldId(p.hold.expect("validated purchase")))
                        .is_some()
                    {
                        return Err("relay settled hold identity drift".into());
                    }
                    // Terminal hold projections are collected after seven real
                    // days. Their absence must not invalidate the transport
                    // receipt committed atomically with the original payment.
                }
                _ => {}
            }
        }
        if market.capital_transaction.is_some() {
            return mush_propagation::commerce::install(ctx, operator.0, a.id.0, tariff);
        }
    }
    if found.is_some() && mush_propagation::commerce::current(ctx.w).is_none() {
        return Err(
            "Exchange account exists without its market; restore committed transport state".into(),
        );
    }
    // Preserve the economy's repair/read-only boot mode. Its own post seam
    // denies money changes, including retail reservations and capture.
    if store.is_frozen() {
        return Ok(());
    }
    let lane = mush_core::CanonicalId::new(crate::definitions::content::LANE_PACK)
        .map_err(|e| e.to_string())?;
    let port = mush_core::CanonicalId::new("ceres_port").map_err(|e| e.to_string())?;
    let house = mush_core::CanonicalId::new("ceres_clearing").map_err(|e| e.to_string())?;
    let mapped = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or("port map missing")?
        .port_map(&lane, &port);
    let crate::port::PortMapState::Present(map) = mapped else {
        return Err("Exchange clearing port map unavailable".into());
    };
    let row = map
        .orgs
        .get(&house)
        .ok_or("Exchange clearing institution missing")?;
    let funding = AccountId(row.account.ok_or("Exchange clearing account missing")?);
    let source = store
        .snapshot()
        .account(funding)
        .ok_or("Exchange clearing account missing")?;
    if source.instrument != currency
        || source.owner != PrincipalId::Organization(mush_org::OrgId(row.org))
        || source.scope != Some(mush_org::OrgScopeId(row.root_scope))
    {
        return Err("Exchange clearing account identity drift".into());
    }
    let account = if let Some(a) = found {
        a.id
    } else {
        let id = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .ok_or("economy missing")?
            .alloc_account()
            .ok_or("account ids exhausted")?;
        mush_economy::open_account(
            ctx,
            Account {
                id,
                owner: owner.clone(),
                instrument: currency,
                scope: Some(scope),
                name: ACCOUNT.into(),
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
    };
    mush_propagation::commerce::install(ctx, operator.0, account.0, tariff)?;
    let capitalization = IdempotencyKey::new("relay.capital", "v1");
    if !mush_economy::idempotency_resolved(ctx, &capitalization)
        && ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().account(funding))
            .and_then(Account::available)
            .is_none_or(|a| a.minor() < 100_000)
    {
        // The port's existing finite tranches run on the economy cadence. Retail
        // is explicitly unavailable until that producer has funded this transfer.
        return Ok(());
    }
    let mut plan = EconomyPlan::new(
        capitalization,
        PrincipalId::Object(ctx.actor),
        EconomicCause::Internal {
            kind: "relay.capital".into(),
            id: operator.0,
        },
        vec![
            RequestedLine {
                account: funding,
                delta: Amount::from_minor(-100_000),
                memo: "Concord charter capital, Ceres clearing".into(),
            },
            RequestedLine {
                account,
                delta: Amount::from_minor(100_000),
                memo: "Exchange finite operating capital".into(),
            },
        ],
    );
    plan.description =
        "Exchange charter capitalization: 1000 cr transferred once from existing clearing funds"
            .into();
    match mush_economy::post(ctx, &plan).map_err(|e| format!("Exchange capitalization: {e}"))? {
        PostOutcome::Posted(tx) | PostOutcome::AlreadyPosted(tx) => {
            mush_propagation::commerce::capitalized(ctx, tx.0);
            Ok(())
        }
        PostOutcome::NeedsApproval(_) => {
            Err("Exchange capitalization needs existing-account approval".into())
        }
    }
}
pub fn register(reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
    reg.tick(
        "relay.capital",
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(5)),
        std::sync::Arc::new(capital_tick),
    )?;
    Ok(())
}

fn capital_tick(ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>) {
    if mush_propagation::commerce::current(ctx.w).is_some_and(|m| m.capital_transaction.is_some()) {
        return;
    }
    if let Err(error) = boot(ctx) {
        tracing::warn!(%error, "relay capitalization unavailable");
    }
}
