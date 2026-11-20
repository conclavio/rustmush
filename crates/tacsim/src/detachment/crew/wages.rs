//! **Crew wages and crew for hire** (doc 48 cut EX.7; Q-48-14, -33, -54).
//!
//! - **The wage** ([`RATING_CR_PER_DAY`] or [`OFFICER_CR_PER_DAY`] an IC day, Q-48-33,
//!   provisional): every standing crew contract owes it, from the employer's own account to the
//!   hiring port's authority (the port def's `authority` org, which keeps the hiring hall), as one
//!   player-owed obligation (`mush_economy::create_player_obligation`, EX.4's class) kept under
//!   `tacsim.crew.wage.<npc>` ([`wage_namespace`]). A rating is a contract scoped to operate a unit;
//!   an officer is one scoped to command a detachment. At 24:1 that is 240 cr and 480 cr a real
//!   day.
//! - **Grace** is EX.4's: [`mush_economy::PLAYER_OWED_MIN_GRACE`] periods, 28 IC days (28 real
//!   hours). A day the employer cannot pay accrues as arrears, a one-term debt under the same
//!   name, paid when the account is funded. **A missed wage never ends the contract**: nothing
//!   here unseats, unhires or moves the crew member.
//! - **The wage ends with the contract** (doc 42's capture exit, `tacsim_crew_captured`); what
//!   accrued stands as its debt, and is collected at a port fire once the employer's account
//!   covers it (`collect_ended`, doc 48 cut EX.7b).
//! - **A discharge settles it** (doc 48 cut EX.7b, `settle_final`): the employer's
//!   `+sortie/discharge` charges the IC day in progress whole beside the arrears, pays the lot if
//!   the account covers it, and ends the wage at once.
//! - **Crew for hire** (Q-48-14's second half): a player lists themselves with
//!   `+sortie/available`, and while they stand in that port's station they are an `Offered` crew
//!   service on its job board ([`services`]), asking the scale wage as an opening figure. The
//!   listing is a record of the player's own, never a contract: an employer contacts them, and
//!   any agreement is theirs to draft.
//!
//! `tick` keeps every wage in step with the contracts on each port fire.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Attr, AttrFlags, Ctx, ObjId, ObjKind, World};
use mush_economy::{AccountId, Amount, EconomyStore, Obligation, ObligationStatus};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{CrewContract, CrewDuty, CrewHandles, HireTerms, NpcMissionScope};
use crate::port::apply::{credit_scale, minor_of};

/// A rating's wage, whole credits an IC day (Q-48-33, provisional).
pub const RATING_CR_PER_DAY: u64 = 10;
/// An officer's wage, whole credits an IC day (Q-48-33, provisional).
pub const OFFICER_CR_PER_DAY: u64 = 20;
/// A wage's namespace prefix.
pub const WAGE_NS_PREFIX: &str = "tacsim.crew.wage.";
/// The attribute a player's own crew listing is kept in.
pub const LISTING_ATTR: &str = "crew:listing";
/// IC seconds in an IC day.
const IC_DAY: i64 = 86_400;

/// The hiring terms' wage, after an offer: `{wage}`.
pub const OFFER_WAGE: &str = "; wage {wage} an IC day";
/// The offers' footer: `{rating}` and `{officer}`.
pub const OFFERS_WAGES: &str = "A hire's wage is {rating} an IC day for a rating and {officer} for an officer, paid to the hiring port.";
/// The offers' second footer.
pub const OFFERS_ABOARD: &str = "Hiring does not move the character aboard.";
/// A contract's wage: `{wage}`, `{payee}` and `{arrears}`.
pub const WAGE_LINE: &str =
    "Wage: {wage} an IC day to {payee}, from the employer's account; {arrears} in arrears.";
/// A contract with no wage open.
pub const WAGE_NONE: &str = "No wage is charged: the hiring port has no authority to pay it to.";
/// `+sortie/available` usage.
pub const LIST_USAGE: &str = "Usage: +sortie/available <duty>[,<duty>] [officer]; duties: mining, combat, recovery, technician.";
/// A listing made: `{rank}`, `{duties}`, `{port}` and `{wage}`.
pub const LISTED: &str =
    "Listed for crew hire: {rank} ({duties}) at {port}, asking {wage} an IC day.";
/// A listing shown: `{rank}`, `{duties}` and `{port}`.
pub const LISTING_SHOW: &str =
    "Listed for crew hire: {rank} ({duties}) at {port}, shown there on its board.";
/// No listing to show or withdraw.
pub const NOT_LISTED: &str = "You are not listed for crew hire.";
/// A listing withdrawn.
pub const UNLISTED: &str = "You are no longer listed for crew hire.";
/// Listing away from a port's station.
pub const LIST_AT_PORT: &str = "List yourself for crew hire from a port's station.";
/// The board's subject for a listing: `{rank}` and `{duties}`.
pub const BOARD_SUBJECT: &str = "{rank} for hire ({duties})";
/// The board's take for a listing: `{name}`.
pub const BOARD_TAKE: &str = "page {name}";
/// Refusal when hiring at a waged port without an active credit account.
pub const HIRE_NO_ACCOUNT: &str = "Open an active credit account before hiring crew at this port.";
/// Refusal when hiring with a delinquent obligation on the employer's account.
pub const HIRE_DELINQUENT: &str = "Settle your delinquent obligations before hiring crew.";
/// Refusal when hiring while unpaid crew wage debts stand on the employer's account.
pub const HIRE_UNPAID_WAGES: &str = "Settle your unpaid crew wages before hiring again.";

