//! **The unload's pure split** (doc 20 cut SM.8; §7.1 "the unload … `min(hopper, max_more(hold
//! plan))`, previewed per good and re-checked at prepare", §6 the hold's unreserved room, §19 item
//! 1 "a full hold never strands the departure"; the SM.8 brief's 3.4, lead rulings 2 and 3,
//! decisions R3, R10 and R18).
//!
//! What a hopper's goods do against the hold's room, in integers, with no world read: the whole
//! unload ([`whole`]) takes each good in id order — the order the preview lists them in — up to
//! the count the room holds ([`crate::port::capacity::units_in`]), the room taken as it goes
//! ([`crate::port::capacity::take_room`]), and says what stays; a part unload ([`part`]) takes a
//! named tonnage of the first good or names the count that fits, never clamped (D-20-9's rule for
//! a named count). The command's preview, the start's offer and the prepare's re-check all split
//! through these two, over the room `crate::surface::act::unload_room` (or the prepare's own
//! `room_less`) computes — so a preview, its commit and its re-check cannot disagree on the rule.
//!
//! [`marker`] derives each move's `inventory.transferred` marker id from `(tv, excursion, ordinal,
//! good)` at the act's due instant (decision R3: an applied marker records one commodity, so the
//! good joins the causal value), so a replayed finish meets `AlreadyExists` at the inventory and
//! the finish's own key is the HIT before it. No RNG; nothing here holds truth — a split is the
//! operator's own hopper's quantities.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use mush_inventory::InventoryError;
use smol_str::SmolStr;

use crate::definitions::GoodsTable;
use crate::port::capacity::{CapacityError, Compartment, Load, take_room, units_in};
use crate::surface::excursion::ExcursionId;

/// **What an unload moves and what stays** (doc 20 cut SM.8): good → tonnes, both in id order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// What moves into the hold.
    pub moves: BTreeMap<CanonicalId, u64>,
    /// What the hold's room leaves in the hopper (a whole unload's; a part unload leaves it
    /// empty — the rest of the hopper is simply not asked for).
    pub stays: BTreeMap<CanonicalId, u64>,
}

/// **The whole unload** (§7.1 "min(hopper, max_more(hold plan))"; lead ruling 3): each good of
/// `hopper` (its available quantity) in id order — the preview's order — up to
/// [`units_in`]`(room)` in every measure the hold's compartment limits, the room taken as it goes
/// ([`take_room`]); whatever does not fit stays. A room that takes nothing moves nothing and
/// every good stays.
///
/// # Errors
///
/// [`CapacityError`] for a good the table does not know, the compartment does not accept or
/// that does not weigh in a limited measure, or an overflow.
pub fn whole(
    goods: &GoodsTable,
    hold: &Compartment,
    hopper: &BTreeMap<CanonicalId, u64>,
    room: &Load,
) -> Result<Split, CapacityError> {
    let mut room = room.clone();
    let mut split = Split::default();
    for (good, available) in hopper {
        if *available == 0 {
            continue;
        }
        let fits = units_in(goods, hold, &room, good)?.min(*available);
        if fits > 0 {
            take_room(goods, hold, &mut room, good, fits)?;
            split.moves.insert(good.clone(), fits);
        }
        let left = available.saturating_sub(fits);
        if left > 0 {
            split.stays.insert(good.clone(), left);
        }
    }
    Ok(split)
}

/// **Which term of a part unload's fit bound it** (the SM.8 lead's pre-commit round, item 4):
/// the fit is `min(the first good's stock in the hopper, what the hold's room takes of it)`, and
/// the refusal names the term that bound — the stock (`UNLOAD_NO_STOCK`) or the room
/// (`UNLOAD_NO_FIT`, or `UNLOAD_NO_ROOM` at a fit of 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// The hopper holds no more of the good than the fit (a tie included: the stock is what
    /// the operator can see and change by mining, the room is not asked about first).
    Stock,
    /// The hold's room takes less of the good than the hopper holds.
    Room,
}

/// **A named tonnage past the fit** (decisions R10, R18): the tonnes that fit and which term
/// bound them ([`Bound`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Short {
    /// The tonnes of the first good that fit, possibly 0.
    pub fit: u64,
    /// The term that bound the fit.
    pub bound: Bound,
}

