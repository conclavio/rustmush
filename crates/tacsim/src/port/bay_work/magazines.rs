//! **A ship's space magazines in the bay** (doc 49 unit CC.7 part 2; Q-49-12).
//!
//! A space hull keeps what she fired ([`crate::space::custody::SPACE_MAGAZINES_CAPABILITY`]): her
//! tube families, spinal guns and point-defence rotaries hold what she left in them until she is
//! rearmed. Docked at a port with a workshop she is the bay's own subject
//! ([`super::facility_of`]), and `+bay/order <ship>=rearm` rearms her as the ground's rearm does a
//! tank's mount, with three differences:
//!
//! - **The rounds are PL.3's space munitions goods** (doc 48): torpedoes, spinal rounds and
//!   point-defence boxes, each the good whose `munitions` row names the family's ammunition.
//! - **Whole units only, so no round is lost.** A rearm loads the whole units of the good that
//!   fit in what the magazine is short. A box of point-defence rounds is 500 of them, so a
//!   rotary short by less than a box waits for the next. The ground's rearm rounds a short
//!   magazine up to a whole unit and loads only what fits.
//! - **The hold first, then the workshop's stores.** The rounds come from her hold where it
//!   carries them. The shortfall is bought from the workshop's stores (the operator's
//!   storefront, as a replaced part is) at the good's catalogue price, and that price joins the
//!   fee held in escrow ([`split`]). Both are drawn when the work is done, in the order's one draw.
//!
//! The labour is the tariff's per round and its time per mount, at the operator's multiplier, as
//! the ground's. The draw counts the rounds' kilograms into her bay record's `loaded`, which the
//! matter census reads (doc 48 cut EX.0).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx};
use mush_economy::VaultId;
use mush_inventory::record::EndpointRef;
use mush_tactics::field_ops::WorkshopFacilityTariff;
use mush_tactics::unit::ComponentId;
use mush_tactics::{TacticalUnit, UnitMutationPlan, apply};

use super::act::GoodUnits;
use super::jobs::{BayJob, QuotedJob};
use crate::definitions::GoodsTable;
use crate::space::custody;

/// The twin tubes' ammunition: PL.3's `torpedo` good loads it.
pub const TORPEDO_AMMUNITION: &str = "tacsim.ammunition.torpedo";
/// The spinal gun's ammunition: PL.3's `spinal_round` good loads it.
pub const SPINAL_AMMUNITION: &str = "tacsim.ammunition.spinal_round";
/// The point-defence rotary's ammunition: PL.3's `pd_box` good loads it, 500 rounds a box.
pub const POINT_DEFENCE_AMMUNITION: &str = "tacsim.ammunition.pd_rotary";

/// **The rearm needs more than her hold and the stores carry**: the good, what the rearm needs,
/// what her hold carries and what the workshop's stores hold.
pub const SHORT_OF_ROUNDS: &str =
    "The rearm needs {need} {good}: her hold carries {hold} and the workshop's stores {store}.";

/// **One magazine of a space hull's unit**: her mount, the rounds it holds, what her fit stows,
/// and the ammunition that loads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Magazine {
    /// The mount's id on her hull.
    pub mount: String,
    /// The rounds it holds.
    pub left: u32,
    /// What her fit stows: the most it holds.
    pub stow: u32,
    /// The ammunition that loads it ([`TORPEDO_AMMUNITION`] and its siblings).
    pub ammunition: &'static str,
}

/// **Every magazine `unit` carries**, with what it holds: her fit's tube families, spinal guns
/// and point-defence rotaries, read against what her unit kept
/// ([`custody::kept_magazines`]). Empty for a unit that flies no space fit (a ground unit).
#[must_use]
pub fn magazines(unit: &TacticalUnit) -> Vec<Magazine> {
    let id = unit.schema().key().id.as_str();
    if custody::space_fit_for_id(id).is_none() {
        return Vec::new();
    }
    let full = custody::space_unit_overlay_for_id(id);
    let kept = custody::kept_magazines(unit);
    let magazine = |mount: &str, stow: u32, ammunition: &'static str| Magazine {
        mount: mount.to_owned(),
        left: kept.get(mount).copied().map_or(stow, |left| left.min(stow)),
        stow,
        ammunition,
    };
    let mut out = Vec::new();
    for tube in &full.torpedo_tubes {
        out.push(magazine(
            &tube.id,
            u32::from(tube.magazine_remaining),
            TORPEDO_AMMUNITION,
        ));
    }
    for gun in &full.railguns {
        out.push(magazine(
            &gun.id,
            u32::from(gun.rounds_remaining),
            SPINAL_AMMUNITION,
        ));
    }
    for pd in &full.pdcs {
        out.push(magazine(
            &pd.id,
            pd.ammo_remaining,
            POINT_DEFENCE_AMMUNITION,
        ));
    }
    out
}