/// HIGH-69: whether `employer` is in good standing to accept a hire on `terms`.
///
/// At a port with no wage authority (`payee` is `None`), no wage or debt is ever opened,
/// so hiring stays unrestricted. At a waged port, the employer must hold an active credit
/// account with no delinquent obligations and no standing unpaid crew wage debts.
pub(super) fn hiring_allowed(
    w: &World,
    employer: ObjId,
    terms: &HireTerms,
) -> Result<(), &'static str> {
    if payee(w, terms).is_none() {
        return Ok(());
    }
    hiring_standing(w, employer)
}

fn hiring_standing(w: &World, employer: ObjId) -> Result<(), &'static str> {
    if crate::port::credit::own_account(w, employer).is_none() {
        return Err(HIRE_NO_ACCOUNT);
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return Err(HIRE_NO_ACCOUNT);
    };
    let debtor = PrincipalId::Object(employer);
    for obligation in store.snapshot().obligations() {
        if obligation.debtor != debtor {
            continue;
        }
        if obligation.status == ObligationStatus::Delinquent {
            return Err(HIRE_DELINQUENT);
        }
        if obligation.schedule.cadence.is_none()
            && live_debt(obligation)
            && matches!(
                &obligation.cause,
                mush_economy::ObligationCause::Ruleset { namespace }
                    if namespace.starts_with(WAGE_NS_PREFIX)
            )
        {
            return Err(HIRE_UNPAID_WAGES);
        }
    }
    Ok(())
}

/// **The namespace a hire's wage is kept under**: its obligation and its arrears.
#[must_use]
pub fn wage_namespace(npc: ObjId) -> String {
    format!("{WAGE_NS_PREFIX}{}", npc.0)
}

/// Whether `terms` are an officer's: scoped to command a detachment.
#[must_use]
pub fn officer(terms: &HireTerms) -> bool {
    terms.mission_scope == NpcMissionScope::DetachmentCommander
}

/// **The wage a hire on `terms` owes an IC day**, minor units of the credit.
#[must_use]
pub fn daily_wage(w: &World, terms: &HireTerms) -> Option<Amount> {
    let cr = if officer(terms) {
        OFFICER_CR_PER_DAY
    } else {
        RATING_CR_PER_DAY
    };
    minor_of(cr, credit_scale(w)).map(Amount::from_minor)
}

/// A rank's word.
fn rank(officer: bool) -> &'static str {
    if officer { "officer" } else { "rating" }
}

/// One duty's word, as `+sortie/crew` and `+sortie/available` print and read it.
#[must_use]
pub fn duty_word(duty: CrewDuty) -> &'static str {
    match duty {
        CrewDuty::Mining => "mining",
        CrewDuty::Combat => "combat",
        CrewDuty::Recovery => "recovery",
        CrewDuty::Technician => "technician",
    }
}

/// The duty a word names.
fn parse_duty(word: &str) -> Option<CrewDuty> {
    [
        CrewDuty::Mining,
        CrewDuty::Combat,
        CrewDuty::Recovery,
        CrewDuty::Technician,
    ]
    .into_iter()
    .find(|d| duty_word(*d) == word)
}

