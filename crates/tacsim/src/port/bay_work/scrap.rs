//! **The scrap ceiling and the bale's return** (doc 39 cut IN.8; §4.6, §4.10, Q-39-16,
//! Q-39-17, Q-39-25, Q-39-26).
//!
//! At a workshop **everything goes to the operator** and the operator pays the title holder:
//!
//! ```text
//! parts_new         = Σ reference of each salvageable part
//! scrappable        = reference_price − parts_new
//! structural_credit = min(yield_kg × alloy_reference / 1,000, scrappable / 5)
//! part_credit       = Σ (reference × used − removal)        (used 0.75 intact, 0.40 worn)
//! bale_credit       = the hull bale's kilograms at their references (raw matter: no fifth)
//! ```
//!
//! The yield, the bale and the workshop's shop bale ([`super::ShopBale`]) sum per good; the whole
//! tonnes mint into the operator's storefront (its stock: "the Yards' pool") and the remainder
//! stays in the shop bale, so nothing under a tonne is lost at a facility. The parts go to the
//! same storefront as lots (its stores). `+bay/return` is the bale's half alone. The credit posts
//! as the operator, from its account to the holder's, when the mint lands (or at once when there
//! is nothing to mint), under an idempotency key of the hull and the act.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, EconomicCause, EconomyPlan, EconomyStore, IdempotencyKey, PostOutcome,
    RequestedLine,
};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Bale, KG_PER_T, ShopBale};

/// **A part's used value, per mille of its reference** (§4.6): intact.
pub const USED_INTACT_PERMILLE: u64 = 750;
/// A worn part's (grade 3: damaged or jammed).
pub const USED_WORN_PERMILLE: u64 = 400;
/// **The fifth** (Q-38-5): the structure is worth at most the scrappable value over this.
pub const CEILING_DIVISOR: u64 = 5;

/// The idempotency namespace a scrap's or a return's credit posts under.
pub const CREDIT_NAMESPACE: &str = "tacsim.bay_scrap";

/// One salvageable part a scrap takes: its reference and whether it is intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartIn {
    /// Its catalogue reference, minor units.
    pub reference_minor: u64,
    /// Intact, or worn.
    pub intact: bool,
}

/// **What a scrap or a return comes to** (§4.6, §4.10), minor units and kilograms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Terms {
    /// The structure's credit: the yield at the alloys' reference, at most the fifth.
    pub structural_minor: u64,
    /// The parts' used value less their removal.
    pub parts_minor: u64,
    /// The hull bale's kilograms bought back at the references.
    pub bale_minor: u64,
    /// Whole tonnes of `structural_alloys` into the operator's pool.
    pub alloys_t: u64,
    /// Whole tonnes of `industrial_parts`.
    pub fittings_t: u64,
    /// The shop bale after: what stays under a tonne.
    pub shop_after: Bale,
}

impl Terms {
    /// The holder's whole credit.
    #[must_use]
    pub const fn credit_minor(&self) -> u64 {
        self.structural_minor
            .saturating_add(self.parts_minor)
            .saturating_add(self.bale_minor)
    }
}

/// **The references a scrap reads**, minor units: the hull's, and a tonne of each bulk good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct References {
    /// The hull's reference price (§4.6: its bill, or a programme hull's buyout).
    pub hull_minor: u64,
    /// A tonne of `structural_alloys`.
    pub alloys_minor_t: u64,
    /// A tonne of `industrial_parts`.
    pub fittings_minor_t: u64,
}

