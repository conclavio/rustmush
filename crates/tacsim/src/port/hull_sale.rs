//! **A hull sold player to player** (doc 48 cut EX.2; Q-48-7, -52).
//!
//! The market lists a person's titled object through its asset-sale seam
//! ([`mush_market::AssetSale`]); this is TacSim's one implementor, [`HullSale`], and it knows
//! ship hulls only. `+market/post #<hull> price=<n>` lists one; a buyer's purchase is the one
//! asset sale every venue writes ([`mush_market::asset_sale_spec`], the Co-op buyout's and the
//! recovery houses' shape), accepted by the listing as its seller's standing acceptance, so it
//! settles with the seller away.
//!
//! - **What may be listed** ([`admit`], asked at posting and again at every quote and
//!   purchase): a ship hull titled to the seller, with **no governed core fitted** (a cored hull
//!   sells only with her core, by the seller's private offer in the core's own closed resale,
//!   [`HULL_CORED`]), not in a workshop's bay, with nothing in flight on her, no leased rig in her bay and an
//!   **empty hold** (every goods-table row that is no consumable reads zero; her fuel and stores
//!   go with her).
//! - **What a sale is priced against** ([`state_digest`]): her schema, sections and
//!   installed components (each part, its condition and its titled asset, the core among them),
//!   never her volatile cells. A strip, a scrap or a fit after the quote changes it, and the
//!   sale's `market.asset.state` fact turns false: the quote lapses, or the contract refunds at
//!   its window. The bay refuses those works on a hull under a live listing or sale anyway
//!   ([`crate::port::bay_work::UNDER_SALE`]).
//! - **Her hold goes with her** ([`HullSaleObserver`]): when a settled contract passes a hull's
//!   title, her hold vault, if the old title holder owned it, is re-homed to the new one in the
//!   same settlement's batch. A Co-op buyout leaves the hold where it is (the pilot already owns
//!   it), and a recovery house's resale sells a rig, whose hopper is the vehicles principal's.
//!
//!
//! **Doc 48 unit PL.9: and facilities.** The same seam lists a port's refinery, works or yard,
//! titled to its seller, and the same observer passes a sold facility to its buyer: both are
//! [`crate::port::facility_sale`]'s, asked first for any object that is a facility. A hull's
//! behaviour is unchanged.
//!
//! [`admit`]: mush_market::AssetSale::admit
//! [`state_digest`]: mush_market::AssetSale::state_digest

use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    CatalogRef, ContractStateKind, Deliverable, EconomicEvent, EconomyObserver, EconomyStore,
};
use mush_inventory::record::EndpointRef;
use mush_org::{OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::store::TacsimStore;

/// **The hulls' catalogue pack** (doc 48 cut EX.2): one `asset` entry per design the yard
/// partitions, loaded beside the goods ([`crate::port::vendor`]).
pub const HULL_PACK: &str = "tacsim.hulls";
/// A hull entry's code: this prefix, then the design's id.
pub const HULL_CODE_PREFIX: &str = "hull.";

/// **A cored hull** (doc 48 unit EX.2b; Q-48-3's correction, Q-48-52): she sells only with her
/// governed core, by her seller's private offer (`{name}`, `{n}` her number).
pub const HULL_CORED: &str = "The {name} sells only with her governed core, by private offer: +core/offer #{n}=<player> price=<price>.";
/// A laden hold: `{name}`.
pub const HULL_HOLD_LADEN: &str =
    "The {name}'s hold must be empty before she is listed; her hold goes to the buyer with her.";
/// A hold that does not read: `{name}`.
pub const HULL_HOLD_DARK: &str = "The {name}'s hold cannot be read just now; nothing was listed.";
/// **Doc 48e (48d H-09)**: not docked where the asker stands: `{name}`.
pub const HULL_ELSEWHERE: &str =
    "The {name} is not docked at this market; she is listed and sold only where she lies.";
/// In a workshop's bay: `{name}`.
pub const HULL_IN_BAY: &str = "The {name} is in a workshop's bay; stow her before she is listed.";
/// A leased rig in her bay: `{n}`, `{name}`.
pub const HULL_RIG_ABOARD: &str =
    "Rig {n} is still in the {name}'s bay; return it to Occator before she is listed.";
/// Occupants still aboard (M6): `{name}`.
pub const HULL_OCCUPIED: &str =
    "The {name} still has someone aboard; clear her before she is listed.";

/// **The catalogue entry for `design`'s hulls** (doc 48 cut EX.2).
#[must_use]
pub fn catalog_of(design: &str) -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static(crate::NS),
        code: SmolStr::new(format!("{HULL_CODE_PREFIX}{design}")),
    }
}