fn duties_words(duties: &BTreeSet<CrewDuty>) -> String {
    duties
        .iter()
        .map(|d| duty_word(*d))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A price in the credit's render.
fn money(w: &World, amount: Amount) -> String {
    crate::port::board::money(w, amount)
}

/// The hiring port's authority for `terms`: its organization and its credit account.
fn payee(w: &World, terms: &HireTerms) -> Option<(OrgId, AccountId)> {
    let venue = mush_market::MarketId::new(terms.port.as_str())?;
    let (def, map) = crate::port::market::port_of_venue(w, &venue)?;
    let row = map?.orgs.get(def.authority.as_ref()?)?;
    Some((OrgId(row.org), AccountId(row.account?)))
}

/// **The offer line's wage clause** for `terms`.
#[must_use]
pub fn offer_clause(w: &World, terms: &HireTerms) -> String {
    daily_wage(w, terms)
        .map(|wage| OFFER_WAGE.replace("{wage}", &money(w, wage)))
        .unwrap_or_default()
}

/// **The offers' footer**, the two scale wages filled.
#[must_use]
pub fn offers_footer(w: &World) -> String {
    let at = |cr| {
        minor_of(cr, credit_scale(w))
            .map(|m| money(w, Amount::from_minor(m)))
            .unwrap_or_default()
    };
    OFFERS_WAGES
        .replace("{rating}", &at(RATING_CR_PER_DAY))
        .replace("{officer}", &at(OFFICER_CR_PER_DAY))
}

/// **A contract's wage line**: what it owes an IC day, to whom, and what stands in arrears.
#[must_use]
pub fn contract_line(w: &World, contract: &CrewContract) -> String {
    let ns = wage_namespace(contract.npc);
    let debtor = PrincipalId::Object(contract.employer);
    let open = mush_economy::player_obligations_of(w, &debtor, &ns)
        .into_iter()
        .find(|o| o.schedule.cadence.is_some() && live(o));
    let Some(open) = open else {
        return WAGE_NONE.to_string();
    };
    let payee = payee(w, &contract.terms).map_or_else(
        || "the hiring port".to_string(),
        |(org, _)| {
            crate::surface::retrieval::principal_name(w, &PrincipalId::Organization(org))
                .to_string()
        },
    );
    let arrears: i64 = mush_economy::debts_of(w, &debtor, &[ns.as_str()])
        .iter()
        .map(|d| d.amount.minor())
        .sum();
    WAGE_LINE
        .replace("{wage}", &money(w, open.amount))
        .replace("{payee}", &payee)
        .replace("{arrears}", &money(w, Amount::from_minor(arrears)))
}

fn live(o: &Obligation) -> bool {
    !matches!(
        o.status,
        ObligationStatus::Cancelled | ObligationStatus::Satisfied
    )
}

/// One wage the contracts want: who pays whom how much.
struct Want {
    debtor: PrincipalId,
    debit: AccountId,
    credit: AccountId,
    amount: Amount,
}

/// **The wages the contracts want charged**: each standing contract, owed by its employer (always
/// a player: only a player in a port room can accept an offer) to its hiring port's authority.
fn wanted(ctx: &Ctx<'_>) -> BTreeMap<String, Want> {
    let mut out = BTreeMap::new();
    let Some(handles) = ctx.w.module_state::<CrewHandles>().cloned() else {
        return out;
    };
    let mut contracts = Vec::new();
    crate::port::census::scan(ctx, &handles.contracts, |c: &CrewContract| {
        contracts.push(c.clone());
    });
    for contract in contracts {
        if ctx
            .w
            .get(contract.employer)
            .is_none_or(|o| o.kind != ObjKind::Player)
        {
            continue;
        }
        // Doc 48 cut EX.7b: a hire whose exit is owed (a discharge's, or a capture's) has left;
        // its wage is not opened again while the roster's half lands.
        if crate::field::npc_exit_of(ctx.w, contract.npc).is_some() {
            continue;
        }
        let Some((_, credit)) = payee(ctx.w, &contract.terms) else {
            continue;
        };
        let Some(amount) = daily_wage(ctx.w, &contract.terms) else {
            continue;
        };
        let Some(debit) = crate::port::credit::own_account(ctx.w, contract.employer).map(|a| a.id)
        else {
            continue;
        };
        if debit == credit {
            continue;
        }
        out.insert(
            wage_namespace(contract.npc),
            Want {
                debtor: PrincipalId::Object(contract.employer),
                debit,
                credit,
                amount,
            },
        );
    }
    out
}

/// Every live recurring wage obligation, by its namespace.
fn live_wages(w: &World) -> BTreeMap<String, Obligation> {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return BTreeMap::new();
    };
    store
        .snapshot()
        .obligations()
        .filter(|o| o.schedule.cadence.is_some() && live(o))
        .filter_map(|o| {
            let ns = o.arrears.as_ref()?.namespace.to_string();
            ns.starts_with(WAGE_NS_PREFIX).then(|| (ns, o.clone()))
        })
        .collect()
}

/// **Keep every wage in step with the contracts** (doc 48 cut EX.7): open a wage for each
/// contract that has none, and end a wage whose contract is gone or changed hands. Never panics; a
/// refusal is logged and retried next fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let wanted = wanted(ctx);
    let live = live_wages(ctx.w);
    for (ns, o) in &live {
        match wanted.get(ns) {
            Some(want)
                if want.debtor == o.debtor
                    && want.debit == o.debit_account
                    && want.credit == o.credit_account =>
            {
                if want.amount != o.amount
                    && let Err(error) =
                        mush_economy::reprice_player_obligation(ctx, o.id, ns, want.amount)
                {
                    tracing::warn!(obligation = o.id.0, %error, "tacsim: a wage was not repriced");
                }
            }
            _ => {
                if let Err(error) = mush_economy::cancel_obligation(ctx, o.id, ns) {
                    tracing::warn!(obligation = o.id.0, %error, "tacsim: a wage did not end");
                }
            }
        }
    }
    let now_ic = ctx.w.world_ic_now().at;
    for (ns, want) in wanted {
        if live.get(&ns).is_some_and(|o| {
            o.debtor == want.debtor
                && o.debit_account == want.debit
                && o.credit_account == want.credit
        }) {
            continue;
        }
        let spec = mush_economy::ObligationSpec {
            debit_account: want.debit,
            credit_account: want.credit,
            amount: want.amount,
            schedule: mush_economy::EconomicSchedule::new(
                mush_economy::EconomicClock::WorldIc,
                now_ic.saturating_add(IC_DAY),
                Some(IC_DAY),
                mush_economy::MAX_CATCH_UP_CAP,
            ),
            grace: mush_economy::GracePolicy::new(mush_economy::PLAYER_OWED_MIN_GRACE),
            notify_on_success: false,
            cause: mush_economy::ObligationCause::Ruleset {
                namespace: SmolStr::new(&ns),
            },
        };
        if let Err(error) = mush_economy::create_player_obligation(ctx, spec, &ns) {
            tracing::warn!(%ns, %error, "tacsim: a wage was not opened");
        }
    }
    collect_ended(ctx, &live);
}