/// **A part unload** (`+tv/unload <t>`; lead ruling 2, decisions R10 and R18): `qty` tonnes of the
/// FIRST good of `hopper` in id order — the preview's order — when `qty` is at most `fit =
/// min(its available, units_in(room))`; else `Err(Short { fit, bound })`, never clamped, `bound`
/// naming the term that bound ([`Bound::Stock`] when the stock is at most the room's count, else
/// [`Bound::Room`]). `fit` may be 0 (the room takes none of the first good, or the hopper is
/// empty — `Bound::Stock` then): the callers answer `UNLOAD_NO_ROOM` at a fit of 0, never "can
/// unload 0 t". A `qty` of 0 is never asked for (the grammar refuses it) and reads as not fitting.
///
/// # Errors
///
/// As [`whole`].
pub fn part(
    goods: &GoodsTable,
    hold: &Compartment,
    hopper: &BTreeMap<CanonicalId, u64>,
    room: &Load,
    qty: u64,
) -> Result<Result<Split, Short>, CapacityError> {
    let Some((good, available)) = hopper.iter().find(|(_, available)| **available > 0) else {
        return Ok(Err(Short {
            fit: 0,
            bound: Bound::Stock,
        }));
    };
    let takes = units_in(goods, hold, room, good)?;
    let (fit, bound) = if *available <= takes {
        (*available, Bound::Stock)
    } else {
        (takes, Bound::Room)
    };
    if qty == 0 || qty > fit {
        return Ok(Err(Short { fit, bound }));
    }
    Ok(Ok(Split {
        moves: [(good.clone(), qty)].into_iter().collect(),
        stays: BTreeMap::new(),
    }))
}

