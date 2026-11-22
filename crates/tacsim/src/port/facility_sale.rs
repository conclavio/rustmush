//! **A facility sold** (doc 48 unit PL.9; Q-48-26, -37, -45, -53).
//!
//! A port's refinery, works and yard are titled to their operators when the lane opens them
//! ([`crate::port::plant`]), so they sell as any titled object does: listed through the market's
//! asset-sale seam ([`crate::port::hull_sale::HullSale`], which asks this module of every
//! facility) and bought under the one asset sale ([`mush_market::asset_sale_spec`]).
//!
//! - **What may be sold** ([`admit`]): a port's refinery, works or yard, titled to its seller.
//!   Not the relay foundry, a programme workshop or a medical room, nor any other facility
//!   ([`FACILITY_NOT_SOLD`]). Not a facility whose stores are a vendor's storefront: the
//!   storefront trades from that vault, and the stores vault goes with the facility.
//! - **Only when idle** (Q-48-45): no open order (a works or yard contract, a work order not
//!   finished, a yard commission still laying up) and nothing in its stores.
//! - **No unpaid running cost** (Q-48-45, doc 48 unit EX.9): a facility whose room owes running
//!   costs is refused ([`FACILITY_IN_DEBT`]); its seller pays the debt first, so no buyer
//!   inherits it.
//! - **What a sale is priced against** ([`state_digest`]): the facility record, its owner,
//!   capabilities, slots, revisions, cost model and customers, and its running-cost standing
//!   (debt, mothballed). A re-term or a debt after the quote changes it, and the quote lapses or
//!   the contract refunds at its window.
//! - **What a sale moves** ([`on_sold`], from the asset sale's observer): the facility record's
//!   owner, made owner-held ([`mush_industry::store::sell_facility`]), so the lane keeps the
//!   buyer and the buyer's terms; the stores vault, re-homed to the buyer; and the payee, which
//!   follows the owner. A sold yard stays public (Q-48-53).
//! - **What the buyer sets** (Q-48-37): `@facility/terms`, its customers and margin. The cost
//!   model stays the authored one.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{CatalogRef, CustodyRef, EconomyStore, VaultId};
use mush_industry::{FacilityRecord, IndustryStore};
use mush_inventory::record::EndpointRef;
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::store::TacsimStore;

/// **The facilities' catalogue pack** (doc 48 unit PL.9): one `asset` entry a kind.
pub const FACILITY_PACK: &str = "tacsim.facilities";
/// A facility entry's code: this prefix, then its kind's word.
pub const FACILITY_CODE_PREFIX: &str = "facility.";

/// The relay foundry: `{name}`.
pub const FACILITY_FOUNDRY: &str = "The {name} is the relay foundry, which is not for sale.";
/// A programme workshop: `{name}`.
pub const FACILITY_WORKSHOP: &str = "The {name} is a program workshop, which is not for sale.";
/// A medical room: `{name}`.
pub const FACILITY_MEDICAL: &str = "The {name} is a medical room, which is not for sale.";
/// Any other facility: `{name}`.
pub const FACILITY_NOT_SOLD: &str =
    "The {name} is not for sale: a port's refinery, works or yard is sold, and nothing else.";
/// Its stores are a vendor's storefront: `{name}`.
pub const FACILITY_STOREFRONT: &str =
    "The {name}'s stores are a vendor's storefront; it is not sold while the vendor trades there.";
/// Work under way: `{name}`.
pub const FACILITY_BUSY: &str =
    "The {name} has work under way; it is sold only when idle, every order closed.";
/// Running costs owed (Q-48-45): `{name}`.
pub const FACILITY_IN_DEBT: &str =
    "The {name} owes running costs; it is sold only once they are paid.";
/// Goods in its stores: `{name}`.
pub const FACILITY_STORES_LADEN: &str =
    "The {name}'s stores hold goods; empty them before it is sold.";
/// Stores that do not read: `{name}`.
pub const FACILITY_STORES_DARK: &str =
    "The {name}'s stores cannot be read just now; nothing was listed.";

/// **A sellable facility's kind**: a port's authored refinery, works or yard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// The `refinery` block.
    Refinery,
    /// The `works` block.
    Works,
    /// The `yard` block: a sold yard stays public (Q-48-53).
    Yard,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Refinery, Self::Works, Self::Yard];

    /// The port entry's block word, and the catalogue entry's.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Refinery => "refinery",
            Self::Works => "works",
            Self::Yard => "yard",
        }
    }

    /// Whether its sale keeps it open to every customer.
    #[must_use]
    pub const fn stays_public(self) -> bool {
        matches!(self, Self::Yard)
    }
}

/// **The catalogue entry a facility of `kind` lists as** (doc 48 unit PL.9).
#[must_use]
pub fn catalog_of(kind: Kind) -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static(crate::NS),
        code: SmolStr::new(format!("{FACILITY_CODE_PREFIX}{}", kind.word())),
    }
}