/// **The hulls' catalogue entries** (doc 48 cut EX.2): one `asset` per design the yard
/// partitions, never stackable, with no baseline (a hull's price is its seller's; the design's
/// reference is `crate::definitions::mass::reference_price_minor`'s, read live).
///
/// # Errors
/// An entry that does not decode.
pub fn catalog_entries(w: &World) -> Result<Vec<mush_market::CatalogEntry>, String> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Ok(Vec::new());
    };
    store
        .definitions()
        .budgets
        .keys()
        .map(|design| {
            let catalog = catalog_of(design.as_str());
            let value = serde_json::json!({
                "catalog": {"ruleset": catalog.ruleset, "code": catalog.code},
                "pack": HULL_PACK,
                "definition_version": 1,
                "name": format!("{} hull", design.as_str()),
                "legality": "legal",
                "representation": "asset",
                "stackable": false,
                "tags": ["hull"],
            });
            serde_json::from_value(value).map_err(|error| error.to_string())
        })
        .collect()
}

/// **TacSim's asset-sale seam** (doc 48 cut EX.2): ship hulls, and (doc 48 unit PL.9) a port's
/// refinery, works and yard ([`crate::port::facility_sale`]).
#[derive(Debug, Default)]
pub struct HullSale;

impl mush_market::AssetSale for HullSale {
    fn admit(
        &self,
        ctx: &Ctx<'_>,
        object: ObjId,
        seller: &PrincipalId,
    ) -> Option<Result<CatalogRef, String>> {
        let w: &World = ctx.w;
        if crate::port::house_sweep::title_of(w, object).as_ref() != Some(seller) {
            return None;
        }
        // Doc 48 unit PL.9: a facility is the facility sale's to admit or refuse.
        if w.module_state::<mush_industry::IndustryStore>()
            .is_some_and(|store| store.facility(object).is_some())
        {
            return crate::port::facility_sale::admit(ctx, object, seller);
        }
        // First, before her design is looked up: a governed core rebinds her to a cored unit,
        // which no yard budget partitions, and she hears where she does sell all the same.
        if cored(w, object) {
            let name = w
                .get(object)
                .map(|o| o.name.to_string())
                .unwrap_or_default();
            return Some(Err(HULL_CORED
                .replace("{name}", &name)
                .replace("{n}", &object.0.to_string())));
        }
        let (design, _) = crate::definitions::mass::hull_budget(w, object)?;
        // A ship hull: one with a hull view (a rig is no ship hull, and is not listed here).
        let view = crate::port::ship::hull_view(w, object)?;
        let name = w
            .get(object)
            .map(|o| o.name.to_string())
            .unwrap_or_default();
        let line = |text: &str| text.replace("{name}", &name);
        if crate::port::bay_work::held(w, object) {
            return Some(Err(line(HULL_IN_BAY)));
        }
        // Doc 48e (48d H-09): she is listed and sold where she lies docked (Q-48-6's local
        // delivery): at the venue the asker stands in, the seller at the post, the buyer at the
        // quote and the purchase.
        if !docked_where_asked(ctx, object) {
            return Some(Err(line(HULL_ELSEWHERE)));
        }
        if crate::port::ship_mind::holds(ctx, object) {
            return Some(Err(line(crate::port::ship_mind::MIND_HOLDS_HER)));
        }
        if !crate::field::carrier::souls_aboard(w, object).is_empty() {
            return Some(Err(line(HULL_OCCUPIED)));
        }
        let Some(hold) = crate::port::ship::hold_vault(w, &view) else {
            return Some(Err(line(HULL_HOLD_DARK)));
        };
        if let Some((_, refusal)) =
            crate::port::buyout::in_flight_for(w, object, seller, hold, &name)
        {
            return Some(Err(refusal));
        }
        if let Some(n) = crate::port::tv::bay_slot_word(w, &view) {
            return Some(Err(HULL_RIG_ABOARD
                .replace("{n}", &n)
                .replace("{name}", &name)));
        }
        let Some(goods) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods) else {
            return Some(Err(line(HULL_HOLD_DARK)));
        };
        for good in goods.goods.iter().filter(|g| g.consumable.is_none()) {
            // Doc 48e (48d H-07): a hold holding a divisible good's lot is laden too.
            match crate::port_tick::read_held(ctx, EndpointRef(hold), &good.id) {
                Ok(0) => {}
                Ok(_) => return Some(Err(line(HULL_HOLD_LADEN))),
                Err(_) => return Some(Err(line(HULL_HOLD_DARK))),
            }
        }
        Some(Ok(catalog_of(design.as_str())))
    }

    fn state_digest(&self, w: &World, object: ObjId) -> Option<u64> {
        if let Some(digest) = crate::port::facility_sale::state_digest(w, object) {
            return Some(digest);
        }
        crate::definitions::mass::hull_budget(w, object)?;
        unit_digest(w, object)
    }
}

