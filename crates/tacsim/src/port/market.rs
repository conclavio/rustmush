//! **The TacSim implementations of the market's seams** (doc 18 cut IC8; doc 18
//! §4.3 "Presence", "The capacity function", "Vendors and buyers"; D-18-12; lead
//! rulings L10–L13).
//!
//! Cut IC7 built three game-neutral seams in `mush-market` and `mush-economy`; this
//! module is the game's answer to each, wired by the composition root only where the
//! TacSim layer composes (`server_modules_with_tacsim`, lead ruling L13):
//!
//! - [`PortVenueGate`] — `mush_market::VenueGate`: which port a room belongs to,
//!   which hold a no-destination accept loads (the charter hull's, wherever it lies:
//!   L10), and the presence refusals in the gate's own words. `Presence::Ship` is
//!   wired here and has no caller until cut IC11;
//! - [`PortCapacity`] — `mush_market::CapacityProvider`: a hull hold's limits as a
//!   bounded plan ([`HoldPlan`], the compartments and the compiled-in goods table's
//!   digest, never goods rows: L12) and the check against it through the one capacity
//!   function, rendered in §4.3's sentences ([`capacity_line`]);
//! - [`PortPartyCaps`] — `mush_economy::PartyCapSource`: the port institutions
//!   (each vendor and the programme org) accept programmatically. It confers **no
//!   money authority**: the accept still posts escrow under the org account's own
//!   debit authority and still refuses when the account is short.
//!
//! **One predicate both seams answer from** (IC7 R5): [`governed_hold`]. The gate's
//! `governed` and the provider's `plan` are both `governed_hold(..).is_some()`, so
//! `plan(..).is_some() == governed(..)` for every endpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use mush_core::{CanonicalId, ObjId, World};
use mush_economy::{CustodyRef, EconomyStore, PartyCap, PartyCapSource};
use mush_inventory::record::EndpointRef;
use mush_market::{
    CapacityPlan, CapacityProvider, CapacityReadError, CapacityRefusal, MarketId, Presence,
    VenueGate, VenueRefusal,
};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};

use crate::definitions::GoodsTable;
use crate::definitions::content::LANE_PACK;
use crate::definitions::goods::{GOODS_JSON, goods_digest, goods_table};
use crate::definitions::logistics::{MASS_KG, VOLUME_L};
use crate::port::apply::room_by_key;
use crate::port::capacity::{
    self, Admission, Compartment, Load, compartment_for, format_m3, format_tonnes,
};
use crate::port::coop::{live_charter, name_of, port_name};
use crate::port::def::{PortDef, room_key};
use crate::port::map::{PortMap, PortMapState};
use crate::port::readiness::report_ports;
use crate::port::ship::{Berth, HullView, berth_of, hold_vault, hull_of_room, hull_view};
use crate::store::TacsimStore;

/// The gate's refusal when the asker is not in the venue's port (`{port}`).
pub const NOT_IN_PORT: &str =
    "That market is {port}'s, and you are not in {port}. Nothing was charged.";
/// The hull is under way (`{name}`, `{port}`).
pub const HOLD_UNDER_WAY: &str =
    "The {name} is under way, so its hold cannot load in {port}. Nothing was charged.";
/// The hull is berthed at another port (`{name}`, `{other}`, `{port}`).
pub const HOLD_ELSEWHERE: &str = "The {name} is berthed at {other}, not {port}, so its hold cannot load here. Nothing was charged.";
/// The hull is at no berth (`{name}`, `{port}`).
pub const HOLD_NOT_HERE: &str =
    "The {name} is not berthed in {port}, so its hold cannot load here. Nothing was charged.";
/// The governed hold is not the asker's charter hull's.
pub const NOT_YOUR_CHARTER: &str =
    "That hold is not on your charter, so it cannot load here. Nothing was charged.";

/// The plan's schema version.
pub const HOLD_PLAN_VERSION: u8 = 1;

// ---- the one predicate ---------------------------------------------------------