/// **The facilities' catalogue entries** (doc 48 unit PL.9): one `asset` a kind, never
/// stackable, with no baseline (a facility's price is its seller's).
///
/// # Errors
/// An entry that does not decode.
pub fn catalog_entries() -> Result<Vec<mush_market::CatalogEntry>, String> {
    Kind::ALL
        .into_iter()
        .map(|kind| {
            let catalog = catalog_of(kind);
            let value = serde_json::json!({
                "catalog": {"ruleset": catalog.ruleset, "code": catalog.code},
                "pack": FACILITY_PACK,
                "definition_version": 1,
                "name": format!("a port's {}", kind.word()),
                "legality": "legal",
                "representation": "asset",
                "stackable": false,
                "tags": ["facility"],
            });
            serde_json::from_value(value).map_err(|error| error.to_string())
        })
        .collect()
}

/// **The kind of the facility `object` is**, when it is a port's authored refinery, works or
/// yard room.
#[must_use]
pub fn kind_of(w: &World, object: ObjId) -> Option<Kind> {
    crate::port::market::ports(w)
        .into_iter()
        .find_map(|(entry, def, _)| {
            Kind::ALL.into_iter().find(|kind| {
                crate::port::plant::block_room(w, &entry, def, kind.word())
                    .ok()
                    .flatten()
                    == Some(object)
            })
        })
}

/// The sentence an unsellable facility hears, by what it is.
fn not_sold(w: &World, facility: &FacilityRecord) -> &'static str {
    let holds = |word: &str| facility.capabilities().iter().any(|c| c.as_str() == word);
    if holds(crate::relay_foundry::KNOWHOW) {
        FACILITY_FOUNDRY
    } else if holds("bay_refit") || is_workshop(w, facility.object()) {
        FACILITY_WORKSHOP
    } else if holds(crate::port::def::MEDICAL_CAPABILITY) {
        FACILITY_MEDICAL
    } else {
        FACILITY_NOT_SOLD
    }
}

/// Whether `object` is a port's authored workshop room.
fn is_workshop(w: &World, object: ObjId) -> bool {
    crate::port::market::ports(w)
        .into_iter()
        .any(|(entry, def, _)| {
            crate::port::workshop::room(w, &entry, def).ok().flatten() == Some(object)
        })
}

/// **The facility's stores vault**: the vault backed by the facility object, when there is one.
#[must_use]
pub fn stores_of(w: &World, object: ObjId) -> Option<VaultId> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .vault_by_backing(&CustodyRef::Container(object))
        .map(|vault| vault.id)
}

/// Whether `vault` is a port vendor's storefront.
fn is_storefront(w: &World, vault: VaultId) -> bool {
    w.module_state::<TacsimStore>().is_some_and(|store| {
        store
            .port_maps()
            .flat_map(|map| map.vendors.values())
            .any(|row| row.vault == vault.0)
    })
}

/// **Whether work is under way at `object`** (Q-48-45): a works or yard contract still open, a
/// yard commission still laying a hull up, or a work order there not finished. The work orders
/// are read from industry's RAM index ([`IndustryStore::open_orders_at`]): the listing runs in
/// the market's context, which may not read `industry.workorder` itself.
fn busy(w: &World, object: ObjId) -> bool {
    let contracts = w.module_state::<TacsimStore>().is_some_and(|store| {
        store
            .works_contracts()
            .iter()
            .any(|c| c.facility == object && c.closed.is_none())
    });
    let commissions = crate::port::commission::commissions(w)
        .iter()
        .any(|record| {
            record.facility == object && record.step != crate::port::commission::Step::Done
        });
    let orders = w
        .module_state::<IndustryStore>()
        .is_some_and(|store| store.open_orders_at(object) > 0);
    contracts || commissions || orders
}

/// Whether `vault` holds anything: a pooled good of the goods table, or any lot. `None` when it
/// does not read.
fn laden(ctx: &Ctx<'_>, vault: VaultId) -> Option<bool> {
    let goods = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)?;
    for good in &goods.goods {
        if crate::port_tick::read_pool(ctx, EndpointRef(vault), &good.id).ok()? > 0 {
            return Some(true);
        }
    }
    let inventory = ctx
        .w
        .module_state::<mush_industry::IndustryHandles>()?
        .0
        .inventory
        .clone();
    let lots = ctx
        .action_state(&inventory.lot_census, &EndpointRef(vault))
        .ok()?
        .map_or(0, |census| census.get());
    Some(lots > 0)
}