/// The most ended-wage debts one port fire collects.
const MAX_COLLECTED_PER_FIRE: usize = 16;

/// Whether `account`'s available funds cover `minor`.
fn covers(w: &World, account: AccountId, minor: i64) -> bool {
    w.module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().account(account)?.available())
        .is_some_and(|available| available.minor() >= minor)
}

/// **An ended wage's debt is collected once the employer can pay it** (doc 48 cut EX.7b): each
/// one-term debt under a wage's name that no live wage still keeps as its arrears (the economy
/// pays those itself, after a period settles) is paid whole, from the employer's account to the
/// hiring port, at the first port fire that finds the account covering it. So what a discharge or
/// a capture left standing is paid when the employer is funded, never forgiven.
fn collect_ended(ctx: &mut Ctx<'_>, live: &BTreeMap<String, Obligation>) {
    let kept: BTreeSet<mush_economy::ObligationId> = live
        .values()
        .filter_map(|o| o.arrears.as_ref()?.debt)
        .collect();
    let owed: Vec<(mush_economy::ObligationId, String)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        store
            .snapshot()
            .obligations()
            .filter(|o| o.schedule.cadence.is_none() && live_debt(o) && !kept.contains(&o.id))
            .filter_map(|o| match &o.cause {
                mush_economy::ObligationCause::Ruleset { namespace }
                    if namespace.starts_with(WAGE_NS_PREFIX)
                        && covers(ctx.w, o.debit_account, o.amount.minor()) =>
                {
                    Some((o.id, namespace.to_string()))
                }
                _ => None,
            })
            .take(MAX_COLLECTED_PER_FIRE)
            .collect()
    };
    for (id, ns) in owed {
        if let Err(error) = mush_economy::pay_debt(ctx, id, &ns) {
            tracing::warn!(debt = id.0, %error, "tacsim: an ended wage's debt was not collected");
        }
    }
}

/// Whether a one-term debt is still owed.
fn live_debt(o: &Obligation) -> bool {
    matches!(
        o.status,
        ObligationStatus::Active | ObligationStatus::Grace { .. } | ObligationStatus::Delinquent
    )
}

/// **What a discharge's final wage came to** (doc 48 cut EX.7b).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FinalWage {
    /// What moved at the discharge, minor units.
    pub(super) paid: i64,
    /// What stands as the employer's debt, minor units.
    pub(super) owing: i64,
    /// Who it is paid to.
    pub(super) payee: String,
}

/// Pay the final IC day directly when the employer's account already covers it, avoiding
/// minting a transient debt obligation in `EconomyStore` on funded discharges (HIGH-69).
fn pay_final_day(
    ctx: &mut Ctx<'_>,
    contract: &CrewContract,
    debit: AccountId,
    credit: AccountId,
    amount: Amount,
    ns: &str,
) -> Result<mush_economy::PostOutcome, mush_economy::EconomyError> {
    let minor = amount.minor();
    if minor <= 0 {
        return Err(mush_economy::EconomyError::ZeroLine(0));
    }
    let rev = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().account(debit).map(|a| a.revision))
        .unwrap_or(0);
    let key = mush_economy::IdempotencyKey::new(
        "tacsim.crew.final_wage",
        format!(
            "{}/{}/{}/{}/{}",
            contract.employer.0,
            contract.npc.0,
            contract.hired_at.at,
            ctx.w.world_ic_now().at,
            rev
        ),
    );
    let mut plan = mush_economy::EconomyPlan::new(
        key,
        PrincipalId::Object(ctx.actor),
        mush_economy::EconomicCause::Internal {
            kind: SmolStr::new(ns),
            id: u64::from(contract.npc.0),
        },
        vec![
            mush_economy::RequestedLine {
                account: debit,
                delta: Amount::from_minor(minor.saturating_neg()),
                memo: "final crew wage".into(),
            },
            mush_economy::RequestedLine {
                account: credit,
                delta: Amount::from_minor(minor),
                memo: "final crew wage".into(),
            },
        ],
    );
    plan.description = format!("final crew wage for #{}", contract.npc.0);
    mush_economy::post(ctx, &plan)
}