/// **The programme hull whose hold `endpoint` is** (doc 18 cut IC8; IC7 R5): the vault
/// exists, its backing is `CustodyRef::SpatialInterior(space)`, and `space` is the
/// stowage of a programme hull ([`hull_of_room`]). `None` for every other endpoint —
/// a storefront, a player's own container vault, an interior vault on any other room.
#[must_use]
pub fn governed_hold(w: &World, endpoint: EndpointRef) -> Option<HullView<'_>> {
    let vault = w
        .module_state::<EconomyStore>()?
        .snapshot()
        .vault(endpoint.vault())?;
    let CustodyRef::SpatialInterior(space) = vault.backing else {
        return None;
    };
    let (view, _) = hull_of_room(w, space)?;
    (view.stowage() == Some(space)).then_some(view)
}

// ---- port resolution -----------------------------------------------------------

/// The lane's port entries as the readiness report resolves them, each with its map
/// when present.
pub(crate) fn ports(w: &World) -> Vec<(CanonicalId, &PortDef, Option<&PortMap>)> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return Vec::new();
    };
    let (_, ports) = report_ports(store, w);
    ports
        .into_iter()
        .map(|(entry, def)| {
            let map = match store.port_map(&lane, &entry) {
                PortMapState::Present(map) => Some(map),
                PortMapState::Absent | PortMapState::Unreadable => None,
            };
            (entry, def, map)
        })
        .collect()
}

/// The port entry whose venue is `venue`.
pub(crate) fn port_of_venue<'a>(
    w: &'a World,
    venue: &MarketId,
) -> Option<(&'a PortDef, Option<&'a PortMap>)> {
    ports(w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == venue.as_str())
        .map(|(_, def, map)| (def, map))
}

/// **The port's named rooms** (lead ruling L11): the gallery, every `rooms` value
/// except the staff `spine`, and the programme office. A hull's interior is not a
/// port room: trading is done at the counters (D-18-12).
fn port_rooms(w: &World, def: &PortDef) -> BTreeSet<ObjId> {
    let mut refs: Vec<&str> = vec![def.gallery.as_str()];
    refs.extend(
        def.rooms
            .iter()
            .filter(|(role, _)| role.as_str() != "spine")
            .map(|(_, reference)| reference.as_str()),
    );
    if let Some(programme) = &def.programme {
        refs.push(programme.office.as_str());
    }
    refs.into_iter()
        .filter_map(|reference| room_key(&def.area, reference).ok())
        .filter_map(|key| room_by_key(w, &key))
        .collect()
}

/// The hull's hold vault as an endpoint.
fn hold_of(w: &World, hull: ObjId) -> Option<EndpointRef> {
    let view = hull_view(w, hull)?;
    hold_vault(w, &view).map(EndpointRef)
}

fn refusal(line: String) -> VenueRefusal {
    VenueRefusal { line }
}

/// **The hull checks** both presences make on a governed hold (lead ruling L10): the
/// hull is the actor's live charter's, and it is docked at the port whose venue this
/// is.
fn hull_admits(w: &World, actor: ObjId, def: &PortDef, hull: ObjId) -> Result<(), VenueRefusal> {
    if live_charter(w, actor).is_none_or(|charter| charter.hull != hull) {
        return Err(refusal(NOT_YOUR_CHARTER.to_owned()));
    }
    let port = port_name(def);
    let name = name_of(w, hull);
    // Doc 18 cut IC9 (lead ruling L13): a docked hull with a filed clearance is
    // treated as under way — nothing may load into a hold whose passage is filed.
    if crate::port::voyage::pending_of(w, hull).is_some() {
        return Err(refusal(
            HOLD_UNDER_WAY
                .replace("{name}", &name)
                .replace("{port}", &port),
        ));
    }
    match berth_of(w, hull) {
        Berth::Docked { def: there, .. } if there.venue == def.venue => Ok(()),
        Berth::Docked { def: there, .. } => Err(refusal(
            HOLD_ELSEWHERE
                .replace("{name}", &name)
                .replace("{other}", &port_name(there))
                .replace("{port}", &port),
        )),
        Berth::UnderWay => Err(refusal(
            HOLD_UNDER_WAY
                .replace("{name}", &name)
                .replace("{port}", &port),
        )),
        Berth::Elsewhere => Err(refusal(
            HOLD_NOT_HERE
                .replace("{name}", &name)
                .replace("{port}", &port),
        )),
    }
}