/// **The rearm jobs `unit`'s magazines need** (the quote): for each magazine short of her fit,
/// the whole units of its good that fit in what it is short, and the rounds they hold. The
/// labour is `tariff.cost_per_ammo_round` a round and its time `duration_ic_rearm_component` a
/// mount, as the ground's rearm. A magazine short by less than one unit of its good, or whose
/// good the goods table does not carry, has no job.
#[must_use]
pub fn quote_jobs(
    unit: &TacticalUnit,
    goods: &GoodsTable,
    tariff: &WorkshopFacilityTariff,
) -> Vec<QuotedJob> {
    magazines(unit)
        .into_iter()
        .filter_map(|magazine| {
            let good = goods.munitions_for(magazine.ammunition)?;
            let per = good.munitions.as_ref()?.rounds_per_unit.max(1);
            let units = magazine.stow.saturating_sub(magazine.left) / per;
            if units == 0 {
                return None;
            }
            let rounds = units.saturating_mul(per);
            Some(QuotedJob {
                job: BayJob::Magazine {
                    mount: ComponentId::new(magazine.mount.as_str()).ok()?,
                    rounds: i64::from(rounds),
                    good: good.id.clone(),
                    units: u64::from(units),
                },
                cost_minor: u64::from(rounds).saturating_mul(tariff.cost_per_ammo_round),
                secs: tariff.duration_ic_rearm_component,
                points: 0,
                part_minor: 0,
            })
        })
        .collect()
}

/// **The goods `jobs`' magazine rearms draw**, summed by good.
#[must_use]
pub fn magazine_goods(jobs: &[QuotedJob]) -> Vec<GoodUnits> {
    let mut goods: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    for job in jobs {
        if let BayJob::Magazine { good, units, .. } = &job.job {
            let sum = goods.entry(good.clone()).or_default();
            *sum = sum.saturating_add(*units);
        }
    }
    goods
        .into_iter()
        .filter(|(_, units)| *units > 0)
        .map(|(good, units)| GoodUnits { good, units })
        .collect()
}

/// **Where a rearm's rounds come from**: what her hold gives and what the workshop's stores
/// sell, by good.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Drawn from her hold.
    pub hold: Vec<GoodUnits>,
    /// Bought from the workshop's stores.
    pub store: Vec<GoodUnits>,
}

/// The catalogue price of one unit of `good`, minor units: what the stores sell it for.
fn unit_price(goods: &GoodsTable, good: &CanonicalId) -> u64 {
    goods
        .good(good)
        .map_or(0, |def| def.reference_major.saturating_mul(100))
}

/// **Split `jobs`' magazine rounds between her hold and the workshop's stores, and price what
/// is bought** (the module head). For each good: as many units as `hold` carries come from it,
/// and the rest from `stores` at the good's catalogue price. That price is set on each job's
/// `part_minor` for the units it buys, so the order's fee holds it in escrow. Both are read as
/// the draw reads them, by what is free: units a market listing holds (the stock clerk's asks)
/// are not for the bay to sell.
///
/// # Errors
///
/// [`SHORT_OF_ROUNDS`] when the hold and the stores together carry too few of a good, or a
/// sentence when a pool cannot be read.
pub fn split(
    ctx: &Ctx<'_>,
    hold: Option<VaultId>,
    stores: Option<VaultId>,
    goods: &GoodsTable,
    jobs: &mut [QuotedJob],
) -> Result<Split, String> {
    let read = |vault: Option<VaultId>, good: &CanonicalId| -> Result<u64, String> {
        vault.map_or(Ok(0), |vault| {
            crate::port_tick::read_available(ctx, EndpointRef(vault), good)
                .map_err(|_| "The rounds' stores cannot be read.".to_owned())
        })
    };
    let mut out = Split::default();
    for need in magazine_goods(jobs) {
        let in_hold = read(hold, &need.good)?;
        let from_hold = need.units.min(in_hold);
        let to_buy = need.units - from_hold;
        if to_buy > 0 {
            let in_store = read(stores, &need.good)?;
            if in_store < to_buy {
                return Err(SHORT_OF_ROUNDS
                    .replace("{need}", &need.units.to_string())
                    .replace("{good}", need.good.as_str())
                    .replace("{hold}", &in_hold.to_string())
                    .replace("{store}", &in_store.to_string()));
            }
            out.store.push(GoodUnits {
                good: need.good.clone(),
                units: to_buy,
            });
        }
        if from_hold > 0 {
            out.hold.push(GoodUnits {
                good: need.good.clone(),
                units: from_hold,
            });
        }
        // The hold's units go to the jobs first, in quote order; each job pays for the rest.
        let price = unit_price(goods, &need.good);
        let mut hold_left = from_hold;
        for job in jobs.iter_mut() {
            if let BayJob::Magazine { good, units, .. } = &job.job
                && *good == need.good
            {
                let taken = (*units).min(hold_left);
                hold_left -= taken;
                job.part_minor = (*units - taken).saturating_mul(price);
            }
        }
    }
    Ok(out)
}

