//! **Running costs, insolvency and the export faucet** (doc 48 unit EX.9; Q-48-21, Q-48-22,
//! Q-48-36, Q-48-38, Q-48-39, Q-48-40, Q-48-55, Q-48-59, Q-48-60).
//!
//! **The running cost follows the work** (Q-48-38). Every costed facility at a laid-up port (its
//! refinery, works, yard and workshop, whoever owns it) pays its owner's share of its authored
//! daily cost (`mush_industry::FacilityCostDef::daily_cost_minor`, payroll included at 150 cr a
//! head, Q-48-60) once an IC day, from its owner's account to the System account
//! [`UPKEEP_SINK`]: all of it to a sink (Q-48-21's correction), nothing to a payee.
//! - **A worked slot-hour** costs its share of the daily cost (`daily / slots / 24`), which the
//!   fee recovers with its margin. The slots worked are the facility's work orders past their
//!   draft (`mush_industry::IndustryStore::worked_orders_at`: a draft escrows nothing and holds
//!   no slot) and its open bay orders, at most its slots, summed over the IC day on every port
//!   fire ([`Worked`]). The tally is checkpointed into the row each IC hour
//!   ([`TALLY_CHECKPOINT_IC`]), so a restart loses at most an hour of it, and a closed day's
//!   figure is kept until its charge posts.
//! - **An idle slot-hour** costs [`IDLE_PERMILLE`] of it (10%, provisional).
//! - **A mill-run refinery** (Q-48-59: a facility nobody can book) pays one slot-hour of its daily
//!   cost a refine batch its operator milled that day, and no standing share. A sold refinery
//!   (owner-held) is costed as any slot facility: its buyer never pays for the operator's mills.
//! - **A facility whose owner has no account to pay from** accrues each day's charge as a debt,
//!   and mothballs.
//!
//! **A house that cannot pay** (Q-48-38, -39, -59): the day's charge and any debt are paid from
//! what the account holds; what it cannot pay is a **debt** owed to the sink, never minted
//! ([`UpkeepRow::debt_minor`]). A facility in debt is **mothballed**:
//! - it owes no standing share while mothballed;
//! - its house places no order of its own there (`crate::port::works::book`), while a customer's
//!   order, whose escrow covers its worked cost, still books and runs;
//! - its owner is throttled ([`in_arrears`], read by `crate::port::price::throttled`): asks up
//!   10% an IC week and no new bids;
//! - every credit its account takes in goes first to the debt, so each fee's margin repays it;
//! - it reopens once the debt is paid and the account holds [`REOPEN_DAYS`] IC days of its standing
//!   share.
//!
//! **The export faucet** (Q-48-22, Q-48-40, Q-48-55) is the System account [`EXPORT_FAUCET`], the
//! lane's one recurring inflow. It pays:
//! - **the consumers**, for what they used up (`crate::port::consume`): the good's authored price
//!   for each act's units, at most the row's daily share;
//! - **a House stipend**, the launch deployment's `stipend_minor_per_ic_day` for each named House,
//!   once an IC day into its treasury (its root-scope account, which buys its licences and hulls,
//!   else the treasury the deployment names). A day the server never fired is not paid.
//!
//! Every post is keyed (`tacsim.upkeep/{facility}/{day}`, `tacsim.export/{row}/{ordinal}`,
//! `tacsim.stipend/{org}/{day}`), so a restart never charges or pays twice; the charged day and the
//! debt are the port's own record ([`PortUpkeep`], `tacsim:upkeep:{entry}`), written beside the
//! post.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicCause, EconomyPlan,
    EconomyStore, IdempotencyKey, InstrumentId, PostOutcome, RequestedLine,
};
use mush_industry::{FacilityCostDef, IndustryStore};
use mush_org::{OrgId, PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;

use crate::port::def::PortDef;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::port::map::PortMap;
use crate::port::stock::IC_DAY;

/// The System account running costs are paid into: a sink.
pub const UPKEEP_SINK: &str = "tacsim.upkeep";
/// The System account the export faucet pays from: the lane's one recurring inflow (Q-48-22).
pub const EXPORT_FAUCET: &str = "tacsim.export";
/// An idle slot-hour's share of a worked one, per mille (Q-48-38, provisional).
pub const IDLE_PERMILLE: u64 = 100;
/// IC days of standing share a mothballed facility's account must hold to reopen (Q-48-38,
/// provisional).
pub const REOPEN_DAYS: u64 = 7;
/// The most IC days one settlement catches up after a gap; older days are not charged.
pub const MAX_CATCH_UP_DAYS: i64 = 7;
/// A day's charge's idempotency scope.
pub const UPKEEP_KEY: &str = "tacsim.upkeep";
/// A consumer's refund's idempotency scope.
pub const EXPORT_KEY: &str = "tacsim.export";
/// A House stipend's idempotency scope.
pub const STIPEND_KEY: &str = "tacsim.stipend";

/// How often, IC seconds, a facility's day tally is checkpointed into its row (services G10): a
/// restart loses at most this much of a day's work.
pub const TALLY_CHECKPOINT_IC: i64 = 3_600;

/// **One facility's worked slot-seconds this IC day** (RAM, `PortTickState::upkeep_worked`;
/// checkpointed into [`UpkeepRow::tally`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worked {
    /// The IC day being summed.
    pub day: i64,
    /// The IC instant last added to.
    pub at_ic: i64,
    /// Busy slots × IC seconds so far.
    pub slot_secs: u64,
}