/// Coalesce any remaining multiple live unpaid debts under `(debtor, ns)` sharing the same
/// `(debit_account, credit_account)` pair into a single debt obligation (HIGH-69).
fn coalesce_remaining_debts(
    ctx: &mut Ctx<'_>,
    debtor: &PrincipalId,
    ns: &str,
) -> Result<(), mush_economy::EconomyError> {
    let debts = mush_economy::debts_of(ctx.w, debtor, &[ns]);
    let mut grouped: BTreeMap<(AccountId, AccountId), Vec<Obligation>> = BTreeMap::new();
    for debt in debts {
        grouped
            .entry((debt.debit_account, debt.credit_account))
            .or_default()
            .push(debt);
    }
    for ((debit, credit), group) in grouped {
        if group.len() <= 1 {
            continue;
        }
        let combined = group
            .iter()
            .try_fold(0_i64, |acc, d| acc.checked_add(d.amount.minor()))
            .ok_or(mush_economy::EconomyError::Overflow)?;
        let due = group
            .iter()
            .map(|d| d.schedule.next_due)
            .min()
            .unwrap_or_else(|| ctx.w.world_ic_now().at);
        let _merged = mush_economy::create_debt_due(
            ctx,
            (debit, credit),
            Amount::from_minor(combined),
            ns,
            due,
        )?;
        for old in group {
            if let Err(error) = mush_economy::cancel_obligation(ctx, old.id, ns) {
                tracing::warn!(
                    debt = old.id.0,
                    %error,
                    "tacsim: a coalesced crew wage debt did not cancel"
                );
            }
        }
    }
    Ok(())
}

/// **Settle a hire's final wage at its discharge** (doc 48 cut EX.7b): the IC day in progress,
/// charged whole, beside whatever stands in arrears, to the wage's own payee.
///
/// - Covered arrears and a covered day in progress are paid directly without creating extra
///   debt obligations, and the recurring wage obligation is ended so **no further period
///   accrues** whatever fires afterwards.
/// - Any unpaid day in progress is coalesced with remaining unpaid arrears under the same
///   `(employer, namespace)` into a single live debt obligation (HIGH-69), first swept
///   [`mush_economy::ARREARS_DUE_IC_SECS`] on (or the earliest existing due instant) and
///   collected at a port fire once the account covers it (`collect_ended`). **A discharge is
///   never refused for want of money.**
///
/// `None` when the hire owed no wage and no debt (a port with no authority, or an employer with
/// no account).
///
/// # Errors
///
/// The economy refused to record the day or to end the wage (it is frozen); nothing is changed.
pub(super) fn settle_final(
    ctx: &mut Ctx<'_>,
    contract: &CrewContract,
) -> Result<Option<FinalWage>, mush_economy::EconomyError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(EconomyStore::is_frozen)
    {
        return Err(mush_economy::EconomyError::EconomyFrozen);
    }
    let ns = wage_namespace(contract.npc);
    let debtor = PrincipalId::Object(contract.employer);
    let open = mush_economy::player_obligations_of(ctx.w, &debtor, &ns)
        .into_iter()
        .find(|o| o.schedule.cadence.is_some() && live(o));
    // Who pays whom the day in progress: the open wage's own accounts, else what a wage opened
    // now would name (a hire discharged before its first port fire).
    let day = match &open {
        Some(o) => Some((o.debit_account, o.credit_account, o.amount)),
        None => payee(ctx.w, &contract.terms).and_then(|(_, credit)| {
            let debit = crate::port::credit::own_account(ctx.w, contract.employer)?.id;
            let amount = daily_wage(ctx.w, &contract.terms)?;
            (debit != credit).then_some((debit, credit, amount))
        }),
    };
    let mut paid = 0_i64;
    let existing = mush_economy::debts_of(ctx.w, &debtor, &[ns.as_str()]);
    for debt in &existing {
        if !covers(ctx.w, debt.debit_account, debt.amount.minor()) {
            continue;
        }
        match mush_economy::pay_debt(ctx, debt.id, &ns) {
            Ok(
                mush_economy::PostOutcome::Posted(_) | mush_economy::PostOutcome::AlreadyPosted(_),
            ) => paid = paid.saturating_add(debt.amount.minor()),
            Ok(mush_economy::PostOutcome::NeedsApproval(_)) => {}
            Err(mush_economy::EconomyError::EconomyFrozen) => {
                return Err(mush_economy::EconomyError::EconomyFrozen);
            }
            Err(error) => {
                tracing::warn!(debt = debt.id.0, %error, "tacsim: a final wage was not paid; it stands");
            }
        }
    }
    if let Some((debit, credit, amount)) = day {
        let mut day_paid = false;
        if covers(ctx.w, debit, amount.minor()) {
            match pay_final_day(ctx, contract, debit, credit, amount, &ns) {
                Ok(
                    mush_economy::PostOutcome::Posted(_)
                    | mush_economy::PostOutcome::AlreadyPosted(_),
                ) => {
                    paid = paid.saturating_add(amount.minor());
                    day_paid = true;
                    if let Some(o) = &open
                        && let Err(error) = mush_economy::cancel_obligation(ctx, o.id, &ns)
                    {
                        tracing::error!(
                            obligation = o.id.0,
                            %error,
                            "tacsim: a paid discharge's recurring wage did not cancel"
                        );
                    }
                }
                Ok(mush_economy::PostOutcome::NeedsApproval(_)) => {}
                Err(mush_economy::EconomyError::EconomyFrozen) => {
                    return Err(mush_economy::EconomyError::EconomyFrozen);
                }
                Err(error) => {
                    tracing::warn!(
                        npc = contract.npc.0,
                        %error,
                        "tacsim: direct final wage payment failed; falling back to debt"
                    );
                }
            }
        }
        if !day_paid {
            let remaining = mush_economy::debts_of(ctx.w, &debtor, &[ns.as_str()]);
            let same_pair: Vec<Obligation> = remaining
                .into_iter()
                .filter(|d| d.debit_account == debit && d.credit_account == credit)
                .collect();
            let combined = same_pair
                .iter()
                .try_fold(amount.minor(), |acc, d| acc.checked_add(d.amount.minor()))
                .ok_or(mush_economy::EconomyError::Overflow)?;
            let default_due = ctx
                .w
                .world_ic_now()
                .at
                .saturating_add(mush_economy::ARREARS_DUE_IC_SECS);
            let due = same_pair
                .iter()
                .map(|d| d.schedule.next_due)
                .fold(default_due, i64::min);
            let last = mush_economy::create_debt_due(
                ctx,
                (debit, credit),
                Amount::from_minor(combined),
                &ns,
                due,
            )?;
            if let Some(o) = &open
                && let Err(error) = mush_economy::cancel_obligation(ctx, o.id, &ns)
            {
                // The wage still runs, so the day is not owed twice: take the debt back.
                if let Err(undo) = mush_economy::cancel_obligation(ctx, last, &ns) {
                    tracing::error!(debt = last.0, %undo, "tacsim: a refused discharge's last day stands");
                }
                return Err(error);
            }
            for old in same_pair {
                if let Err(error) = mush_economy::cancel_obligation(ctx, old.id, &ns) {
                    tracing::warn!(
                        debt = old.id.0,
                        %error,
                        "tacsim: a coalesced crew wage debt did not cancel"
                    );
                }
            }
        }
    }
    coalesce_remaining_debts(ctx, &debtor, &ns)?;
    let owing: i64 = mush_economy::debts_of(ctx.w, &debtor, &[ns.as_str()])
        .iter()
        .map(|d| d.amount.minor())
        .sum();
    if paid == 0 && owing == 0 {
        return Ok(None);
    }
    let payee = payee(ctx.w, &contract.terms).map_or_else(
        || "the hiring port".to_string(),
        |(org, _)| {
            crate::surface::retrieval::principal_name(ctx.w, &PrincipalId::Organization(org))
                .to_string()
        },
    );
    Ok(Some(FinalWage { paid, owing, payee }))
}