// ---- the venue gate ------------------------------------------------------------

/// **The port presence gate** (doc 18 §4.3 "Presence", D-18-12; lead rulings L10,
/// L11). Total and world-only.
#[derive(Debug)]
pub struct PortVenueGate;

impl VenueGate for PortVenueGate {
    fn venue_of_room(&self, w: &World, room: ObjId) -> Option<MarketId> {
        ports(w)
            .into_iter()
            .find(|(_, def, _)| port_rooms(w, def).contains(&room))
            .and_then(|(_, def, _)| MarketId::new(def.venue.as_str()))
    }

    fn endpoint_for(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        presence: Presence,
    ) -> Option<EndpointRef> {
        port_of_venue(w, venue)?;
        let charter = live_charter(w, actor)?;
        match presence {
            // L10: the charter hull's hold wherever the hull lies; `admit` says why it
            // may not load, in the gate's own words.
            Presence::Here => hold_of(w, charter.hull),
            Presence::Ship(hull) => (charter.hull == hull).then(|| hold_of(w, hull)).flatten(),
        }
    }

    fn admit(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        endpoint: Option<EndpointRef>,
        presence: Presence,
    ) -> Result<(), VenueRefusal> {
        let Some((def, _)) = port_of_venue(w, venue) else {
            // A venue no port authors is ungoverned.
            return Ok(());
        };
        if presence == Presence::Here {
            let here = w
                .get(actor)
                .and_then(|o| o.location)
                .and_then(|room| self.venue_of_room(w, room));
            if here.as_ref() != Some(venue) {
                return Err(refusal(NOT_IN_PORT.replace("{port}", &port_name(def))));
            }
        }
        match (endpoint, presence) {
            (Some(endpoint), _) => match governed_hold(w, endpoint) {
                Some(view) => hull_admits(w, actor, def, view.hull()),
                // An ungoverned endpoint is the market's own rules' to judge.
                None => Ok(()),
            },
            (None, Presence::Ship(hull)) => hull_admits(w, actor, def, hull),
            (None, Presence::Here) => Ok(()),
        }
    }

    fn governed(&self, w: &World, endpoint: EndpointRef) -> bool {
        governed_hold(w, endpoint).is_some()
    }

    fn port_seller(&self, w: &World, venue: &MarketId, seller: &PrincipalId) -> bool {
        let PrincipalId::Organization(org) = seller else {
            return false;
        };
        port_of_venue(w, venue)
            .and_then(|(_, map)| map)
            .is_some_and(|map| vendor_orgs(map).contains(org))
    }
}

/// The orgs a port map records as **vendors** (part e).
fn vendor_orgs(map: &PortMap) -> BTreeSet<OrgId> {
    map.vendors
        .keys()
        .filter_map(|id| map.orgs.get(id))
        .map(|row| OrgId(row.org))
        .collect()
}

// ---- the party-cap source ------------------------------------------------------

/// **The port institutions' programmatic acceptance** (doc 18 §4.1/§4.7, Q-18-10;
/// cut IC7's seam; lead ruling L13): `PartyCap::Accept` for an org a lane port map
/// records as one of its **vendors**, or as its port's **programme org** (the Trust,
/// for cuts IC10/IC13), and for nothing else. It confers **no money authority**: the
/// accept still runs `accept_core`, still posts escrow under the org account's own
/// debit authority, and still refuses when the account is short.
#[derive(Debug)]
pub struct PortPartyCaps;

impl PartyCapSource for PortPartyCaps {
    fn has_cap(&self, w: &World, principal: &PrincipalId, cap: PartyCap) -> bool {
        match cap {
            PartyCap::Accept => {}
        }
        let PrincipalId::Organization(org) = principal else {
            return false;
        };
        ports(w).into_iter().any(|(_, def, map)| {
            let Some(map) = map else {
                return false;
            };
            let programme = def
                .programme
                .as_ref()
                .and_then(|p| map.orgs.get(&p.org))
                .map(|row| OrgId(row.org));
            programme == Some(*org) || vendor_orgs(map).contains(org)
        })
    }
}

// ---- the capacity provider -----------------------------------------------------