impl Worked {
    /// **Add the busy slots since the last fire**, and close the day when it turned: the closed
    /// day and its slot-seconds, if one closed. The span past the old day's end is the new day's.
    #[must_use]
    pub fn add(&mut self, now_ic: i64, busy: u64) -> Option<(i64, u64)> {
        let today = now_ic.div_euclid(IC_DAY);
        let span = |from: i64, to: i64| {
            busy.saturating_mul(u64::try_from(to.saturating_sub(from)).unwrap_or(0))
        };
        if today == self.day {
            self.slot_secs = self
                .slot_secs
                .saturating_add(span(self.at_ic.max(today * IC_DAY), now_ic));
            self.at_ic = now_ic;
            return None;
        }
        let closed = if today > self.day {
            let end = (self.day + 1).saturating_mul(IC_DAY);
            Some((
                self.day,
                self.slot_secs
                    .saturating_add(span(self.at_ic.min(end), end)),
            ))
        } else {
            None
        };
        let start = today.saturating_mul(IC_DAY);
        *self = Worked {
            day: today,
            at_ic: now_ic,
            slot_secs: if closed.is_some() {
                span(start, now_ic)
            } else {
                0
            },
        };
        closed
    }
}

/// **The tally a facility resumes from** after a restart (services G10): its row's checkpoint,
/// when it is of today or earlier (the next [`Worked::add`] closes an earlier day), else a fresh
/// one from `now`. Pure.
#[must_use]
pub fn resume_tally(checkpoint: Option<Worked>, today: i64, now: i64) -> Worked {
    checkpoint
        .filter(|tally| tally.day <= today)
        .unwrap_or(Worked {
            day: today,
            at_ic: now,
            slot_secs: 0,
        })
}

/// **Whether a facility's tally is checkpointed into its row now**: at a day's close, with no
/// checkpoint yet, or [`TALLY_CHECKPOINT_IC`] after the last. Pure.
#[must_use]
pub fn checkpoint_due(checkpoint: Option<Worked>, now: i64, closed: bool) -> bool {
    closed || checkpoint.is_none_or(|tally| now.saturating_sub(tally.at_ic) >= TALLY_CHECKPOINT_IC)
}

// ---- the record ------------------------------------------------------------------------

/// The document prefix of a port's running-cost record: `tacsim:upkeep:{entry}`.
pub const UPKEEP_PREFIX: &str = "tacsim:upkeep:";

/// `tacsim:upkeep:{entry}`.
#[must_use]
pub fn upkeep_key(entry: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{UPKEEP_PREFIX}{entry}"))
}

/// **A port's running-cost record** (doc 48 unit EX.9), one document a port, written whole by
/// the port tick when it changes: the day its houses may start using up goods, and each costed
/// facility's account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortUpkeep {
    /// The port entry.
    pub entry: CanonicalId,
    /// The first IC day its houses use up goods: the day after the tick first saw the port.
    pub consume_from: i64,
    /// Each costed facility's account, by its room's object id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facilities: BTreeMap<u32, UpkeepRow>,
    /// The export faucet's cursor (services G15): each consumption row's last act paid, by the
    /// row's id (`crate::port::consume::ConsumeId::key_text`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refunded: BTreeMap<SmolStr, u64>,
}

/// **One facility's running-cost account** (Q-48-21, -38, -59): the last IC day charged and its
/// charge, the debt owed to the system's sink, whether its owner has mothballed it, and (a
/// mill-run refinery) the batches already charged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpkeepRow {
    /// The last IC day charged.
    pub day: i64,
    /// The owner the last charge was made to, `OrgId.0` for an organisation (the throttle's
    /// reading); absent for a person's facility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<u64>,
    /// Running costs owed and not yet paid, minor units: a debt to the system's sink, never
    /// minted.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub debt_minor: i64,
    /// Mothballed: it owes no standing share and its house places no orders of its own there,
    /// until its debt is paid and it can pay seven IC days ahead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mothballed: bool,
    /// The last IC day's charge, minor units (its worked and standing shares, or its batches).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub charged_minor: i64,
    /// A mill-run refinery's refine batches already charged.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub batches: u64,
    /// Running costs paid to the sink in all, minor units.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paid_minor: i64,
    /// The day's worked tally as last checkpointed (services G10): a restart resumes from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tally: Option<Worked>,
    /// A closed day's worked slot-seconds, kept until that day's charge posts (services G10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<ClosedDay>,
    /// Up to [`MAX_CATCH_UP_DAYS`] closed IC days' worked slot-seconds retained so multi-day
    /// catch-up never drops earlier worked-slot surcharges (`P2-TSW-34`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed_days: Vec<ClosedDay>,
}

impl UpkeepRow {
    /// Record a newly closed day's worked slot-seconds, retaining at most [`MAX_CATCH_UP_DAYS`]
    /// newest closed days and keeping `closed` synchronized for compatibility (`P2-TSW-34`).
    pub(crate) fn record_closed_day(&mut self, day: i64, slot_secs: u64) {
        if let Some(prev) = self.closed
            && !self.closed_days.iter().any(|c| c.day == prev.day)
        {
            self.closed_days.push(prev);
        }
        if let Some(existing) = self.closed_days.iter_mut().find(|c| c.day == day) {
            existing.slot_secs = existing.slot_secs.max(slot_secs);
        } else {
            self.closed_days.push(ClosedDay { day, slot_secs });
        }
        self.closed_days.sort_by_key(|c| c.day);
        let max_keep = usize::try_from(MAX_CATCH_UP_DAYS).unwrap_or(7);
        while self.closed_days.len() > max_keep {
            self.closed_days.remove(0);
        }
        self.closed = self.closed_days.last().copied();
    }