/// A sum of minor units in the credit's render.
pub(super) fn money_minor(w: &World, minor: i64) -> String {
    money(w, Amount::from_minor(minor))
}

/// **What `employer` owes `npc`'s wage in arrears**, minor units.
#[must_use]
pub fn arrears_of(w: &World, employer: ObjId, npc: ObjId) -> i64 {
    let ns = wage_namespace(npc);
    mush_economy::debts_of(w, &PrincipalId::Object(employer), &[ns.as_str()])
        .iter()
        .map(|d| d.amount.minor())
        .sum()
}

// ---- Crew for hire: a player's own listing (Q-48-14) ----------------------------------------

/// **A player's listing of themselves for crew hire**: where, as what, for which duties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewListing {
    /// The port's venue whose board shows it.
    pub port: SmolStr,
    /// Whether they offer to command a detachment (an officer) or to operate a unit (a rating).
    pub officer: bool,
    /// The duties they offer.
    pub duties: BTreeSet<CrewDuty>,
}

/// `who`'s listing, when it is their own (written by `+sortie/available`, never by hand).
#[must_use]
pub fn listing_of(w: &World, who: ObjId) -> Option<CrewListing> {
    let attr = w.attr(who, LISTING_ATTR)?;
    if attr.owner != who {
        return None;
    }
    let listing: CrewListing = serde_json::from_value(attr.as_json()?.clone()).ok()?;
    (!listing.duties.is_empty() && listing.duties.len() <= 4).then_some(listing)
}

/// The port whose station `who` stands in, by venue.
pub(super) fn port_here(w: &World, who: ObjId) -> Option<SmolStr> {
    crate::port::market::ports(w)
        .into_iter()
        .map(|(_, def, _)| def.venue.as_str().to_owned())
        .find(|venue| crate::port::passenger::at_station(w, who, venue))
        .map(SmolStr::from)
}

fn port_title(w: &World, venue: &str) -> String {
    crate::port::passenger::port_title(w, venue)
}

fn wage_for(w: &World, officer: bool) -> String {
    let cr = if officer {
        OFFICER_CR_PER_DAY
    } else {
        RATING_CR_PER_DAY
    };
    minor_of(cr, credit_scale(w))
        .map(|m| money(w, Amount::from_minor(m)))
        .unwrap_or_default()
}