/// **One compartment in a hold plan.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCompartment {
    /// The stowage id (`cargo`, `propellant`, `provisions`).
    pub id: CanonicalId,
    /// The stowage tags it accepts.
    pub accepts: BTreeSet<CanonicalId>,
    /// Measure → limit.
    pub limits: BTreeMap<CanonicalId, u64>,
}

/// **A hull hold's capacity plan** (doc 18 cut IC8, lead ruling L12): the hull's
/// `logistics.stowage` as compartments, and the lowercase hex digest of the
/// compiled-in goods table the plan was made against — **no goods rows**. The Draw
/// prepare checks against it with no World (IC7 L3); a plan made by one binary and
/// checked by another whose goods table differs fails closed (`Unreadable`) instead of
/// mis-measuring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldPlan {
    /// [`HOLD_PLAN_VERSION`].
    pub v: u8,
    /// `goods_digest(GOODS_JSON)`, lowercase hex.
    pub goods: String,
    /// The compartments, in stowage order.
    pub compartments: Vec<PlanCompartment>,
}

/// The compiled-in goods table and its digest, loaded once (a failure is cached as
/// `None`, and every check then reads `Unreadable`).
fn compiled_goods() -> Option<&'static (String, GoodsTable)> {
    static GOODS: OnceLock<Option<(String, GoodsTable)>> = OnceLock::new();
    GOODS
        .get_or_init(|| {
            let table = goods_table().ok()?;
            let digest = goods_digest(GOODS_JSON).ok()?;
            Some((digest.to_hex(), table))
        })
        .as_ref()
}

/// **The hold plan of `view`**, or `None` when the goods table does not load.
#[must_use]
pub fn hold_plan(view: &HullView<'_>) -> Option<HoldPlan> {
    plan_of(view.logistics)
}

/// The plan of one hull definition's logistics block.
fn plan_of(logistics: &crate::definitions::LogisticsDef) -> Option<HoldPlan> {
    let (digest, _) = compiled_goods()?;
    Some(HoldPlan {
        v: HOLD_PLAN_VERSION,
        goods: digest.clone(),
        compartments: logistics
            .stowage
            .iter()
            .map(|(id, def)| PlanCompartment {
                id: id.clone(),
                accepts: def.accepts.clone(),
                limits: def.limits.clone(),
            })
            .collect(),
    })
}

/// **The hull hold capacity provider** (doc 18 §4.3 "The capacity function"; cut
/// IC8). `plan` answers exactly where [`governed_hold`] does.
#[derive(Debug)]
pub struct PortCapacity;

impl CapacityProvider for PortCapacity {
    fn plan(&self, w: &World, endpoint: EndpointRef) -> Option<CapacityPlan> {
        let view = governed_hold(w, endpoint)?;
        // A plan that will not form is still a governed endpoint: an unreadable plan
        // refuses at the check (never an ungoverned answer for a hull's hold).
        let value = hold_plan(&view)
            .and_then(|plan| serde_json::to_value(plan).ok())
            .unwrap_or(serde_json::Value::Null);
        Some(CapacityPlan(value))
    }

