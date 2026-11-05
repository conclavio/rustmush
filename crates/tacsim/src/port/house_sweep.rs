//! **A recovery house's sweep** (doc 41 cut RS.6c-1; §4.4 "On its own account", Q-41-16, round-2
//! item 41-7; the owner's rulings of 2026-11-04 on the ownerless wreck and owned ground).
//!
//! When the house holds no retrieval, its arm ([`crate::port::house_arm`]) sweeps: a wreck that has
//! lain on a lane site's index for [`SWEEP_IC_DAYS`] with no salvage claim filed and no retrieval
//! posted, on open ground or a Registry seed (`open_ground`: a player's claimed block is left
//! alone until a later cut builds the registry-notice ask), is recovered by the house on its own
//! account:
//!
//! - **An ownerless wreck** (no title on the economy: a raid's hulls are made with none) is titled
//!   to the house's dispatcher under the registry issuer before the tow, as salvage of the unowned
//!   (ruling 1), so the load is the house's own and no claim is filed.
//! - **A titled wreck** is loaded into the house's custody and its salvage claim filed by the
//!   dispatcher on the one claim path (`+salvage/claim`), awarded to the dispatcher at the
//!   window's end (an Object claimant, round-2 item 41-7).
//!
//! At home, berthed in the workshop and titled to the dispatcher, she is **assessed** ([`assess`]):
//! the full restoration quote — the workshop's fee (labour at the operator's multiplier and every
//! replaced part) and the bale's alloys and fittings at their references — against the design's
//! reference price less [`RESALE_MARGIN_PERMILLE`]. Under the line she is **kept**: her title passes
//! to the house (cut RS.6c-2 repairs her first and offers her for sale, `crate::port::house_sale`). Over it, past repair, or a design the
//! yard does not bill, she is **scrapped** at the workshop under IN.8, and the scrap's credit, paid
//! into the dispatcher's float, goes back to the house's account.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::AssetOp;
use mush_org::{OrgId, OrgScopeId, PrincipalId};

use crate::surface::act::{Where, where_is};
use crate::surface::site_record::LeftCondition;

/// **How long a wreck lies before the house sweeps it**, IC days (two write-off bars,
/// provisional; Q-41-16).
pub const SWEEP_IC_DAYS: i64 = 56;
/// **The resale margin** (Q-41-16, provisional): a hull is kept for resale only when her full
/// restoration quote is under her reference price less this share, per mille.
pub const RESALE_MARGIN_PERMILLE: u64 = 200;

/// **What the house does with a swept hull it holds** (Q-41-16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Repaired and offered for sale (RS.6c-2): her title passes to the house.
    Keep {
        /// The full restoration quote, minor units.
        quote_minor: u64,
        /// Her reference price, minor units.
        reference_minor: u64,
    },
    /// Broken up at the workshop under IN.8.
    Scrap {
        /// Why.
        why: String,
    },
}

/// **Whether `cell` of `site` is open to the sweep** (the owner's ruling 2): open ground or a
/// Registry seed — anything but a block a player holds.
fn open_ground(ctx: &Ctx<'_>, site: &CanonicalId, cell: (i64, i64)) -> bool {
    matches!(
        crate::surface::claim::tenure_at(ctx, site, cell),
        mush_tactics::field_ops::GroundTenure::Unclaimed
    )
}

/// **The wreck the house sweeps next**, `(hull, site)`: the first wreck on a lane site's index
/// past [`SWEEP_IC_DAYS`], on open ground, with no custody row and no live retrieval, on a site
/// no field holds.
#[must_use]
pub fn target(ctx: &Ctx<'_>) -> Option<(ObjId, CanonicalId)> {
    let now = ctx.w.world_ic_now().at;
    let bar = SWEEP_IC_DAYS.saturating_mul(86_400);
    for site in crate::surface::chart::lane_sites(ctx.w) {
        let record = crate::surface::tow::record_or_new(ctx, &site.entry);
        if record.open_field().is_some() {
            continue;
        }
        for left in &record.units {
            if left.condition != LeftCondition::Wreck || now.saturating_sub(left.left_ic) < bar {
                continue;
            }
            let hull = left.unit;
            let cell = (
                left.pose.x_cm.div_euclid(1_000),
                left.pose.y_cm.div_euclid(1_000),
            );
            if crate::surface::salvage::custody_of(ctx, hull).is_some()
                || crate::surface::retrieval::order_of(ctx.w, hull).is_some_and(|o| o.active())
                || !open_ground(ctx, &site.entry, cell)
            {
                continue;
            }
            return Some((hull, site.entry.clone()));
        }
    }
    None
}

/// The title holder of `hull` on the economy, if titled.
#[must_use]
pub fn title_of(w: &World, hull: ObjId) -> Option<PrincipalId> {
    w.module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .asset(hull)
        .map(|asset| asset.title_owner.clone())
}