/// **`+sortie/available [<duty>[,<duty>] [officer]]`**: list yourself at the port you stand in,
/// or show your listing. **`+sortie/unavailable`**: withdraw it.
pub(crate) fn available(ctx: &mut Ctx<'_>, raw: &str, withdraw: bool) -> Result<String, String> {
    let actor = ctx.actor;
    if ctx.w.get(actor).is_none_or(|o| o.kind != ObjKind::Player) {
        return Err(LIST_USAGE.into());
    }
    if withdraw {
        if !raw.is_empty() {
            return Err(LIST_USAGE.into());
        }
        if listing_of(ctx.w, actor).is_none() {
            return Ok(NOT_LISTED.into());
        }
        ctx.w
            .remove_attr(actor, LISTING_ATTR)
            .map_err(|e| e.to_string())?;
        return Ok(UNLISTED.into());
    }
    if raw.is_empty() {
        return Ok(listing_of(ctx.w, actor).map_or_else(
            || NOT_LISTED.to_string(),
            |l| {
                LISTING_SHOW
                    .replace("{rank}", rank(l.officer))
                    .replace("{duties}", &duties_words(&l.duties))
                    .replace("{port}", &port_title(ctx.w, &l.port))
            },
        ));
    }
    let mut words = raw.split_whitespace();
    let duties: Option<BTreeSet<CrewDuty>> = words
        .next()
        .unwrap_or_default()
        .split(',')
        .map(|w| parse_duty(w.trim()))
        .collect();
    let officer = match words.next() {
        None => false,
        Some("officer") => true,
        Some(_) => return Err(LIST_USAGE.into()),
    };
    let Some(duties) = duties.filter(|d| !d.is_empty()) else {
        return Err(LIST_USAGE.into());
    };
    if words.next().is_some() {
        return Err(LIST_USAGE.into());
    }
    let port = port_here(ctx.w, actor).ok_or(LIST_AT_PORT)?;
    let listing = CrewListing {
        port: port.clone(),
        officer,
        duties,
    };
    let mut attr = Attr::json(
        serde_json::to_value(&listing).map_err(|e| e.to_string())?,
        actor,
    );
    attr.flags = AttrFlags::WIZARD_ONLY | AttrFlags::HIDDEN | AttrFlags::NO_INHERIT;
    ctx.w
        .set_attr(actor, LISTING_ATTR, attr)
        .map_err(|e| e.to_string())?;
    Ok(LISTED
        .replace("{rank}", rank(officer))
        .replace("{duties}", &duties_words(&listing.duties))
        .replace("{port}", &port_title(ctx.w, &port))
        .replace("{wage}", &wage_for(ctx.w, officer)))
}

