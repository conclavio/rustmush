//! **Running costs, insolvency and the export faucet** (doc 48 unit EX.9; Q-48-21, Q-48-22,
//! Q-48-36, Q-48-38, Q-48-39, Q-48-40, Q-48-55, Q-48-59, Q-48-60).
//!
//! **The running cost follows the work** (Q-48-38). Every costed facility at a laid-up port (its
//! refinery, works, yard and workshop, whoever owns it) pays its owner's share of its authored
//! daily cost (`mush_industry::FacilityCostDef::daily_cost_minor`, payroll included at 150 cr a
//! head, Q-48-60) once an IC day, from its owner's account to the System account
//! [`UPKEEP_SINK`]: all of it to a sink (Q-48-21's correction), nothing to a payee.
//! - **A worked slot-hour** costs its share of the daily cost (`daily / slots / 24`), which the
//!   fee recovers with its margin. The slots worked are the facility's open work orders
//!   (`mush_industry::IndustryStore::open_orders_at`) and open bay orders, at most its slots,
//!   summed over the IC day on every port fire ([`Worked`], RAM only: a restart counts the rest of
//!   that day's work as idle).
//! - **An idle slot-hour** costs [`IDLE_PERMILLE`] of it (10%, provisional).
//! - **A mill-run refinery** (Q-48-59: a facility nobody can book) pays one slot-hour of its daily
//!   cost a refine batch its operator milled that day, and no standing share.
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
/// How far back the faucet looks for a consumption act to pay, IC seconds.
const REFUND_WINDOW_IC: i64 = 2 * IC_DAY;

/// **One facility's worked slot-seconds this IC day** (RAM; `PortTickState::upkeep_worked`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// What an account can pay now: its balance less its holds, minor units.
fn available(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).and_then(Account::available))
        .map_or(0, Amount::minor)
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

/// **One keyed transfer** of `minor` from `from` to `to`, as the tick. `Ok(true)` when it is
/// posted (now or before), `Ok(false)` when there is nothing to move, `Err` when it must wait.
fn transfer(
    ctx: &mut Ctx<'_>,
    key: IdempotencyKey,
    (from, to): (AccountId, AccountId),
    minor: i64,
    (scope, id, description): (&'static str, u64, String),
) -> Result<bool, ()> {
    if minor <= 0 {
        return Ok(false);
    }
    let (Some(debit), credit) = (
        Amount::from_minor(minor).checked_neg(),
        Amount::from_minor(minor),
    ) else {
        return Err(());
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
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(true),
        Ok(PostOutcome::NeedsApproval(_)) | Err(_) => Err(()),
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
            // The busy slots now, summed into the day.
            let busy = {
                let orders = ctx
                    .w
                    .module_state::<IndustryStore>()
                    .map_or(0, |store| store.open_orders_at(room));
                let bays = ctx
                    .w
                    .module_state::<crate::store::TacsimStore>()
                    .map_or(0, |store| store.bay_open_at(room));
                u64::try_from(orders.saturating_add(bays))
                    .unwrap_or(u64::MAX)
                    .min(u64::from(slots))
            };
            let closed = ctx
                .w
                .module_state_mut::<crate::port_tick::PortTickState>()
                .and_then(|state| {
                    let worked = state.upkeep_worked.entry(room.0).or_insert(Worked {
                        day: today,
                        at_ic: now,
                        slot_secs: 0,
                    });
                    worked.add(now, busy)
                });
            let owner = record.owner().clone();
            let org = match &owner {
                PrincipalId::Organization(org) => Some(org.0),
                _ => None,
            };
            let refinery = block == "refinery";
            let operator = def.refinery.as_ref().map(|r| r.operator.clone());
            let Some(mut row) = next.facilities.get(&room.0).cloned() else {
                // First sight: charging starts with today.
                let batches = match (&operator, refinery) {
                    (Some(operator), true) => refine_batches(ctx, &map, &def, operator),
                    _ => 0,
                };
                next.facilities.insert(
                    room.0,
                    UpkeepRow {
                        day: today - 1,
                        org,
                        batches,
                        ..UpkeepRow::default()
                    },
                );
                continue;
            };
            if row.day >= today - 1 && closed.is_none_or(|(day, _)| day <= row.day) {
                continue;
            }
            let Some(payer) = payer_of(ctx.w, &owner) else {
                continue;
            };
            // Each closed day since the last charge, the oldest skipped past the catch-up bound.
            let from = (row.day + 1).max(today - MAX_CATCH_UP_DAYS);
            for day in from..today {
                let charge = if refinery {
                    if day == today - 1 {
                        let total = operator
                            .as_ref()
                            .map_or(row.batches, |op| refine_batches(ctx, &map, &def, op));
                        let charge = batch_charge(&cost, total.saturating_sub(row.batches));
                        row.batches = total.max(row.batches);
                        charge
                    } else {
                        0
                    }
                } else {
                    let worked = closed
                        .filter(|(closed_day, _)| *closed_day == day)
                        .map_or(0, |(_, secs)| secs);
                    day_charge(&cost, slots, worked, row.mothballed)
                };
                let standing = if refinery {
                    0
                } else {
                    standing_day(&cost, slots)
                };
                let (paid, debt, mothballed) = settle(
                    charge,
                    row.debt_minor,
                    available(ctx.w, payer),
                    row.mothballed,
                    standing,
                );
                let key = IdempotencyKey::new(UPKEEP_KEY, format!("{}/{day}", room.0));
                if transfer(
                    ctx,
                    key,
                    (payer, sink),
                    paid,
                    (
                        UPKEEP_KEY,
                        u64::from(room.0),
                        format!("running costs of #{} for IC day {day}", room.0),
                    ),
                )
                .is_err()
                {
                    break;
                }
                if mothballed != row.mothballed {
                    tracing::info!(
                        facility = room.0,
                        debt,
                        mothballed,
                        "tacsim: a facility's running costs changed its standing"
                    );
                }
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

/// **The faucet pays each consumer back** for its recent acts: the units at the good's authored
/// price, at most the row's daily share, once an act.
fn refunds(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now().at;
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
        for act in ledger
            .recent
            .iter()
            .filter(|act| act.at_ic >= now.saturating_sub(REFUND_WINDOW_IC))
        {
            let major = refund_major(act.units, price, row.export_major_per_ic_day);
            let Some(minor) = crate::port::apply::minor_of(major, scale) else {
                continue;
            };
            let key = IdempotencyKey::new(EXPORT_KEY, format!("{}/{}", id.key_text(), act.ordinal));
            if mush_economy::idempotency_resolved(ctx, &key) {
                continue;
            }
            let _ = transfer(
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
            );
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
}