/// **The terms** of a scrap yielding `yield_kg` of alloys with `parts` salvageable, each removed
/// at `removal_minor`, the hull's `bale` bought back, into a workshop whose shop bale holds
/// `shop` (a return is a scrap with no yield, no parts and no hull reference).
#[must_use]
pub fn terms(
    references: References,
    yield_kg: u64,
    parts: &[PartIn],
    removal_minor: u64,
    bale: Option<Bale>,
    shop: Bale,
) -> Terms {
    let parts_new = parts
        .iter()
        .fold(0u64, |sum, part| sum.saturating_add(part.reference_minor));
    let scrappable = references.hull_minor.saturating_sub(parts_new);
    let structural_minor =
        at_reference(yield_kg, references.alloys_minor_t).min(scrappable / CEILING_DIVISOR);
    let parts_minor = parts.iter().fold(0u64, |sum, part| {
        let used = if part.intact {
            USED_INTACT_PERMILLE
        } else {
            USED_WORN_PERMILLE
        };
        sum.saturating_add(
            (part.reference_minor.saturating_mul(used) / 1_000).saturating_sub(removal_minor),
        )
    });
    let bale = bale.unwrap_or_default();
    let bale_minor = at_reference(bale.alloys_kg, references.alloys_minor_t)
        .saturating_add(at_reference(bale.fittings_kg, references.fittings_minor_t));
    let alloys = shop
        .alloys_kg
        .saturating_add(yield_kg)
        .saturating_add(bale.alloys_kg);
    let fittings = shop.fittings_kg.saturating_add(bale.fittings_kg);
    Terms {
        structural_minor,
        parts_minor,
        bale_minor,
        alloys_t: alloys / KG_PER_T,
        fittings_t: fittings / KG_PER_T,
        shop_after: Bale {
            alloys_kg: alloys % KG_PER_T,
            fittings_kg: fittings % KG_PER_T,
        },
    }
}

/// `kg` at `minor_t` a tonne, rounded down.
fn at_reference(kg: u64, minor_t: u64) -> u64 {
    u64::try_from(u128::from(kg) * u128::from(minor_t) / u128::from(KG_PER_T)).unwrap_or(u64::MAX)
}

/// **The settlement an owed scrap or return carries** (doc 39 cut IN.8): the credit, who pays it
/// and to whom, the shop bale it leaves, and whether it empties the hull's bale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    /// The holder's credit, minor units.
    pub credit_minor: u64,
    /// The operator org, `OrgId.0`.
    pub operator: u64,
    /// The operator's scoped credit account, `AccountId.0`.
    pub operator_account: u64,
    /// The holder's credit account, `AccountId.0`.
    pub holder_account: u64,
    /// The shop bale once the mint lands (its `pending` cleared).
    pub shop: ShopBale,
    /// Whether the hull's own bale is emptied (bought back).
    pub clear_bale: bool,
}

/// **Whether the operator can pay `credit` now**: its account's available balance covers it.
#[must_use]
pub fn operator_covers(w: &World, account: AccountId, credit: u64) -> bool {
    credit == 0
        || w.module_state::<EconomyStore>()
            .and_then(|store| store.snapshot().account(account)?.available())
            .is_some_and(|available| {
                u64::try_from(available.minor()).is_ok_and(|have| have >= credit)
            })
}

/// **Land a settlement** for act `act` on `hull`, by `by`: the credit posts as the operator, the
/// shop bale is written, and the hull's bale emptied when it was bought back. A credit that does
/// not post is logged for staff (the matter has already moved).
pub(crate) fn land(ctx: &mut Ctx<'_>, hull: ObjId, act: u32, by: ObjId, settle: &Settlement) {
    crate::store::put_shop_bale(ctx, &settle.shop);
    if settle.clear_bale
        && let Ok(mut record) = super::record_of(ctx.w, hull)
    {
        record.bale = None;
        crate::store::put_bay_hull(ctx, &record);
    }
    if settle.credit_minor == 0 {
        return;
    }
    let posted = i64::try_from(settle.credit_minor).ok().and_then(|credit| {
        let debit = Amount::from_minor(credit).checked_neg()?;
        let plan = EconomyPlan {
            description: format!("Scrap and bale buy-back for #{}", hull.0),
            ..EconomyPlan::new(
                IdempotencyKey::new(CREDIT_NAMESPACE, format!("{}/{act}", hull.0)),
                PrincipalId::Organization(OrgId(settle.operator)),
                EconomicCause::Internal {
                    kind: SmolStr::new_static(CREDIT_NAMESPACE),
                    id: u64::from(hull.0),
                },
                vec![
                    RequestedLine {
                        account: AccountId(settle.operator_account),
                        delta: debit,
                        memo: SmolStr::new_static(CREDIT_NAMESPACE),
                    },
                    RequestedLine {
                        account: AccountId(settle.holder_account),
                        delta: Amount::from_minor(credit),
                        memo: SmolStr::new_static(CREDIT_NAMESPACE),
                    },
                ],
            )
        };
        Some(mush_economy::post(ctx, &plan))
    });
    match posted {
        Some(Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_))) => {}
        other => {
            tracing::error!(
                hull = hull.0, act, credit = settle.credit_minor, outcome = ?other,
                "tacsim: a scrap's credit did not post; the matter is in the operator's stores; \
                 staff pay the holder by hand"
            );
            ctx.out.sys_to_actor(
                by,
                "tacsim",
                format!(
                    "The yard took #{}'s matter but could not pay for it; staff are paged.",
                    hull.0
                ),
            );
        }
    }
}

