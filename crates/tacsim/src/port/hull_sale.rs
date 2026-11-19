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
/// In a workshop's bay: `{name}`.
pub const HULL_IN_BAY: &str = "The {name} is in a workshop's bay; stow her before she is listed.";
/// A leased rig in her bay: `{n}`, `{name}`.
pub const HULL_RIG_ABOARD: &str =
    "Rig {n} is still in the {name}'s bay; return it to Occator before she is listed.";

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
        let Some(hold) = crate::port::ship::hold_vault(w, &view) else {
            return Some(Err(line(HULL_HOLD_DARK)));
        };
        // `in_flight_on` asks after the pilot's own open sales; a seller that is no person has
        // none, and the hull itself never sells, so it stands in.
        let pilot = match seller {
            PrincipalId::Object(o) => *o,
            _ => object,
        };
        if let Some((_, refusal)) = crate::port::buyout::in_flight_on(w, object, pilot, hold, &name)
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
            HULL_IN_BAY,
            HULL_RIG_ABOARD,
        ] {
            assert!(line.chars().count() < 120, "{line}");
        }
    }
}