    /// Worked slot-seconds recorded for `day`, reading both `closed_days` and legacy `closed`.
    #[must_use]
    pub(crate) fn closed_slot_secs(&self, day: i64) -> u64 {
        self.closed_days
            .iter()
            .find(|c| c.day == day)
            .or_else(|| self.closed.as_ref().filter(|c| c.day == day))
            .map_or(0, |c| c.slot_secs)
    }

    /// Drop closed-day tallies at or before `day` once charged.
    pub(crate) fn clear_closed_through(&mut self, day: i64) {
        self.closed_days.retain(|c| c.day > day);
        if self.closed.is_some_and(|c| c.day <= day) {
            self.closed = self.closed_days.last().copied();
        }
    }
}

/// **A closed IC day's worked slot-seconds**, not yet charged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedDay {
    /// The IC day.
    pub day: i64,
    /// Busy slots × IC seconds that day.
    pub slot_secs: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` signature
fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` signature
fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

// ---- the arithmetic --------------------------------------------------------------------

/// **One IC day's running cost of a booked facility**, minor units: the worked slot-seconds at
/// the slot-hour, and (unless mothballed) the idle ones at [`IDLE_PERMILLE`] of it. The worked
/// share rounds up, the idle share down. Pure.
#[must_use]
pub fn day_charge(cost: &FacilityCostDef, slots: u32, worked_secs: u64, mothballed: bool) -> i64 {
    let daily = u128::from(cost.daily_cost_minor());
    let capacity = u128::from(slots.max(1)) * u128::from(IC_DAY.unsigned_abs());
    let worked = u128::from(worked_secs).min(capacity);
    let worked_minor = (daily * worked).div_ceil(capacity);
    let idle_minor = if mothballed {
        0
    } else {
        daily * (capacity - worked) * u128::from(IDLE_PERMILLE) / (capacity * 1_000)
    };
    i64::try_from(worked_minor + idle_minor).unwrap_or(i64::MAX)
}

/// **A mill-run refinery's charge for `batches`**: one slot-hour of its daily cost a batch
/// (rounded up). Pure.
#[must_use]
pub fn batch_charge(cost: &FacilityCostDef, batches: u64) -> i64 {
    let daily = u128::from(cost.daily_cost_minor());
    let per = daily.div_ceil(u128::from(mush_industry::cost::WORK_UNITS_PER_DAY));
    i64::try_from(per * u128::from(batches)).unwrap_or(i64::MAX)
}

/// **What the export faucet pays a consumer for one act**, whole credits: the units at the good's
/// authored price, at most the row's daily share. Pure.
#[must_use]
pub fn refund_major(units: u64, price_major: u64, share_major: u64) -> u64 {
    units.saturating_mul(price_major).min(share_major)
}

/// **The standing share of one IC day**, minor units: a whole day idle. Pure.
#[must_use]
pub fn standing_day(cost: &FacilityCostDef, slots: u32) -> i64 {
    day_charge(cost, slots, 0, false)
}

/// **What a settlement leaves**: `(paid, debt, mothballed)` from the day's charge, the debt
/// carried, what the account can pay and the standing share. Pure.
#[must_use]
pub fn settle(
    charge: i64,
    debt: i64,
    available: i64,
    mothballed: bool,
    standing: i64,
) -> (i64, i64, bool) {
    let owed = charge.saturating_add(debt).max(0);
    let paid = owed.min(available.max(0));
    let debt = owed - paid;
    let left = available.max(0) - paid;
    let reopen = i64::try_from(REOPEN_DAYS)
        .unwrap_or(i64::MAX)
        .saturating_mul(standing);
    let mothballed = if debt > 0 {
        true
    } else {
        mothballed && left < reopen
    };
    (paid, debt, mothballed)
}

// ---- the readings ------------------------------------------------------------------------

/// **Whether `org` owes running costs or has a facility mothballed** (Q-48-39): the throttle's
/// second sign, beside a delinquent obligation (`crate::port::price::throttled`).
#[must_use]
pub fn in_arrears(w: &World, org: OrgId) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .is_some_and(|store| {
            store.port_upkeeps().any(|doc| {
                doc.facilities
                    .values()
                    .any(|row| row.org == Some(org.0) && (row.debt_minor > 0 || row.mothballed))
            })
        })
}

/// **What `org` has paid in running costs in all**, minor units, across its facilities.
#[must_use]
pub fn paid_by(w: &World, org: OrgId) -> i64 {
    w.module_state::<crate::store::TacsimStore>()
        .map_or(0, |store| {
            store
                .port_upkeeps()
                .flat_map(|doc| doc.facilities.values())
                .filter(|row| row.org == Some(org.0))
                .map(|row| row.paid_minor)
                .sum()
        })
}

/// **Whether the facility on `room` is mothballed** (Q-48-38).
#[must_use]
pub fn mothballed(w: &World, room: ObjId) -> bool {
    row_of(w, room).is_some_and(|row| row.mothballed)
}

/// **A facility's running-cost row**, when one is kept.
#[must_use]
pub fn row_of(w: &World, room: ObjId) -> Option<UpkeepRow> {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|store| {
            store
                .port_upkeeps()
                .find_map(|doc| doc.facilities.get(&room.0).cloned())
        })
}