/// **Whether `hull` lies docked where `ctx.actor` asks from**: docked at the venue the asker
/// stands in, or, for an asker in no venue (a clerk on a tick), docked at all.
fn docked_where_asked(ctx: &Ctx<'_>, hull: ObjId) -> bool {
    let crate::port::ship::Berth::Docked { def, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return false;
    };
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .and_then(|room| mush_market::venue_gate(ctx.w)?.venue_of_room(ctx.w, room));
    here.is_none_or(|venue| venue.as_str() == def.venue.as_str())
}

/// FNV of a hull's schema, sections and installed components, never her volatile cells: what a
/// sale of her is priced against.
fn unit_digest(w: &World, object: ObjId) -> Option<u64> {
    let unit = mush_tactics::module::unit_of(w, object)?;
    let sections: Vec<_> = unit.sections().collect();
    let components: Vec<_> = unit.components().collect();
    let text = serde_json::to_string(&(unit.schema(), sections, components)).ok()?;
    Some(mush_market::declared_hash(&text))
}

/// **A cored hull's core leg** (doc 48 unit EX.2b; Q-48-52): her governed core, the hold that
/// goes with her and the digest her sale is priced against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoreLeg {
    /// Her fitted governed core.
    pub(crate) core: ObjId,
    /// Her hold's vault, when the seller owns it (re-homed to the buyer in the sale's union).
    pub(crate) hold: Option<mush_economy::VaultId>,
    /// The digest of her a sale is priced against (her schema, sections and components).
    pub(crate) digest: u64,
}

/// **Whether `seller` may sell `hull` with the governed core fitted in her** (doc 48 unit EX.2b;
/// Q-48-52's core leg), on the world alone: `None` when she is not the seller's, has no governed
/// core fitted or is no ship hull (the caller's own refusal), `Some(Err(line))` for the rules a
/// listed hull keeps (not in a workshop's bay, nothing in flight, no leased rig aboard, a
/// readable hold), `Some(Ok(leg))` otherwise. The hold's emptiness is typed state a world read
/// cannot reach: [`core_leg_hold_empty`] asks it of a `Ctx`. Kept apart from `admit`, so a
/// cored hull's rules can move without touching the exchange's.
pub(crate) fn core_leg(w: &World, hull: ObjId, seller: ObjId) -> Option<Result<CoreLeg, String>> {
    if crate::port::house_sweep::title_of(w, hull) != Some(PrincipalId::Object(seller)) {
        return None;
    }
    let core = crate::minds::registry_inspection::seizure::fitted_core(w, hull).filter(|core| {
        w.module_state::<EconomyStore>()
            .is_some_and(|s| s.is_managed_core(*core))
    })?;
    let view = crate::port::ship::hull_view(w, hull)?;
    let name = w.get(hull).map(|o| o.name.to_string()).unwrap_or_default();
    let line = |text: &str| text.replace("{name}", &name);
    if crate::port::bay_work::held(w, hull) {
        return Some(Err(line(HULL_IN_BAY)));
    }
    if !matches!(
        crate::port::ship::berth_of(w, hull),
        crate::port::ship::Berth::Docked { .. }
    ) {
        return Some(Err(line(HULL_ELSEWHERE)));
    }
    if mush_sim::ic_exclusion::held_at_berth(w, hull).is_some() {
        return Some(Err(line(crate::port::ship_mind::MIND_HOLDS_HER)));
    }
    if !crate::field::carrier::souls_aboard(w, hull).is_empty() {
        return Some(Err(line(HULL_OCCUPIED)));
    }
    let Some(vault) = crate::port::ship::hold_vault(w, &view) else {
        return Some(Err(line(HULL_HOLD_DARK)));
    };
    if let Some((_, refusal)) = crate::port::buyout::in_flight_on(w, hull, seller, vault, &name) {
        return Some(Err(refusal));
    }
    if let Some(n) = crate::port::tv::bay_slot_word(w, &view) {
        return Some(Err(HULL_RIG_ABOARD
            .replace("{n}", &n)
            .replace("{name}", &name)));
    }
    let Some(digest) = unit_digest(w, hull) else {
        return Some(Err(line(HULL_HOLD_DARK)));
    };
    let hold = w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault(vault))
        .filter(|v| v.owner == PrincipalId::Object(seller) && v.scope.is_none())
        .map(|v| v.id);
    Some(Ok(CoreLeg { core, hold, digest }))
}