/// **May `seller` list the facility `object`, and as which entry** (doc 48 unit PL.9): the seam's
/// facility arm. `None` for an object that is no facility, or one whose record's owner is not
/// the seller (the market already holds the title to be the seller's).
#[must_use]
pub fn admit(
    ctx: &Ctx<'_>,
    object: ObjId,
    seller: &PrincipalId,
) -> Option<Result<CatalogRef, String>> {
    let w: &World = ctx.w;
    let facility = w.module_state::<IndustryStore>()?.facility(object)?.clone();
    if facility.owner() != seller {
        return None;
    }
    let name = w
        .get(object)
        .map(|o| o.name.to_string())
        .unwrap_or_default();
    let line = |text: &str| text.replace("{name}", &name);
    let Some(kind) = kind_of(w, object) else {
        return Some(Err(line(not_sold(w, &facility))));
    };
    if busy(w, object) {
        return Some(Err(line(FACILITY_BUSY)));
    }
    // Q-48-45: no unpaid running cost. The debt is the facility's room's, and the buyer would
    // otherwise inherit it (production G1).
    if crate::port::upkeep::row_of(w, object).is_some_and(|row| row.debt_minor > 0) {
        return Some(Err(line(FACILITY_IN_DEBT)));
    }
    if let Some(vault) = stores_of(w, object) {
        if is_storefront(w, vault) {
            return Some(Err(line(FACILITY_STOREFRONT)));
        }
        if crate::port::voyage::goods_pending_aboard(w, EndpointRef(vault)).is_some()
            || crate::port::buyout::open_bid_into(w, vault)
        {
            return Some(Err(line(FACILITY_BUSY)));
        }
        match laden(ctx, vault) {
            Some(false) => {}
            Some(true) => return Some(Err(line(FACILITY_STORES_LADEN))),
            None => return Some(Err(line(FACILITY_STORES_DARK))),
        }
    }
    Some(Ok(catalog_of(kind)))
}

/// **What a facility's sale is priced against** (doc 48 unit PL.9): its record, whole; its
/// running-cost standing (its debt and whether it is mothballed), so a debt run up after the
/// quote voids the sale (production G1); and whether work is under way there, so an order
/// booked between the purchase and the settlement voids it too (production G2).
#[must_use]
pub fn state_digest(w: &World, object: ObjId) -> Option<u64> {
    let facility = w.module_state::<IndustryStore>()?.facility(object)?;
    let standing = crate::port::upkeep::row_of(w, object)
        .map(|row| (row.debt_minor, row.mothballed))
        .unwrap_or_default();
    let text = serde_json::to_string(&(facility, standing, busy(w, object))).ok()?;
    Some(mush_market::declared_hash(&text))
}

/// **A settled sale passed the facility `object`'s title from `from` to `to`** (doc 48 unit
/// PL.9, Q-48-53): the record's owner becomes the buyer, owner-held (a yard kept public), and
/// the stores vault, if the seller owned it, is re-homed to the buyer, in the settlement's own
/// batch. A replay finds both moved and does nothing. Never panics: a step that cannot be taken
/// is logged for staff.
pub fn on_sold(ctx: &mut Ctx<'_>, object: ObjId, from: &PrincipalId, to: &PrincipalId) {
    if crate::port::house_sweep::title_of(ctx.w, object).as_ref() != Some(to) {
        return;
    }
    let public = kind_of(ctx.w, object).is_some_and(Kind::stays_public);
    if let Err(error) = mush_industry::store::sell_facility(ctx, object, to.clone(), public) {
        tracing::error!(
            facility = object.0,
            %error,
            "tacsim: a sold facility's record was not passed to its buyer; staff re-own it by hand"
        );
    }
    let Some(vault) = stores_of(ctx.w, object).and_then(|id| {
        ctx.w
            .module_state::<EconomyStore>()?
            .snapshot()
            .vault(id)
            .cloned()
    }) else {
        return;
    };
    if vault.owner != *from {
        return;
    }
    let scope = crate::port::hull_sale::scope_of(ctx.w, to);
    if !mush_economy::rehome_vault(ctx, vault.id, to.clone(), scope, vault.name.clone()) {
        tracing::error!(
            facility = object.0,
            vault = vault.id.0,
            "tacsim: a sold facility's stores were not re-homed to its buyer; staff re-own them"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pl9_a_facility_entry_is_its_kind_under_the_prefix() {
        assert_eq!(catalog_of(Kind::Works).code.as_str(), "facility.works");
        assert_eq!(catalog_of(Kind::Yard).ruleset.as_str(), crate::NS);
        let entries = catalog_entries().expect("the entries decode");
        assert_eq!(entries.len(), 3);
        assert!(Kind::Yard.stays_public());
        assert!(!Kind::Works.stays_public() && !Kind::Refinery.stays_public());
    }

    #[test]
    fn pl9_the_refusals_say_why_under_the_floor() {
        for line in [
            FACILITY_FOUNDRY,
            FACILITY_WORKSHOP,
            FACILITY_MEDICAL,
            FACILITY_NOT_SOLD,
            FACILITY_STOREFRONT,
            FACILITY_BUSY,
            FACILITY_STORES_LADEN,
            FACILITY_STORES_DARK,
        ] {
            let filled = line.replace("{name}", "Stickney Yards Maintenance Bay");
            assert!(filled.chars().count() <= 120, "{filled}");
            assert!(line.contains("{name}"), "{line}");
        }
    }
}