/// **Load `rounds` into `mount`'s magazine on `unit`** (the work): what it holds rises by
/// `rounds`, never past what her fit stows, and her unit's magazine record is rewritten
/// ([`custody::magazine_op`]): dropped when every magazine is full.
///
/// # Errors
///
/// A sentence when `mount` is no magazine of hers or the record cannot be written.
pub fn load(
    unit: &TacticalUnit,
    mount: &ComponentId,
    rounds: i64,
    at_ic: i64,
) -> Result<TacticalUnit, String> {
    let mut all = magazines(unit);
    let magazine = all
        .iter_mut()
        .find(|magazine| magazine.mount == mount.as_str())
        .ok_or_else(|| format!("{mount} is no magazine of hers"))?;
    let rounds = u32::try_from(rounds.max(0)).unwrap_or(u32::MAX);
    magazine.left = magazine.left.saturating_add(rounds).min(magazine.stow);
    let record: BTreeMap<String, u32> = all
        .iter()
        .filter(|magazine| magazine.left < magazine.stow)
        .map(|magazine| (magazine.mount.clone(), magazine.left))
        .collect();
    let Some(op) = custody::magazine_op(unit, &record).map_err(|e| e.to_string())? else {
        return Ok(unit.clone());
    };
    let plan =
        UnitMutationPlan::new(vec![op], Some(unit.revision()), None).map_err(|e| e.to_string())?;
    apply(unit, &plan, at_ic)
        .map(|(after, _)| after)
        .map_err(|e| e.to_string())
}

/// **The op that carries `after`'s magazine record onto `before`**, if it changed (the order's
/// completion): the condition plan the bay writes carries cells and conditions only.
///
/// # Errors
///
/// A sentence when the record cannot be written.
pub fn record_op(
    before: &TacticalUnit,
    after: &TacticalUnit,
) -> Result<Option<mush_tactics::UnitMutationOp>, String> {
    custody::magazine_op(before, &custody::kept_magazines(after)).map_err(|e| e.to_string())
}

/// `3 torpedo, 2 pd_box`.
fn words(goods: &[GoodUnits]) -> String {
    goods
        .iter()
        .map(|row| format!("{} {}", row.units, row.good))
        .collect::<Vec<_>>()
        .join(", ")
}

/// **The quote's line for a rearm's rounds**: what comes from her hold and what is bought from
/// the workshop's stores, with its price (`money` writes a sum of minor units).
#[must_use]
pub fn split_line(split: &Split, bought_minor: u64, money: &dyn Fn(u64) -> String) -> String {
    match (split.hold.is_empty(), split.store.is_empty()) {
        (_, true) => format!("  Rounds from her hold: {}.", words(&split.hold)),
        (true, false) => format!(
            "  Rounds bought from the workshop's stores: {}, {}.",
            words(&split.store),
            money(bought_minor)
        ),
        (false, false) => format!(
            "  Rounds from her hold: {}; bought from the workshop's stores: {}, {}.",
            words(&split.hold),
            words(&split.store),
            money(bought_minor)
        ),
    }
}

/// **What a taken order buys from the stores**, for its reply: ` Bought from the workshop's
/// stores: 5 torpedo.`, or nothing.
#[must_use]
pub fn bought_words(store: &[GoodUnits]) -> String {
    if store.is_empty() {
        String::new()
    } else {
        format!(" Bought from the workshop's stores: {}.", words(store))
    }
}