    fn check(
        &self,
        plan: &CapacityPlan,
        commodity: &CanonicalId,
        quantity: u64,
        read_pool: &mut dyn FnMut(&CanonicalId) -> Result<u64, CapacityReadError>,
    ) -> Result<(), CapacityRefusal> {
        let plan: HoldPlan =
            serde_json::from_value(plan.0.clone()).map_err(|_| CapacityRefusal::Unreadable)?;
        if plan.v != HOLD_PLAN_VERSION {
            return Err(CapacityRefusal::Unreadable);
        }
        let Some((digest, goods)) = compiled_goods() else {
            return Err(CapacityRefusal::Unreadable);
        };
        if *digest != plan.goods {
            return Err(CapacityRefusal::Unreadable);
        }
        let compartments: Vec<Compartment> = plan
            .compartments
            .iter()
            .map(|c| Compartment {
                id: c.id.clone(),
                accepts: c.accepts.clone(),
                limits: c.limits.clone(),
            })
            .collect();
        let no_place = || CapacityRefusal::Refused {
            fits: 0,
            line: format!("{} has no place aboard.", label_of(goods, commodity)),
        };
        let Some(compartment) = compartment_for(goods, &compartments, commodity) else {
            return Err(no_place());
        };
        // Every good the compartment accepts, read; an unreadable pool refuses, never 0.
        let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        for good in goods
            .goods
            .iter()
            .filter(|good| compartment.accepts.contains(&good.stowage))
        {
            let held =
                read_pool(&good.id).map_err(|CapacityReadError| CapacityRefusal::Unreadable)?;
            if held > 0 {
                contents.insert(good.id.clone(), held);
            }
        }
        match capacity::admits(goods, &compartment, &contents, (commodity, quantity)) {
            Admission::Admitted(_) => Ok(()),
            Admission::Refused(refused) => {
                let mut with = contents;
                let slot = with.entry(commodity.clone()).or_insert(0);
                *slot = slot.saturating_add(quantity);
                let Ok(after) = capacity::load(goods, &compartment, &with) else {
                    return Err(no_place());
                };
                Err(CapacityRefusal::Refused {
                    fits: refused.fits,
                    line: capacity_line(goods, &compartment, commodity, quantity, &refused, &after),
                })
            }
            Admission::Error(_) => Err(no_place()),
        }
    }
}

/// A good's label, else its id.
fn label_of(goods: &GoodsTable, good: &CanonicalId) -> String {
    goods
        .good(good)
        .map_or_else(|| good.to_string(), |row| row.label.clone())
}

/// One measure's figure in a measure map, by the measure's id text.
fn figure(map: &BTreeMap<CanonicalId, u64>, measure: &str) -> Option<u64> {
    map.iter()
        .find(|(id, _)| id.as_str() == measure)
        .map(|(_, value)| *value)
}

/// A limit in whole tonnes.
fn limit_t(compartment: &Compartment) -> String {
    figure(&compartment.limits, MASS_KG).map_or_else(String::new, |kg| (kg / 1_000).to_string())
}

/// A limit in whole cubic metres.
fn limit_m3(compartment: &Compartment) -> String {
    figure(&compartment.limits, VOLUME_L).map_or_else(String::new, |l| (l / 1_000).to_string())
}