/// **Whether the port `entry`'s houses may use up goods on IC day `today`** (doc 48 unit EX.9):
/// from the day after the port tick first kept its running costs, so a port stocked today
/// stands at its authored stock until the day turns.
#[must_use]
pub fn consumes(w: &World, entry: &CanonicalId, today: i64) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.port_upkeep(entry))
        .is_some_and(|doc| today >= doc.consume_from)
}

/// **The costed facilities a port authors**: `(block, room)` for its refinery, works, yard and
/// workshop, each resolved on this world.
fn facilities(w: &World, entry: &CanonicalId, def: &PortDef) -> Vec<(&'static str, ObjId)> {
    let mut out = Vec::new();
    for block in ["refinery", "works", "yard"] {
        if let Ok(Some(room)) = crate::port::plant::block_room(w, entry, def, block) {
            out.push((block, room));
        }
    }
    if let Ok(Some(room)) = crate::port::workshop::room(w, entry, def) {
        out.push(("workshop", room));
    }
    out
}

/// The lowest Active credit account `owner` holds with `scope`.
fn account_of(w: &World, owner: &PrincipalId, root_scope: bool) -> Option<AccountId> {
    let instrument = InstrumentId::new(crate::economy::CREDIT)?;
    let root = match owner {
        PrincipalId::Organization(org) if root_scope => Some(
            w.module_state::<mush_org::OrgStore>()?
                .state()
                .org(*org)?
                .root_scope,
        ),
        _ => None,
    };
    w.module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(owner)
        .filter(|a| {
            a.status == AccountStatus::Active
                && a.instrument == instrument
                && (!root_scope || a.scope == root)
        })
        .map(|a| a.id)
        .min()
}

/// **The account a facility's owner pays from**: a lane institution's own account (its port map
/// row's), else an organisation's root-scope treasury, else a person's lowest credit account.
fn payer_of(w: &World, owner: &PrincipalId) -> Option<AccountId> {
    match owner {
        PrincipalId::Organization(org) => w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| {
                store.port_maps().find_map(|map| {
                    map.orgs
                        .values()
                        .find(|row| row.org == org.0)
                        .and_then(|row| row.account)
                })
            })
            .map(AccountId)
            .or_else(|| crate::port::pools::treasury_of(w, *org)),
        other => account_of(w, other, false),
    }
}

/// Raw available balance of `account`, floored at 0 (used by authored mill-run refineries so
/// G11 holds batches on a refused house transfer until the rule clears).
fn raw_available(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().account(account))
        .and_then(mush_economy::Account::available)
        .map_or(0, Amount::minor)
        .max(0)
}

/// Pure cap applying an optional account `ApprovalRule` threshold and rolling-window headroom
/// to an account's raw available balance (`P2-TSW-34`).
fn payable_cap(
    avail: i64,
    approval: Option<&mush_economy::ApprovalRule>,
    spent_in_window: i64,
) -> i64 {
    let mut cap = avail.max(0);
    if let Some(rule) = approval {
        cap = cap.min(rule.threshold.minor().max(0));
        if let Some(window) = rule.window.as_ref() {
            let left = window
                .total
                .minor()
                .saturating_sub(spent_in_window.max(0))
                .max(0);
            cap = cap.min(left);
        }
    }
    cap
}

/// What an account can pay unconditionally now without triggering an account `ApprovalRule`
/// or exceeding its rolling spending window (`P2-TSW-34`).
fn payable_available(w: &World, account: AccountId) -> i64 {
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return 0;
    };
    let Some(acc) = economy.snapshot().account(account) else {
        return 0;
    };
    let avail = acc.available().map_or(0, Amount::minor);
    let spent = acc
        .approval
        .as_ref()
        .and_then(|rule| rule.window.as_ref())
        .map_or(0, |window| {
            economy
                .spent_in_window(account, w.now(), window.period_secs)
                .minor()
        });
    payable_cap(avail, acc.approval.as_ref(), spent)
}

/// Outcome of one upkeep/faucet transfer attempt (`P2-TSW-34`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferOutcome {
    /// Posted (or already posted) or zero amount.
    Posted,
    /// Blocked by an account approval rule / rolling window.
    NeedsApproval,
    /// Structural or transient posting failure.
    Failed,
}