/// **A hull's surviving structural points** (§4.6): every facet's armour and every section cell
/// above its minimum, never the heat pair.
#[must_use]
pub fn surviving_points(unit: &mush_tactics::TacticalUnit) -> u64 {
    use mush_tactics::unit::CellAddr;
    unit.cells()
        .filter(|(addr, _)| match addr {
            CellAddr::Section { cell, .. } => {
                cell.as_str() != crate::definitions::unit::HEAT_CELL
                    && cell.as_str() != crate::definitions::unit::HEAT_SINK_CELL
            }
            CellAddr::Facet { .. } => true,
            _ => false,
        })
        .fold(0u64, |sum, (_, cell)| {
            sum.saturating_add(u64::try_from(cell.current.saturating_sub(cell.min)).unwrap_or(0))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: References = References {
        hull_minor: 7_549_500,
        alloys_minor_t: 45_000,
        fittings_minor_t: 52_000,
    };

    fn escort_parts(intact: bool) -> Vec<PartIn> {
        [2_800_000, 250_000, 900_000, 2_200_000]
            .into_iter()
            .map(|reference_minor| PartIn {
                reference_minor,
                intact,
            })
            .collect()
    }

    /// **Doc 39 §4.6's escort** (cut IN.8): a whole intact escort at Stickney — 692 cr for the
    /// structure (under the 2,799 fifth), 44,525 for the four parts, about 45,200 in all; 1 t
    /// into the pool and 539 kg to the shop bale.
    #[test]
    fn in8_a_whole_escort_scraps_for_its_parts_and_a_tonne() {
        let t = terms(
            AT,
            1_539,
            &escort_parts(true),
            40_000,
            None,
            Bale::default(),
        );
        assert_eq!(t.structural_minor, 69_255);
        assert_eq!(t.parts_minor, 4_452_500);
        assert_eq!(t.credit_minor(), 4_521_755);
        assert_eq!((t.alloys_t, t.fittings_t), (1, 0));
        assert_eq!(t.shop_after.alloys_kg, 539);
        // Worn parts come back at 0.40.
        let worn = terms(
            AT,
            1_539,
            &escort_parts(false),
            40_000,
            None,
            Bale::default(),
        );
        assert_eq!(worn.parts_minor, 2_460_000 - 160_000);
    }

    /// **The Dray** (§4.6): 90,000 with a 30,000 plant; the fifth (12,000) caps the 94 t;
    /// the plant used is 22,100 after its removal; 34,100 in all and 94 t of alloys. With the
    /// plant destroyed the fifth of 90,000 is 18,000.
    #[test]
    fn in8_a_dray_is_capped_by_the_fifth() {
        let at = References {
            hull_minor: 9_000_000,
            ..AT
        };
        let plant = [PartIn {
            reference_minor: 3_000_000,
            intact: true,
        }];
        let t = terms(at, 94_500, &plant, 40_000, None, Bale::default());
        assert_eq!(t.structural_minor, 1_200_000);
        assert_eq!(t.credit_minor(), 3_410_000);
        assert_eq!(t.alloys_t, 94);
        let wrecked = terms(at, 94_500, &[], 40_000, None, Bale::default());
        assert_eq!(wrecked.credit_minor(), 1_800_000);
    }

    /// **A return** (§4.10): 914 kg of alloys back at 411 cr into the shop bale; a second
    /// return takes it past a tonne, which mints.
    #[test]
    fn in8_a_returned_bale_is_bought_at_the_reference_and_mints_at_a_tonne() {
        let none = References {
            hull_minor: 0,
            ..AT
        };
        let bale = Bale {
            alloys_kg: 914,
            fittings_kg: 0,
        };
        let first = terms(none, 0, &[], 0, Some(bale), Bale::default());
        assert_eq!(first.credit_minor(), 41_130);
        assert_eq!((first.alloys_t, first.shop_after.alloys_kg), (0, 914));
        let second = terms(none, 0, &[], 0, Some(bale), first.shop_after);
        assert_eq!((second.alloys_t, second.shop_after.alloys_kg), (1, 828));
    }
}