/// **The §4.3 capacity sentences** (doc 18 §4.3 "The capacity function"; cut IC8):
/// verbatim where §4.3 writes them, the rest in its voice. `{label}` is the goods row's
/// label; cubic metres are the compartment's load **after** the addition
/// ([`format_m3`], truncated: lead ruling L12 — §4.3's 86 t example is an empty hold,
/// where both readings agree); tonnes are [`format_tonnes`]. Which sentence is decided
/// by the compartment (`propellant` is the tank, `provisions` the stores locker, any
/// other the hold) and by `refusal.bound` and `refusal.fits`.
#[must_use]
pub fn capacity_line(
    goods: &GoodsTable,
    compartment: &Compartment,
    good: &CanonicalId,
    qty: u64,
    refusal: &capacity::CapacityRefusal,
    after: &Load,
) -> String {
    let label = label_of(goods, good);
    let fits = refusal.fits;
    match compartment.id.as_str() {
        "propellant" => {
            if fits == 0 {
                format!("The tank is full: no more {label} fits.")
            } else {
                format!(
                    "{qty} t of {label} weighs past the tank's {} t rating. {fits} t fits.",
                    limit_t(compartment)
                )
            }
        }
        "provisions" => {
            if fits == 0 {
                return format!("The stores locker is full: no more {label} fits.");
            }
            // The IC2 locker rule: the limits agree on the unit count, so any one
            // limit divided by its per-unit figure is the locker's capacity.
            let units = goods
                .good(good)
                .and_then(|row| {
                    compartment.limits.iter().find_map(|(m, limit)| {
                        row.per_unit(m).and_then(|per| limit.checked_div(per))
                    })
                })
                .unwrap_or(0);
            format!(
                "{qty} crew-days of {label} do not fit the stores locker's {units} crew-days. \
                 {fits} fit."
            )
        }
        _ => {
            if fits == 0 {
                let room_t = format_tonnes(figure(&refusal.room, MASS_KG).unwrap_or(0));
                let room_m3 = format_m3(figure(&refusal.room, VOLUME_L).unwrap_or(0));
                return format!(
                    "The hold has {room_t} t of room by mass and {room_m3} m³ by volume: no more \
                     {label} fits."
                );
            }
            let after_m3 = format_m3(figure(after, VOLUME_L).unwrap_or(0));
            let by_mass = refusal.bound.iter().any(|m| m.as_str() == MASS_KG);
            let by_volume = refusal.bound.iter().any(|m| m.as_str() == VOLUME_L);
            match (by_mass, by_volume) {
                (true, true) => format!(
                    "{qty} t of {label} weighs past the hold's {} t rating and packs to {after_m3} \
                     m³; the hold takes {} m³. {fits} t fits.",
                    limit_t(compartment),
                    limit_m3(compartment)
                ),
                (true, false) => format!(
                    "{qty} t of {label} weighs past the hold's {} t rating ({after_m3} m³). {fits} \
                     t fits.",
                    limit_t(compartment)
                ),
                (false, _) => format!(
                    "{qty} t of {label} packs to {after_m3} m³; the hold takes {} m³. {fits} t \
                     fits.",
                    limit_m3(compartment)
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::LogisticsDef;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("id")
    }

    fn goods() -> GoodsTable {
        goods_table().expect("the compiled-in goods table")
    }

    /// The Dray's logistics, from its shipped pack.
    fn dray() -> LogisticsDef {
        let pack = crate::pack::load_pack(crate::definitions::content::DRAY_JSON)
            .expect("the Dray pack loads");
        match pack.entry(&cid(crate::definitions::content::DRAY)) {
            Some(crate::pack::PackEntry::Unit(spec)) => {
                spec.logistics.clone().expect("the Dray's logistics")
            }
            _ => panic!("the Dray unit"),
        }
    }

    fn compartment(id: &str) -> Compartment {
        let logistics = dray();
        let def = logistics.stowage.get(&cid(id)).expect("the compartment");
        Compartment::from_stowage(&cid(id), def)
    }

    fn per(goods: &GoodsTable, good: &str, measure: &str) -> u64 {
        figure(&goods.good(&cid(good)).expect("a good").measures, measure).expect("a figure")
    }

    /// The line the capacity function's refusal renders, for `qty` of `good` on top of
    /// `contents`.
    fn line(comp: &Compartment, contents: &[(&str, u64)], good: &str, qty: u64) -> String {
        let goods = goods();
        let contents: BTreeMap<CanonicalId, u64> =
            contents.iter().map(|(g, q)| (cid(g), *q)).collect();
        let Admission::Refused(refused) =
            capacity::admits(&goods, comp, &contents, (&cid(good), qty))
        else {
            panic!("{qty} of {good} fitted");
        };
        let mut with = contents;
        *with.entry(cid(good)).or_insert(0) += qty;
        let after = capacity::load(&goods, comp, &with).expect("a load");
        capacity_line(&goods, comp, &cid(good), qty, &refused, &after)
    }

    /// **The §4.3 sentences, byte for byte** (doc 18 cut IC8): every load is computed
    /// from the goods measures and the hold's limits, never typed as a vertex.
    #[test]
    fn ic8_the_capacity_lines_read_as_section_4_3_writes_them() {
        let goods = goods();
        let cargo = compartment("cargo");
        assert_eq!(
            line(&cargo, &[], "volatiles", 86),
            "86 t of water ice packs to 120.4 m³; the hold takes 120 m³. 85 t fits."
        );
        assert_eq!(
            line(&cargo, &[], "structural_alloys", 101),
            "101 t of structural alloy stock weighs past the hold's 100 t rating (30.3 m³). \
             100 t fits."
        );
        // One tonne past the metals/ice vertex, solved here from the measures.
        let big_m = figure(&cargo.limits, MASS_KG).expect("mass limit");
        let big_v = figure(&cargo.limits, VOLUME_L).expect("volume limit");
        let (vd, vb) = (
            per(&goods, "metals", VOLUME_L),
            per(&goods, "volatiles", VOLUME_L),
        );
        let unit = per(&goods, "metals", MASS_KG);
        let tonnes = big_m / unit;
        let b = (big_v - tonnes * vd) / (vb - vd);
        let d = tonnes - b;
        assert_eq!(d * vd + b * vb, big_v, "the vertex is integral");
        assert_eq!(
            line(&cargo, &[("metals", d)], "volatiles", b + 1),
            format!(
                "{} t of water ice weighs past the hold's {} t rating and packs to {} m³; the \
                 hold takes {} m³. {b} t fits.",
                b + 1,
                big_m / 1_000,
                format_m3(d * vd + (b + 1) * vb),
                big_v / 1_000
            )
        );
        // A tonne short of the vertex's concentrate, the vertex's ice: no more ice fits.
        assert_eq!(
            line(
                &cargo,
                &[("metals", d - 1), ("volatiles", b)],
                "volatiles",
                1
            ),
            format!(
                "The hold has 1 t of room by mass and {} m³ by volume: no more water ice fits.",
                format_m3(vd)
            )
        );
        let tank = compartment("propellant");
        assert_eq!(
            line(&tank, &[], "propellant", 81),
            "81 t of reaction mass weighs past the tank's 80 t rating. 80 t fits."
        );
        assert_eq!(
            line(&tank, &[("propellant", 80)], "propellant", 1),
            "The tank is full: no more reaction mass fits."
        );
        let locker = compartment("provisions");
        assert_eq!(
            line(&locker, &[], "provisions", 121),
            "121 crew-days of voyage provisions do not fit the stores locker's 120 crew-days. \
             120 fit."
        );
        assert_eq!(
            line(&locker, &[("provisions", 120)], "provisions", 1),
            "The stores locker is full: no more voyage provisions fits."
        );
    }

    /// **The Dray's plan is small** (lead ruling L12): under the market's 2 KiB plan
    /// bound with at least 1 KiB to spare, and it round-trips strictly.
    #[test]
    fn ic8_the_drays_plan_is_bounded() {
        let plan = plan_of(&dray()).expect("a plan");
        let bytes = serde_json::to_vec(&plan).expect("encodes");
        assert!(
            bytes.len().saturating_add(1_024) <= mush_market::MAX_CAPACITY_PLAN_BYTES,
            "{} bytes",
            bytes.len()
        );
        let back: HoldPlan = serde_json::from_slice(&bytes).expect("decodes");
        assert_eq!(back, plan);
        assert!(CapacityPlan(serde_json::to_value(&plan).expect("value")).within_bounds());
    }

    /// **The check fails closed**: an unreadable pool, a plan from another goods table
    /// and a malformed plan are `Unreadable`; a good no compartment takes has no place
    /// aboard; an admitted load is `Ok`.
    #[test]
    fn ic8_the_check_fails_closed() {
        let plan = plan_of(&dray()).expect("a plan");
        let value = serde_json::to_value(&plan).expect("value");
        let provider = PortCapacity;
        let mut empty = |_: &CanonicalId| Ok(0);
        assert_eq!(
            provider.check(&CapacityPlan(value.clone()), &cid("metals"), 10, &mut empty),
            Ok(())
        );
        let mut broken = |_: &CanonicalId| Err(CapacityReadError);
        assert_eq!(
            provider.check(
                &CapacityPlan(value.clone()),
                &cid("metals"),
                10,
                &mut broken
            ),
            Err(CapacityRefusal::Unreadable)
        );
        let mut other = plan.clone();
        other.goods = "00".repeat(32);
        assert_eq!(
            provider.check(
                &CapacityPlan(serde_json::to_value(&other).expect("value")),
                &cid("metals"),
                10,
                &mut empty
            ),
            Err(CapacityRefusal::Unreadable)
        );
        assert_eq!(
            provider.check(
                &CapacityPlan(serde_json::json!({ "v": 1, "goods": "x" })),
                &cid("metals"),
                10,
                &mut empty
            ),
            Err(CapacityRefusal::Unreadable)
        );
        assert_eq!(
            provider.check(&CapacityPlan(value), &cid("nonesuch"), 1, &mut empty),
            Err(CapacityRefusal::Refused {
                fits: 0,
                line: "nonesuch has no place aboard.".to_owned()
            })
        );
    }
}