/// **The marker of one good's move** (lead ruling 2, decision R3): `derived_marker_id(due_ic,
/// {"tacsim.unload": {"tv", "excursion" (its Display or null), "ordinal", "good"}})` — 80
/// lowercase hex, deterministic, distinct by good, ordinal and excursion; the due instant orders
/// it and separates a staff re-creation of the record that reuses an ordinal.
///
/// # Errors
///
/// [`InventoryError::Identity`] when the causal value cannot be hashed (never expected).
pub fn marker(
    tv: ObjId,
    excursion: Option<ExcursionId>,
    ordinal: u32,
    good: &CanonicalId,
    due_ic: i64,
) -> Result<SmolStr, InventoryError> {
    mush_inventory::derived_marker_id(
        due_ic,
        &serde_json::json!({
            "tacsim.unload": {
                "tv": tv.0,
                "excursion": excursion.map(|id| id.to_string()),
                "ordinal": ordinal,
                "good": good.as_str(),
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::goods::goods_table;
    use crate::definitions::logistics::{MASS_KG, VOLUME_L};

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("id")
    }

    /// The Dray-P's ore compartment, from its shipped pack.
    fn hold() -> Compartment {
        let pack = crate::pack::load_pack(crate::definitions::content::DRAY_P_JSON)
            .expect("the Dray-P pack loads");
        let logistics = match pack.entry(&cid(crate::definitions::content::DRAY_P)) {
            Some(crate::pack::PackEntry::Unit(spec)) => {
                spec.logistics.clone().expect("the Dray-P's logistics")
            }
            _ => panic!("the Dray-P unit"),
        };
        let def = logistics
            .stowage
            .get(&cid("cargo"))
            .expect("its cargo hold");
        Compartment::from_stowage(&cid("cargo"), def)
    }

    fn per(goods: &GoodsTable, good: &str, measure: &str) -> u64 {
        goods
            .good(&cid(good))
            .and_then(|row| row.per_unit(&cid(measure)))
            .expect("a figure")
    }

    /// A room of `t` tonnes of metal ore's mass and `l` litres.
    fn room(kg: u64, l: u64) -> Load {
        [(cid(MASS_KG), kg), (cid(VOLUME_L), l)]
            .into_iter()
            .collect()
    }

    fn map(rows: &[(&str, u64)]) -> BTreeMap<CanonicalId, u64> {
        rows.iter().map(|(g, q)| (cid(g), *q)).collect()
    }

    /// **The whole unload takes the goods in id order** (§7.1; lead ruling 3): with room for all,
    /// both goods move and nothing stays; with room for the first and part of the second, the
    /// second's remainder stays; with room for nothing, every good stays; carbon ore is bound by
    /// its packed volume where mass alone would take more.
    #[test]
    fn sm8_whole_takes_the_goods_in_id_order() {
        let goods = goods_table().expect("the goods table");
        let hold = hold();
        let kg = per(&goods, "metal_ore", MASS_KG);
        let hopper = map(&[("metal_ore", 9), ("ice_ore", 3)]);
        let all = whole(&goods, &hold, &hopper, &room(100 * kg, 1_000_000)).expect("splits");
        assert_eq!(all.moves, hopper);
        assert!(all.stays.is_empty());
        // `ice_ore` sorts before `metal_ore`: ice is taken first, metal is cut short.
        assert!(cid("ice_ore") < cid("metal_ore"));
        let some = whole(&goods, &hold, &hopper, &room(5 * kg, 1_000_000)).expect("splits");
        assert_eq!(some.moves, map(&[("ice_ore", 3), ("metal_ore", 2)]));
        assert_eq!(some.stays, map(&[("metal_ore", 7)]));
        let none = whole(&goods, &hold, &hopper, &room(kg - 1, 1_000_000)).expect("splits");
        assert!(none.moves.is_empty());
        assert_eq!(none.stays, hopper);
        // Carbon: the room by mass holds 10 t, by packed volume fewer.
        let carbon_l = per(&goods, "carbon_ore", VOLUME_L);
        let carbon = map(&[("carbon_ore", 10)]);
        let tight = whole(&goods, &hold, &carbon, &room(10 * kg, 8 * carbon_l)).expect("splits");
        assert_eq!(tight.moves, map(&[("carbon_ore", 8)]));
        assert_eq!(tight.stays, map(&[("carbon_ore", 2)]));
    }

    /// **A part unload names the count that fits, and what bound it** (lead ruling 2, decisions
    /// R10 and R18; the lead's pre-commit round, item 4): a tonnage within the fit moves exactly
    /// that of the first good, nothing of the second and no stays; past the hopper's stock, the
    /// stock and `Bound::Stock` (a tie with the room too); past the hold's room, the room's count
    /// and `Bound::Room`; a room that takes none of the first good, 0 and `Bound::Room`; an empty
    /// hopper, 0 and `Bound::Stock`.
    #[test]
    fn sm8_part_names_the_count_that_fits() {
        let goods = goods_table().expect("the goods table");
        let hold = hold();
        let kg = per(&goods, "metal_ore", MASS_KG);
        let hopper = map(&[("ice_ore", 4), ("metal_ore", 8)]);
        let wide = room(100 * kg, 1_000_000);
        let three = part(&goods, &hold, &hopper, &wide, 3).expect("splits");
        assert_eq!(
            three,
            Ok(Split {
                moves: map(&[("ice_ore", 3)]),
                stays: BTreeMap::new(),
            })
        );
        let short = |fit, bound| Err(Short { fit, bound });
        assert_eq!(
            part(&goods, &hold, &hopper, &wide, 5).expect("splits"),
            short(4, Bound::Stock),
            "past the hopper's 4 t, the hold wide open"
        );
        assert_eq!(
            part(&goods, &hold, &hopper, &room(4 * kg, 1_000_000), 5).expect("splits"),
            short(4, Bound::Stock),
            "a tie of stock and room names the stock"
        );
        assert_eq!(
            part(&goods, &hold, &hopper, &room(2 * kg, 1_000_000), 3).expect("splits"),
            short(2, Bound::Room),
            "past the hold's 2 t"
        );
        assert_eq!(
            part(&goods, &hold, &hopper, &room(kg - 1, 1_000_000), 1).expect("splits"),
            short(0, Bound::Room),
            "the hold takes none of it"
        );
        assert_eq!(
            part(&goods, &hold, &BTreeMap::new(), &wide, 1).expect("splits"),
            short(0, Bound::Stock),
            "an empty hopper"
        );
    }

    /// **A marker is its inputs** (decision R3): deterministic; 80 lowercase hex; different for
    /// another good, another ordinal, another excursion and another due instant.
    #[test]
    fn sm8_a_marker_is_its_inputs() {
        let excursion = Some(ExcursionId {
            tv: ObjId(900),
            launch_ic: 1_000,
        });
        let metal = cid("metal_ore");
        let base = marker(ObjId(900), excursion, 7, &metal, 5_000).expect("derives");
        assert_eq!(
            base,
            marker(ObjId(900), excursion, 7, &metal, 5_000).expect("derives")
        );
        assert_eq!(base.len(), 80);
        assert!(mush_inventory::is_marker_id(&base));
        for other in [
            marker(ObjId(900), excursion, 7, &cid("ice_ore"), 5_000),
            marker(ObjId(900), excursion, 8, &metal, 5_000),
            marker(ObjId(900), None, 7, &metal, 5_000),
            marker(ObjId(901), excursion, 7, &metal, 5_000),
            marker(ObjId(900), excursion, 7, &metal, 5_001),
        ] {
            assert_ne!(other.expect("derives"), base);
        }
    }
}