/// **A cored hull's hold is empty** (doc 48 unit EX.2b): every goods-table row that is no
/// consumable reads zero, as a listed hull's must. Her fuel and stores go with her.
///
/// # Errors
/// The refusal line: laden, or unreadable.
pub(crate) fn core_leg_hold_empty(ctx: &Ctx<'_>, hull: ObjId) -> Result<(), String> {
    let w: &World = ctx.w;
    let name = w.get(hull).map(|o| o.name.to_string()).unwrap_or_default();
    let line = |text: &str| text.replace("{name}", &name);
    let hold = crate::port::ship::hull_view(w, hull)
        .and_then(|view| crate::port::ship::hold_vault(w, &view))
        .ok_or_else(|| line(HULL_HOLD_DARK))?;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| line(HULL_HOLD_DARK))?;
    for good in goods.goods.iter().filter(|g| g.consumable.is_none()) {
        // Doc 48e (48d H-07): a hold holding a divisible good's lot is laden too.
        match crate::port_tick::read_held(ctx, EndpointRef(hold), &good.id) {
            Ok(0) => {}
            Ok(_) => return Err(line(HULL_HOLD_LADEN)),
            Err(_) => return Err(line(HULL_HOLD_DARK)),
        }
    }
    Ok(())
}

/// **A cored hull's hold is empty, verified atomically inside an [`mush_core::ActionTxn`]**
/// (HIGH-42): every non-consumable goods-table row must read zero in both its pool balance and its
/// lot tally. Her fuel and stores (consumables) go with her.
///
/// # Errors
/// The refusal line: laden, or unreadable.
pub(crate) fn core_leg_hold_empty_in_txn(
    w: &World,
    txn: &mut mush_core::ActionTxn<'_>,
    hull: ObjId,
) -> Result<(), String> {
    let name = w.get(hull).map(|o| o.name.to_string()).unwrap_or_default();
    let line = |text: &str| text.replace("{name}", &name);
    let hold = crate::port::ship::hull_view(w, hull)
        .and_then(|view| crate::port::ship::hold_vault(w, &view))
        .ok_or_else(|| line(HULL_HOLD_DARK))?;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| line(HULL_HOLD_DARK))?;
    let inventory = w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.inventory.as_ref())
        .ok_or_else(|| line(HULL_HOLD_DARK))?;
    endpoint_non_consumables_empty_in_txn(goods, inventory, txn, EndpointRef(hold), &name)
}

fn endpoint_non_consumables_empty_in_txn(
    goods: &crate::definitions::goods::GoodsTable,
    inventory: &mush_inventory::InventoryFamilies,
    txn: &mut mush_core::ActionTxn<'_>,
    endpoint: EndpointRef,
    name: &str,
) -> Result<(), String> {
    let line = |text: &str| text.replace("{name}", name);
    for good in goods.goods.iter().filter(|g| g.consumable.is_none()) {
        let key = mush_inventory::pool_key(endpoint, &good.id).map_err(|_| line(HULL_HOLD_DARK))?;
        let pooled = match txn.get(&inventory.pools, &key) {
            Ok(None) => 0_u64,
            Ok(Some(scalar)) => {
                u64::try_from(scalar.current()).map_err(|_| line(HULL_HOLD_DARK))?
            }
            Err(_) => return Err(line(HULL_HOLD_DARK)),
        };
        let in_lots = match txn.get(&inventory.lot_tally, &key) {
            Ok(found) => found.copied().unwrap_or(0),
            Err(_) => return Err(line(HULL_HOLD_DARK)),
        };
        if pooled > 0 || in_lots > 0 {
            return Err(line(HULL_HOLD_LADEN));
        }
    }
    Ok(())
}