/// **Title an ownerless wreck to the dispatcher** (the owner's ruling 1: salvage of the unowned),
/// under the registry issuer.
///
/// # Errors
///
/// The economy's refusal.
pub(crate) fn title_ownerless(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<(), String> {
    let issuer = mush_org::SystemPrincipalId::new(crate::port::hull::REGISTRY_ISSUER)
        .ok_or("the registry issuer is not a system principal")?;
    mush_economy::issue_title(
        ctx,
        hull,
        PrincipalId::Object(ctx.actor),
        None,
        issuer,
        None,
    )
    .map(|_| ())
    .map_err(|e| format!("the wreck could not be titled: {e}"))
}

/// **The assessment** (Q-41-16): kept for resale when her full restoration quote is under her
/// reference price less the margin, else scrapped. Read where she stands (berthed in the home
/// workshop).
#[must_use]
pub fn assess(w: &World, hull: ObjId) -> Disposition {
    let Some((design, budget)) = crate::definitions::mass::hull_budget(w, hull) else {
        return Disposition::Scrap {
            why: "the yard does not bill her design".to_owned(),
        };
    };
    let quote = match crate::port::bay_work::quote_here(w, hull) {
        Ok((_, quote)) => quote,
        Err(why) => return Disposition::Scrap { why },
    };
    let fee = crate::port::bay_work::BayQuote::fee_minor(&quote.jobs);
    let bale = crate::port::bay_work::BayQuote::bale_need(&quote.jobs, Some(&budget));
    let per_t = |good: &str| crate::definitions::mass::good_reference_minor(w, good);
    let materials = bale
        .alloys_kg
        .saturating_mul(per_t("structural_alloys"))
        .saturating_add(bale.fittings_kg.saturating_mul(per_t("industrial_parts")))
        .div_ceil(1_000);
    let quote_minor = fee.saturating_add(materials);
    let reference_minor = crate::definitions::mass::reference_price_minor(w, &design, &budget);
    let line =
        reference_minor.saturating_mul(1_000u64.saturating_sub(RESALE_MARGIN_PERMILLE)) / 1_000;
    if quote_minor < line {
        Disposition::Keep {
            quote_minor,
            reference_minor,
        }
    } else {
        Disposition::Scrap {
            why: format!(
                "her restoration ({quote_minor} minor) is not under her resale line ({line} minor)"
            ),
        }
    }
}

/// **Pass a kept hull's title to the house** (round-2 item 41-7): one `TransferTitle` from the
/// dispatcher to the operator at its root scope, under the dispatcher's own authority (it holds
/// `asset.title.transfer` there, `crate::port::house`).
///
/// # Errors
///
/// The economy's refusal.
pub(crate) fn to_house(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    (org, scope): (OrgId, OrgScopeId),
) -> Result<(), String> {
    let mut plan = mush_economy::EconomyPlan::new(
        mush_economy::IdempotencyKey::new("tacsim.house.title", hull.0.to_string()),
        PrincipalId::Object(ctx.actor),
        mush_economy::EconomicCause::Internal {
            kind: smol_str::SmolStr::new_static("tacsim.house.title"),
            id: u64::from(hull.0),
        },
        Vec::new(),
    );
    plan.asset_ops = vec![AssetOp::TransferTitle {
        object: hull,
        to: PrincipalId::Organization(org),
        to_scope: Some(scope),
    }];
    plan.description = "a swept hull to the house".to_owned();
    mush_economy::post(ctx, &plan)
        .map(|_| ())
        .map_err(|e| format!("the title did not pass to the house: {e}"))
}

/// **The scrap's credit back to the house**: `credit` minor units — what the scrap paid into the
/// dispatcher's float — posted from the float to the house's account.
///
/// # Errors
///
/// The economy's refusal.
pub(crate) fn return_surplus(
    ctx: &mut Ctx<'_>,
    (float, house): (u64, u64),
    credit: u64,
) -> Result<(), String> {
    let surplus = i64::try_from(credit).unwrap_or(i64::MAX);
    if surplus <= 0 {
        return Ok(());
    }
    let mut plan = mush_economy::EconomyPlan::new(
        mush_economy::IdempotencyKey::new(
            "tacsim.house.surplus",
            format!("{float}/{}", ctx.w.world_ic_now().at),
        ),
        PrincipalId::Object(ctx.actor),
        mush_economy::EconomicCause::Internal {
            kind: smol_str::SmolStr::new_static("tacsim.house.surplus"),
            id: float,
        },
        vec![
            mush_economy::RequestedLine {
                account: mush_economy::AccountId(float),
                delta: mush_economy::Amount::from_minor(-surplus),
                memo: "surplus".into(),
            },
            mush_economy::RequestedLine {
                account: mush_economy::AccountId(house),
                delta: mush_economy::Amount::from_minor(surplus),
                memo: "surplus".into(),
            },
        ],
    );
    plan.description = "the sweep's proceeds to the house".to_owned();
    mush_economy::post(ctx, &plan)
        .map(|_| ())
        .map_err(|e| format!("the proceeds did not post: {e}"))
}

/// **The swept hull the house holds now**, if any: one in the lifter's bay that is neither the
/// crawler nor a retrieval's, or one berthed in the home workshop titled to the dispatcher or in
/// its custody, not yet scrapped.
#[must_use]
pub fn held(
    ctx: &Ctx<'_>,
    (lifter, crawler): (ObjId, ObjId),
    room: Option<ObjId>,
) -> Option<ObjId> {
    let dispatcher = crate::port::coop::helm_holder(ctx.w, lifter)?;
    let ours = |hull: ObjId| {
        hull != crawler
            && hull != lifter
            && crate::surface::retrieval::order_of(ctx.w, hull).is_none_or(|o| !o.active())
            && (title_of(ctx.w, hull) == Some(PrincipalId::Object(dispatcher))
                || crate::surface::salvage::custody_of(ctx, hull)
                    .is_some_and(|row| row.holder == dispatcher))
            && !crate::port::bay_work::scrapped(ctx.w, hull)
    };
    let bay = crate::port::ship::hull_view(ctx.w, lifter)
        .and_then(|view| view.bay().map(|(_, room)| room));
    [bay, room]
        .into_iter()
        .flatten()
        .flat_map(|at| ctx.w.contents_of(at).to_vec())
        .find(|hull| {
            ours(*hull)
                && (where_is(ctx.w, *hull) == (Where::Bay { hull: lifter })
                    || (room.is_some() && ctx.w.get(*hull).and_then(|o| o.location) == room))
        })
}