/// **The System account `name`**, opened create-only the first time (the port treasury's rule,
/// `crate::port::apply`): the lowest Active credit account the System principal holds.
pub(crate) fn system_account(ctx: &mut Ctx<'_>, name: &str) -> Option<AccountId> {
    let principal = SystemPrincipalId::new(name)?;
    let owner = PrincipalId::System(principal);
    if let Some(found) = account_of(ctx.w, &owner, false) {
        return Some(found);
    }
    let instrument = InstrumentId::new(crate::economy::CREDIT)?;
    let id = ctx.w.module_state_mut::<EconomyStore>()?.alloc_account()?;
    mush_economy::open_account(
        ctx,
        Account {
            id,
            owner,
            instrument,
            scope: None,
            name: SmolStr::new(name),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            last_hold_seq: 0,
            credit: CreditPolicy::unbounded(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    Some(id)
}

/// **One keyed transfer** of `minor` from `from` to `to`, as the tick.
fn transfer(
    ctx: &mut Ctx<'_>,
    key: IdempotencyKey,
    (from, to): (AccountId, AccountId),
    minor: i64,
    (scope, id, description): (&'static str, u64, String),
) -> TransferOutcome {
    if minor <= 0 {
        return TransferOutcome::Posted;
    }
    let (Some(debit), credit) = (
        Amount::from_minor(minor).checked_neg(),
        Amount::from_minor(minor),
    ) else {
        return TransferOutcome::Failed;
    };
    let plan = EconomyPlan {
        description,
        visibility: mush_economy::EconomicVisibility::Public,
        ..EconomyPlan::new(
            key,
            PrincipalId::Object(ctx.actor),
            EconomicCause::Internal {
                kind: SmolStr::new_static(scope),
                id,
            },
            vec![
                RequestedLine {
                    account: from,
                    delta: debit,
                    memo: SmolStr::new_static(scope),
                },
                RequestedLine {
                    account: to,
                    delta: credit,
                    memo: SmolStr::new_static(scope),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => TransferOutcome::Posted,
        Ok(PostOutcome::NeedsApproval(_)) | Err(mush_economy::EconomyError::ApprovalRequired) => {
            TransferOutcome::NeedsApproval
        }
        Err(_) => TransferOutcome::Failed,
    }
}

// ---- the tick --------------------------------------------------------------------------

/// **The running costs, the faucet's refunds and the House stipends**, on every port fire
/// (`crate::port_tick`, after the berths and the wages).
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    if ctx.w.module_state::<EconomyStore>().is_none() {
        return;
    }
    running_costs(ctx);
    refunds(ctx);
    stipends(ctx);
}

/// The refine batches `operator`'s refine mills at the port have milled in all.
fn refine_batches(ctx: &Ctx<'_>, map: &PortMap, def: &PortDef, operator: &CanonicalId) -> u64 {
    let Some(mills) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .map(|handles| handles.families.mills.clone())
    else {
        return 0;
    };
    def.vendors
        .iter()
        .filter(|vendor| &vendor.org == operator)
        .flat_map(|vendor| vendor.mill.iter().map(move |mill| (vendor, mill)))
        .filter(|(_, mill)| mill.kind.is_refine())
        .filter_map(|(vendor, mill)| {
            let id = crate::port::mill::MillId::derive(
                &map.pack,
                &map.entry,
                &vendor.org,
                mill.key_good(),
            )
            .ok()?;
            crate::port::mill::ledger_at(ctx, &mills, &id)
                .ok()
                .flatten()
        })
        .map(|ledger| ledger.batches)
        .fold(0, u64::saturating_add)
}

/// **Every port's facilities, charged for each IC day closed** since their last charge.
#[allow(clippy::too_many_lines)] // one facility's day, start to end
fn running_costs(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now().at;
    let today = now.div_euclid(IC_DAY);
    let ports: Vec<(CanonicalId, PortDef, PortMap)> = crate::port::market::ports(ctx.w)
        .into_iter()
        .filter(|(entry, _, _)| crate::port::stock::entry_latched(ctx.w, entry))
        .filter_map(|(entry, def, map)| Some((entry, def.clone(), map?.clone())))
        .collect();
    if ports.is_empty() {
        return;
    }
    let Some(sink) = system_account(ctx, UPKEEP_SINK) else {
        return;
    };
    for (entry, def, map) in ports {
        let kept = ctx
            .w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| store.port_upkeep(&entry).cloned());
        // First sight of the port: its consumption starts tomorrow.
        let mut next = kept.clone().unwrap_or_else(|| PortUpkeep {
            entry: entry.clone(),
            consume_from: today + 1,
            facilities: BTreeMap::new(),
            refunded: BTreeMap::new(),
        });
        for (block, room) in facilities(ctx.w, &entry, &def) {
            let Some(record) = ctx
                .w
                .module_state::<IndustryStore>()
                .and_then(|store| store.facility(room))
                .cloned()
            else {
                continue;
            };
            let Some(cost) = record.cost().copied() else {
                continue;
            };
            let slots = record.maximum_slots();
            // The busy slots now, summed into the day: the orders past their draft (a draft
            // holds no slot, services G17) and the open bay orders.
            let busy = {
                let orders = ctx
                    .w
                    .module_state::<IndustryStore>()
                    .map_or(0, |store| store.worked_orders_at(room));
                let bays = ctx
                    .w
                    .module_state::<crate::store::TacsimStore>()
                    .map_or(0, |store| store.bay_open_at(room));
                u64::try_from(orders.saturating_add(bays))
                    .unwrap_or(u64::MAX)
                    .min(u64::from(slots))
            };
            let owner = record.owner().clone();
            let org = match &owner {
                PrincipalId::Organization(org) => Some(org.0),
                _ => None,
            };
            // An authored refinery is mill-run and pays by its operator's batches; a sold one
            // (owner-held) is costed as any slot facility, its buyer never paying for the
            // operator's mills (production G5).
            let by_batch = block == "refinery" && record.holding().is_authored();
            let operator = def.refinery.as_ref().map(|r| r.operator.clone());
            let first = !next.facilities.contains_key(&room.0);
            let mut row = next.facilities.get(&room.0).cloned().unwrap_or_else(|| {
                // First sight: charging starts with today.
                let batches = match (&operator, by_batch) {
                    (Some(operator), true) => refine_batches(ctx, &map, &def, operator),
                    _ => 0,
                };
                UpkeepRow {
                    day: today - 1,
                    org,
                    batches,
                    ..UpkeepRow::default()
                }
            });
            // The day's tally: RAM, seeded from the row's checkpoint after a restart, and
            // checkpointed into the row each IC hour and at each day's close (services G10).
            let seed = resume_tally(row.tally, today, now);
            let tallied = ctx
                .w
                .module_state_mut::<crate::port_tick::PortTickState>()
                .map(|state| {
                    let worked = state.upkeep_worked.entry(room.0).or_insert(seed);
                    let closed = worked.add(now, busy);
                    (closed, *worked)
                });
            if let Some((closed, worked)) = tallied {
                if let Some((day, slot_secs)) = closed {
                    // Kept up to MAX_CATCH_UP_DAYS until each day's charge posts (P2-TSW-34).
                    row.record_closed_day(day, slot_secs);
                }
                if checkpoint_due(row.tally, now, closed.is_some()) {
                    row.tally = Some(worked);
                }
            }
            if first || row.day >= today - 1 {
                next.facilities.insert(room.0, row);
                continue;
            }
            // No payer: the days still accrue as a debt, and it mothballs (services G14).
            let payer = payer_of(ctx.w, &owner);
            // Each closed day since the last charge, the oldest skipped past the catch-up bound.
            let from = (row.day + 1).max(today - MAX_CATCH_UP_DAYS);
            for day in from..today {
                // A refinery's batches: the total read now, the cursor moved only once the
                // charge posts (services G11).
                let mut batches = None;
                let charge = if by_batch {
                    if day == today - 1 {
                        let total = operator
                            .as_ref()
                            .map_or(row.batches, |op| refine_batches(ctx, &map, &def, op));
                        batches = Some(total.max(row.batches));
                        batch_charge(&cost, total.saturating_sub(row.batches))
                    } else {
                        0
                    }
                } else {
                    let worked = row.closed_slot_secs(day);
                    day_charge(&cost, slots, worked, row.mothballed)
                };
                let standing = if by_batch {
                    0
                } else {
                    standing_day(&cost, slots)
                };
                let avail = payer.map_or(0, |payer| {
                    if by_batch {
                        raw_available(ctx.w, payer)
                    } else {
                        payable_available(ctx.w, payer)
                    }
                });
                let (mut paid, mut debt, mut mothballed) =
                    settle(charge, row.debt_minor, avail, row.mothballed, standing);
                if let Some(payer) = payer {
                    let key = IdempotencyKey::new(UPKEEP_KEY, format!("{}/{day}", room.0));
                    match transfer(
                        ctx,
                        key,
                        (payer, sink),
                        paid,
                        (
                            UPKEEP_KEY,
                            u64::from(room.0),
                            format!("running costs of #{} for IC day {day}", room.0),
                        ),
                    ) {
                        TransferOutcome::Posted => {}
                        TransferOutcome::NeedsApproval if by_batch => break,
                        TransferOutcome::NeedsApproval => {
                            // P2-TSW-34: an approval rule or rolling window never freezes upkeep
                            // on owner-held or slot facilities; treat the blocked payment as
                            // unpaid so debt accrues, the facility mothballs, and `row.day`
                            // advances. Authored mill-run refineries (`by_batch`) keep G11's
                            // batch-hold semantics until the house account clears.
                            (paid, debt, mothballed) =
                                settle(charge, row.debt_minor, 0, row.mothballed, standing);
                        }
                        TransferOutcome::Failed => break,
                    }
                }
                if mothballed != row.mothballed {
                    tracing::info!(
                        facility = room.0,
                        debt,
                        mothballed,
                        "tacsim: a facility's running costs changed its standing"
                    );
                }
                if let Some(batches) = batches {
                    row.batches = batches;
                }
                row.clear_closed_through(day);
                row.day = day;
                row.charged_minor = charge;
                row.paid_minor = row.paid_minor.saturating_add(paid);
                row.org = org;
                row.debt_minor = debt;
                row.mothballed = mothballed;
            }
            next.facilities.insert(room.0, row);
        }
        if kept.as_ref() != Some(&next) {
            crate::store::put_port_upkeep(ctx, &next);
        }
    }
}

/// The authored price of `good` to `vendor`: its bid's, else its ask's, whole credits.
fn authored_price(vendor: &crate::port::def::VendorDef, good: &CanonicalId) -> Option<u64> {
    vendor
        .bids
        .iter()
        .find(|bid| &bid.good == good)
        .map(|bid| bid.price_major)
        .or_else(|| {
            vendor
                .asks
                .iter()
                .find(|ask| &ask.good == good)
                .map(|ask| ask.price_major)
        })
}

/// **The faucet pays each consumer back** for its acts: the units at the good's authored price,
/// at most the row's daily share, once an act. Each row's last act paid is the port's record's
/// ([`PortUpkeep::refunded`]), so a downtime of any length pays every act the ledger's ring still
/// holds, and an act is posted once (services G15).
fn refunds(ctx: &mut Ctx<'_>) {
    let due: Vec<_> = crate::port::consume::ledgers(ctx)
        .into_iter()
        .filter(|(_, _, row, _)| row.export_major_per_ic_day > 0)
        .collect();
    if due.is_empty() {
        return;
    }
    let Some(faucet) = system_account(ctx, EXPORT_FAUCET) else {
        return;
    };
    let scale = crate::port::apply::credit_scale(ctx.w);
    for (map, vendor, row, ledger) in due {
        let Some(payee) = map
            .orgs
            .get(&vendor.org)
            .and_then(|org| org.account)
            .map(AccountId)
        else {
            continue;
        };
        let Some(price) = authored_price(&vendor, &row.good) else {
            continue;
        };
        let Ok(id) = ledger.consume_id() else {
            continue;
        };
        // The port's record carries the cursor; running costs open it on the same fire.
        let Some(mut doc) = ctx
            .w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| store.port_upkeep(&map.entry).cloned())
        else {
            continue;
        };
        let row_key = id.key_text();
        let through = doc.refunded.get(row_key.as_str()).copied().unwrap_or(0);
        let mut acts: Vec<_> = ledger
            .recent
            .iter()
            .filter(|act| act.ordinal > through)
            .collect();
        acts.sort_by_key(|act| act.ordinal);
        let mut paid_through = through;
        for act in acts {
            let major = refund_major(act.units, price, row.export_major_per_ic_day);
            let Some(minor) = crate::port::apply::minor_of(major, scale) else {
                break;
            };
            let key = IdempotencyKey::new(EXPORT_KEY, format!("{row_key}/{}", act.ordinal));
            if transfer(
                ctx,
                key,
                (faucet, payee),
                minor,
                (
                    EXPORT_KEY,
                    act.ordinal,
                    format!(
                        "the export buyers pay {} for {} {}",
                        vendor.org, act.units, row.good
                    ),
                ),
            ) != TransferOutcome::Posted
            {
                break;
            }
            paid_through = act.ordinal;
        }
        if paid_through != through {
            doc.refunded.insert(SmolStr::new(&row_key), paid_through);
            crate::store::put_port_upkeep(ctx, &doc);
        }
    }
}

/// **The House stipends** (Q-48-55): each named House the launch deployment gives a stipend is
/// paid it once an IC day, into its treasury.
fn stipends(ctx: &mut Ctx<'_>) {
    let houses: Vec<(OrgId, AccountId, i64)> = ctx
        .w
        .module_state::<crate::launch::registration::LaunchService>()
        .map(|launch| {
            let houses = &launch.0.deployment().houses;
            [&houses.meridiani, &houses.coprates, &houses.tycho]
                .into_iter()
                .filter_map(|house| {
                    house
                        .stipend_minor_per_ic_day
                        .filter(|minor| *minor > 0)
                        .map(|minor| (house.organization, house.treasury, minor))
                })
                .collect()
        })
        .unwrap_or_default();
    if houses.is_empty() {
        return;
    }
    let today = ctx.w.world_ic_now().at.div_euclid(IC_DAY);
    let Some(faucet) = system_account(ctx, EXPORT_FAUCET) else {
        return;
    };
    for (org, named, minor) in houses {
        // Its root-scope treasury, which buys its licences and hulls; else the treasury the
        // deployment names.
        let treasury = crate::port::pools::treasury_of(ctx.w, org).unwrap_or(named);
        let key = IdempotencyKey::new(STIPEND_KEY, format!("{}/{today}", org.0));
        if mush_economy::idempotency_resolved(ctx, &key) {
            continue;
        }
        let _ = transfer(
            ctx,
            key,
            (faucet, treasury),
            minor,
            (
                STIPEND_KEY,
                org.0,
                format!("the House stipend for IC day {today}"),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stickney's works (§4.4): 1,693.16 cr a day over two slots.
    fn works() -> FacilityCostDef {
        FacilityCostDef {
            capital_major: 3_000_000,
            lifetime_years: 25,
            upkeep_permille: 20,
            staff: 8,
            wage_major: 150,
            margin_permille: 250,
        }
    }

    /// **The cost follows the work** (Q-48-38): an idle day is 10% of the daily cost, a day of
    /// both slots worked is the whole of it, and a mothballed idle day owes nothing.
    #[test]
    fn ex9_a_days_running_cost_follows_the_work() {
        let daily = i64::try_from(works().daily_cost_minor()).unwrap();
        assert_eq!(daily, 169_316);
        assert_eq!(day_charge(&works(), 2, 0, false), 16_931);
        assert_eq!(day_charge(&works(), 2, 2 * 86_400, false), daily);
        // One slot worked six IC hours: its share in full, the rest at a tenth.
        let six = day_charge(&works(), 2, 6 * 3_600, false);
        assert_eq!(six, 21_165 + 14_815);
        assert_eq!(day_charge(&works(), 2, 0, true), 0);
        assert_eq!(day_charge(&works(), 2, 6 * 3_600, true), 21_165);
        // A worked share past the slots is capped.
        assert_eq!(day_charge(&works(), 2, 9 * 86_400, false), daily);
        assert_eq!(standing_day(&works(), 2), 16_931);
    }

    /// **A mill-run refinery pays a slot-hour a batch** (Q-48-59).
    #[test]
    fn ex9_a_refinery_pays_by_the_batch() {
        let kasei = FacilityCostDef {
            capital_major: 6_000_000,
            lifetime_years: 30,
            upkeep_permille: 20,
            staff: 10,
            wage_major: 150,
            margin_permille: 0,
        };
        let per = i64::try_from(kasei.daily_cost_minor().div_ceil(24)).unwrap();
        assert_eq!(batch_charge(&kasei, 0), 0);
        assert_eq!(batch_charge(&kasei, 2), 2 * per);
    }

    /// **Insolvency** (Q-48-38, -59): what cannot be paid is a debt and mothballs; income pays
    /// the debt first; the facility reopens once it can pay seven days ahead.
    #[test]
    fn ex9_a_house_that_cannot_pay_mothballs_and_repays() {
        // Paid in full.
        assert_eq!(settle(100, 0, 1_000, false, 100), (100, 0, false));
        // Short: the rest is a debt, and it mothballs.
        assert_eq!(settle(100, 0, 40, false, 100), (40, 60, true));
        // Income pays the debt first; with less than seven days ahead it stays mothballed.
        assert_eq!(settle(0, 60, 500, true, 100), (60, 0, true));
        // Seven days ahead after the debt: it reopens.
        assert_eq!(settle(0, 60, 760, true, 100), (60, 0, false));
        // Nothing to pay with: the debt grows, never minted.
        assert_eq!(settle(30, 60, 0, true, 100), (0, 90, true));
    }

    /// **The faucet pays a consumer back at the price, within its share** (Q-48-40).
    #[test]
    fn ex9_a_refund_is_the_units_at_the_price_within_the_share() {
        assert_eq!(refund_major(40, 460, 18_400), 18_400);
        assert_eq!(refund_major(20, 460, 18_400), 9_200);
        assert_eq!(refund_major(50, 460, 18_400), 18_400, "capped at the share");
        assert_eq!(refund_major(5, 460, 0), 0, "no share, no refund");
    }

    /// **A restart resumes the day's tally from the row's checkpoint** (doc 48e, services G10):
    /// the work before it is kept, not lost to the idle rate; a checkpoint from a later day than
    /// today is no seed; and one is taken hourly and at each day's close.
    #[test]
    fn g10_a_restart_resumes_the_days_tally() {
        let checkpoint = Worked {
            day: 0,
            at_ic: 3_600,
            slot_secs: 7_200,
        };
        let mut resumed = resume_tally(Some(checkpoint), 0, 7_200);
        assert_eq!(resumed.slot_secs, 7_200, "the hours before the restart");
        assert_eq!(resumed.add(7_200, 2), None);
        assert_eq!(
            resumed.slot_secs, 14_400,
            "and the hour since, at two slots"
        );
        // Yesterday's checkpoint closes yesterday on the first add.
        let mut late = resume_tally(Some(checkpoint), 1, IC_DAY + 60);
        assert_eq!(late.add(IC_DAY + 60, 0), Some((0, 7_200)));
        // None, or one from a day not yet begun: a fresh tally.
        let fresh = resume_tally(None, 0, 500);
        assert_eq!((fresh.day, fresh.at_ic, fresh.slot_secs), (0, 500, 0));
        let ahead = Worked {
            day: 2,
            ..checkpoint
        };
        assert_eq!(resume_tally(Some(ahead), 0, 500).slot_secs, 0);
        // Checkpoints: the first, hourly, and at a close.
        assert!(checkpoint_due(None, 0, false));
        assert!(!checkpoint_due(Some(checkpoint), 3_600 + 3_599, false));
        assert!(checkpoint_due(Some(checkpoint), 3_600 + 3_600, false));
        assert!(checkpoint_due(Some(checkpoint), 3_601, true));
    }

    /// The day's tally closes at the IC day's turn, splitting the span across it.
    #[test]
    fn ex9_the_worked_tally_closes_at_the_day() {
        let mut worked = Worked {
            day: 0,
            at_ic: 0,
            slot_secs: 0,
        };
        assert_eq!(worked.add(3_600, 2), None);
        assert_eq!(worked.slot_secs, 7_200);
        // Over the turn: an hour to the old day's end at one slot, an hour into the new one.
        assert_eq!(worked.add(IC_DAY - 3_600, 0), None);
        assert_eq!(worked.add(IC_DAY + 3_600, 1), Some((0, 7_200 + 3_600)));
        assert_eq!(worked.day, 1);
        assert_eq!(worked.slot_secs, 3_600);
    }

    #[test]
    fn p2_tsw_34_closed_days_retain_up_to_max_catch_up_days_without_dropping_surcharges() {
        let mut row = UpkeepRow::default();
        for d in 1..=9 {
            row.record_closed_day(d, u64::try_from(d).expect("pos") * 3_600);
        }
        assert_eq!(
            row.closed_days.len(),
            usize::try_from(MAX_CATCH_UP_DAYS).expect("cap")
        );
        // Days 1 and 2 rolled off past MAX_CATCH_UP_DAYS = 7; days 3..=9 all kept.
        assert_eq!(row.closed_slot_secs(1), 0);
        assert_eq!(row.closed_slot_secs(2), 0);
        for d in 3..=9 {
            assert_eq!(
                row.closed_slot_secs(d),
                u64::try_from(d).expect("pos") * 3_600
            );
        }
        row.clear_closed_through(5);
        assert_eq!(row.closed_slot_secs(5), 0);
        assert_eq!(row.closed_slot_secs(6), 6 * 3_600);
        assert_eq!(row.closed.map(|c| c.day), Some(9));
    }

    #[test]
    fn p2_tsw_34_payable_available_respects_approval_threshold_and_window() {
        let rule = mush_economy::ApprovalRule {
            threshold: Amount::from_minor(500),
            policy: mush_org::ApprovalPolicy::OneAuthorizedActor,
            window: Some(mush_economy::ApprovalWindow {
                total: Amount::from_minor(1_000),
                period_secs: 86_400,
            }),
        };
        assert_eq!(
            payable_cap(50_000, Some(&rule), 0),
            500,
            "capped at approval threshold so upkeep never freezes on NeedsApproval"
        );
        assert_eq!(
            payable_cap(50_000, Some(&rule), 800),
            200,
            "capped at remaining rolling approval window headroom"
        );
        assert_eq!(
            payable_cap(50_000, Some(&rule), 1_000),
            0,
            "saturated rolling window leaves zero unconditionally payable funds"
        );
        // Unpaid portion accrues into debt and mothballs the facility.
        assert_eq!(settle(2_000, 0, 500, false, 1_000), (500, 1_500, true));
        assert_eq!(settle(2_000, 0, 0, false, 1_000), (0, 2_000, true));
    }
}