/// **The players for hire on `venue`'s board**: each player standing in the port's station with
/// a listing for that port, an `Offered` crew service asking the scale wage as an opening figure.
#[must_use]
pub fn services(w: &World, venue: &mush_market::MarketId) -> Vec<mush_market::Service> {
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    let mut out = Vec::new();
    for room in crate::port::passenger::station_rooms(w, venue.as_str()) {
        for who in w.contents_of(room).iter().copied() {
            if !w.get(who).is_some_and(|o| o.kind == ObjKind::Player) {
                continue;
            }
            let Some(listing) = listing_of(w, who).filter(|l| l.port == venue.as_str()) else {
                continue;
            };
            let Some(pay) = minor_of(
                if listing.officer {
                    OFFICER_CR_PER_DAY
                } else {
                    RATING_CR_PER_DAY
                },
                credit_scale(w),
            ) else {
                continue;
            };
            let name = crate::port::coop::name_of(w, who);
            out.push(mush_market::Service {
                kind: mush_market::ServiceKind::Crew,
                side: mush_market::ServiceSide::Offered,
                venue: venue.clone(),
                poster: PrincipalId::Object(who),
                poster_name: name.clone().into(),
                subject: BOARD_SUBJECT
                    .replace("{rank}", rank(listing.officer))
                    .replace("{duties}", &duties_words(&listing.duties))
                    .into(),
                pay: Amount::from_minor(pay),
                instrument: instrument.clone(),
                negotiable: true,
                bond: None,
                term: mush_market::ServiceTerm::Daily,
                contract: None,
                take: BOARD_TAKE.replace("{name}", &name).into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ex7_a_wage_is_named_by_its_hire_and_the_duties_read_back() {
        assert_eq!(wage_namespace(ObjId(12)), "tacsim.crew.wage.12");
        for duty in [
            CrewDuty::Mining,
            CrewDuty::Combat,
            CrewDuty::Recovery,
            CrewDuty::Technician,
        ] {
            assert_eq!(parse_duty(duty_word(duty)), Some(duty));
        }
        assert_eq!(parse_duty("pilot"), None);
    }

    /// Every sentence at its widest fill stays within 120 columns.
    #[test]
    fn ex7_the_sentences_fit_120_columns() {
        let filled = [
            OFFERS_WAGES
                .replace("{rating}", "10.00cr")
                .replace("{officer}", "20.00cr"),
            OFFERS_ABOARD.to_string(),
            WAGE_LINE
                .replace("{wage}", "20.00cr")
                .replace("{payee}", "Freeport Registry of Ceres")
                .replace("{arrears}", "5,600.00cr"),
            WAGE_NONE.to_string(),
            LIST_USAGE.to_string(),
            LISTED
                .replace("{rank}", "officer")
                .replace("{duties}", "mining, combat, recovery, technician")
                .replace("{port}", "Stickney Yards")
                .replace("{wage}", "20.00cr"),
            LISTING_SHOW
                .replace("{rank}", "officer")
                .replace("{duties}", "mining, combat, recovery, technician")
                .replace("{port}", "Stickney Yards"),
            NOT_LISTED.to_string(),
            UNLISTED.to_string(),
            LIST_AT_PORT.to_string(),
            HIRE_NO_ACCOUNT.to_string(),
            HIRE_DELINQUENT.to_string(),
            HIRE_UNPAID_WAGES.to_string(),
        ];
        for line in filled {
            assert!(line.chars().count() <= 120, "{} wide: {line}", line.len());
        }
    }

    #[test]
    fn high69_hiring_standing_and_debt_coalescing() {
        let (base, reg) = mush_testkit::boot_world();
        let mut engine = mush_core::Engine::with_modules(
            base,
            reg,
            vec![Box::new(mush_economy::EconomyModule::new())],
        )
        .expect("engine");
        let employer = engine
            .world
            .create(ObjKind::Player, "Employer", mush_core::GOD, None)
            .expect("employer");
        let npc = engine
            .world
            .create(ObjKind::Npc, "Mendes", mush_core::GOD, None)
            .expect("npc");
        // Without an active credit account, hiring at a waged port is refused.
        assert_eq!(
            hiring_standing(&engine.world, employer),
            Err(HIRE_NO_ACCOUNT)
        );

        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: employer,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        assert!(crate::economy::ensure_instrument(&mut ctx, &Default::default()).is_some());
        let instrument =
            mush_economy::InstrumentId::new(crate::economy::CREDIT).expect("credit instrument");
        let emp_acc = AccountId(101);
        mush_economy::open_account(
            &mut ctx,
            mush_economy::Account {
                id: emp_acc,
                owner: PrincipalId::Object(employer),
                instrument: instrument.clone(),
                scope: None,
                name: SmolStr::new("employer"),
                status: mush_economy::AccountStatus::Active,
                posted_balance: Amount::ZERO,
                held_amount: Amount::ZERO,
                last_hold_seq: 0,
                credit: mush_economy::CreditPolicy::forbid(),
                last_journal_seq: 0,
                revision: 1,
                approval: None,
            },
        );
        let hall_acc = AccountId(102);
        mush_economy::open_account(
            &mut ctx,
            mush_economy::Account {
                id: hall_acc,
                owner: PrincipalId::Organization(OrgId(77)),
                instrument,
                scope: None,
                name: SmolStr::new("hall"),
                status: mush_economy::AccountStatus::Active,
                posted_balance: Amount::ZERO,
                held_amount: Amount::ZERO,
                last_hold_seq: 0,
                credit: mush_economy::CreditPolicy::forbid(),
                last_journal_seq: 0,
                revision: 1,
                approval: None,
            },
        );

        // Clean active account (even at zero balance before any default) is allowed to hire.
        assert_eq!(hiring_standing(ctx.w, employer), Ok(()));

        // Create two unpaid crew wage debts under the same (employer, namespace).
        let ns = wage_namespace(npc);
        let due_a = ctx.w.world_ic_now().at + 500;
        let due_b = ctx.w.world_ic_now().at + 1_000;
        let _d1 = mush_economy::create_debt_due(
            &mut ctx,
            (emp_acc, hall_acc),
            Amount::from_minor(1_000),
            &ns,
            due_a,
        )
        .expect("debt 1");
        let _d2 = mush_economy::create_debt_due(
            &mut ctx,
            (emp_acc, hall_acc),
            Amount::from_minor(2_500),
            &ns,
            due_b,
        )
        .expect("debt 2");
        assert_eq!(hiring_standing(ctx.w, employer), Err(HIRE_UNPAID_WAGES));

        // Coalescing merges both live debts into one single live obligation of 3_500 with earliest due.
        let debtor = PrincipalId::Object(employer);
        coalesce_remaining_debts(&mut ctx, &debtor, &ns).expect("coalesce");
        let remaining = mush_economy::debts_of(ctx.w, &debtor, &[ns.as_str()]);
        assert_eq!(remaining.len(), 1);
        let merged = remaining.first().expect("merged debt");
        assert_eq!(merged.amount.minor(), 3_500);
        assert_eq!(merged.schedule.next_due, due_a);

        // Canceling the wage debt restores standing; a non-wage active debt does not block
        // until delinquent.
        mush_economy::cancel_obligation(&mut ctx, merged.id, &ns).expect("cancel");
        assert_eq!(hiring_standing(ctx.w, employer), Ok(()));
        let other = mush_economy::create_debt_due(
            &mut ctx,
            (emp_acc, hall_acc),
            Amount::from_minor(500),
            "tacsim.other.fee",
            due_a,
        )
        .expect("other debt");
        let recorded = ctx
            .w
            .module_state::<EconomyStore>()
            .is_some_and(|store| store.snapshot().obligation(other).is_some());
        assert!(recorded);
    }
}