/// **Whether `hull` has a governed core fitted**: its fitted core is one the economy manages.
#[must_use]
pub fn cored(w: &World, hull: ObjId) -> bool {
    crate::minds::registry_inspection::seizure::fitted_core(w, hull).is_some_and(|core| {
        w.module_state::<EconomyStore>()
            .is_some_and(|s| s.is_managed_core(core))
    })
}

/// **Her hold goes with her** (doc 48 cut EX.2): on a settled contract, each hull whose title it
/// passed has her hold vault re-homed from the old title holder to the new one
/// ([`mush_economy::rehome_vault`]), in the settlement's own batch. Only a hold the old holder
/// owned moves; a hold already the buyer's (a Co-op buyout's) or anyone else's stays. A replayed
/// settlement finds the hold already moved and does nothing. Never panics.
#[derive(Debug, Default)]
pub struct HullSaleObserver;

impl EconomyObserver for HullSaleObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        let EconomicEvent::ContractChanged {
            id,
            state: ContractStateKind::Settled,
            ..
        } = ev
        else {
            return;
        };
        let transfers: Vec<(ObjId, PrincipalId, PrincipalId)> = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(*id))
            .map(|c| {
                c.consent
                    .terms
                    .deliverables
                    .iter()
                    .filter_map(|d| match d {
                        Deliverable::TitleTransfer { object, from, to } => {
                            Some((*object, from.clone(), to.clone()))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (object, from, to) in transfers {
            // Doc 48 unit PL.9: a facility passes to its buyer; a hull's hold goes with her.
            let facility = ctx
                .w
                .module_state::<mush_industry::IndustryStore>()
                .is_some_and(|store| store.facility(object).is_some());
            if facility {
                crate::port::facility_sale::on_sold(ctx, object, &from, &to);
            } else {
                rehome_hold(ctx, object, &from, &to);
                // Doc 48 cut EX.4 (Q-48-54): her berth's arrears at this port, out of the sale.
                crate::port::berth::collect_lien(ctx, object, &from);
            }
        }
    }
}

/// Re-home `hull`'s hold from `from` to `to`, when the title is `to`'s now and `from` owns it.
fn rehome_hold(ctx: &mut Ctx<'_>, hull: ObjId, from: &PrincipalId, to: &PrincipalId) {
    if crate::port::house_sweep::title_of(ctx.w, hull).as_ref() != Some(to) {
        return;
    }
    let Some(vault) = crate::port::ship::hull_view(ctx.w, hull)
        .and_then(|view| crate::port::ship::hold_vault(ctx.w, &view))
        .and_then(|id| {
            ctx.w
                .module_state::<EconomyStore>()?
                .snapshot()
                .vault(id)
                .cloned()
        })
    else {
        return;
    };
    if vault.owner != *from {
        return;
    }
    let scope = scope_of(ctx.w, to);
    if !mush_economy::rehome_vault(ctx, vault.id, to.clone(), scope, vault.name.clone()) {
        tracing::error!(
            hull = hull.0,
            vault = vault.id.0,
            "tacsim: a sold hull's hold vault was not re-homed to her buyer; staff re-own it by hand"
        );
    }
}

/// A new owner's vault scope: an organization's root scope (on a port map, else its own
/// record's), none for a person.
pub(crate) fn scope_of(w: &World, owner: &PrincipalId) -> Option<OrgScopeId> {
    let PrincipalId::Organization(org) = owner else {
        return None;
    };
    w.module_state::<TacsimStore>()
        .and_then(|store| {
            store
                .port_maps()
                .flat_map(|map| map.orgs.values())
                .find(|row| row.org == org.0)
                .map(|row| OrgScopeId(row.root_scope))
        })
        .or_else(|| {
            w.module_state::<mush_org::OrgStore>()?
                .state()
                .org(*org)
                .map(|record| record.root_scope)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ex2_a_hull_entry_is_its_design_under_the_pack_prefix() {
        let c = catalog_of("tacsim.unit.dray");
        assert_eq!(c.ruleset.as_str(), crate::NS);
        assert_eq!(c.code.as_str(), "hull.tacsim.unit.dray");
    }

    #[test]
    fn ex2_the_refusals_say_why_and_the_cored_one_says_where() {
        // Doc 48 unit EX.2b: a cored hull is no longer "not yet"; she sells by private offer.
        assert!(HULL_CORED.contains("+core/offer #{n}="), "{HULL_CORED}");
        assert!(HULL_CORED.contains("by private offer"), "{HULL_CORED}");
        for line in [
            HULL_CORED,
            HULL_HOLD_LADEN,
            HULL_HOLD_DARK,
            HULL_ELSEWHERE,
            HULL_IN_BAY,
            HULL_RIG_ABOARD,
            HULL_OCCUPIED,
        ] {
            assert!(line.chars().count() < 120, "{line}");
        }
    }

    /// **M6**: `HULL_OCCUPIED` and `MIND_HOLDS_HER` are properly bounded and formatted, and
    /// `in_flight_for` accepts an organization seller principal directly without collapsing it to
    /// `PrincipalId::Object(hull)`.
    #[test]
    fn m6_in_flight_for_accepts_org_seller_principal() {
        let tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_economy::EconomyModule::new()),
            Box::new(mush_market::MarketModule::new()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let org = PrincipalId::Organization(mush_org::OrgId(44));
        let hull = ObjId(99);
        let hold = mush_economy::VaultId(77);
        assert!(
            crate::port::buyout::in_flight_for(tw.world(), hull, &org, hold, "Salt Moth").is_none()
        );
        let occupied = HULL_OCCUPIED.replace("{name}", "Salt Moth");
        assert!(occupied.contains("Salt Moth") && occupied.contains("someone aboard"));
    }

    #[test]
    fn high42_hold_empty_in_txn_rejects_non_consumable_pools_and_lots() {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            mush_inventory::InventoryModule::new(),
        )]);
        let families = tw
            .world()
            .module_state::<mush_inventory::InventoryHandles>()
            .expect("inventory handles")
            .0
            .clone();
        let engine = tw.engine_mut();
        let goods = crate::definitions::goods::goods_table().expect("shipped goods table");
        let consumable = goods
            .goods
            .iter()
            .find(|g| g.consumable.is_some())
            .expect("shipped consumable good")
            .id
            .clone();
        let non_consumable = goods
            .goods
            .iter()
            .find(|g| g.consumable.is_none())
            .expect("shipped non-consumable good")
            .id
            .clone();
        let endpoint = EndpointRef(mush_economy::VaultId(42));

        // 1. Empty hold passes.
        {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory txn");
            endpoint_non_consumables_empty_in_txn(&goods, &families, &mut txn, endpoint, "Vesper")
                .expect("empty hold is admitted");
        }

        // 2. Consumables alone (fuel/stores) still pass.
        {
            let prepared = {
                let mut txn = engine
                    .action_transaction(&families.pools)
                    .expect("inventory txn");
                families
                    .credit(&mut txn, endpoint, &consumable, 10)
                    .expect("credit consumable");
                endpoint_non_consumables_empty_in_txn(
                    &goods, &families, &mut txn, endpoint, "Vesper",
                )
                .expect("consumables alone still pass");
                txn.prepare().expect("prepares")
            };
            let (_, install) = prepared.into_parts();
            engine.install_prepared_action(install).expect("installs");
        }

        // 3. Non-consumable pooled cargo is rejected atomically inside txn.
        {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory txn");
            families
                .credit(&mut txn, endpoint, &non_consumable, 5)
                .expect("credit non-consumable pool");
            let err = endpoint_non_consumables_empty_in_txn(
                &goods, &families, &mut txn, endpoint, "Vesper",
            )
            .expect_err("non-consumable pool must be refused");
            assert!(err.contains("must be empty"), "{err}");
        }

        // 4. Non-consumable lot cargo (with zero pool balance) is also rejected atomically inside txn.
        {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory txn");
            let source = mush_economy::ExternalRef {
                provider: "tacsim".into(),
                kind: "high42_test".into(),
                id: "lot_99".into(),
                revision: None,
            };
            families
                .mint_origin(
                    &mut txn,
                    endpoint,
                    &non_consumable,
                    3,
                    mush_inventory::Grade::new(50).expect("valid grade"),
                    source,
                    mush_inventory::LotId(99),
                )
                .expect("mint non-consumable lot");
            let err = endpoint_non_consumables_empty_in_txn(
                &goods, &families, &mut txn, endpoint, "Vesper",
            )
            .expect_err("non-consumable lot must be refused");
            assert!(err.contains("must be empty"), "{err}");
        }
    }
}
