//! **The owned-hull register and the provenance** (doc 39 cut IN.9, §4.13; Q-39-33, the
//! expensive-to-reverse marker): what a yard commission leaves behind, and the one place every
//! reader of a built hull asks.
//!
//! - **The register** — `tacsim:owned_hull:{hull}` ([`OwnedHull`]): one row a commissioned hull —
//!   the hull, the design, the mark, the home port, the provenance id, and the lay-up row (a
//!   ship's [`HullRow`] or a rig's [`RigRow`]) the commission made it through. A built hull is
//!   **never a programme row**: the applier's preflight and `hull_line` never read this, so the
//!   pool stays the seed. The readers — `hull_view`, `hull_of_room`, `helm_of`, the carrier
//!   scans (`bay_carriers`, `world_hulls`), the rig lessee reader (`rig_lease`'s title view) —
//!   ask it **after** the programme and yard rows.
//! - **The provenance** — `tacsim:hull_provenance:{order}` ([`HullProvenance`]): the order, the
//!   facility, the design's digest, the kit lot, the part lots fitted and the instant; the record
//!   every later claim, resale and scrap reads (Q-38-12's reason).
//! - **Who reaches the helm** (§4.13): a hull titled to the pilot, or to an organization the pilot
//!   `acts_for` (`mush_economy::acts_for`), lowest id first ([`helm_hulls`]). A delegate without
//!   office waits on doc 27 Q-A1; until then an organization's hulls are its officers'.
//!
//! Each document is written whole through [`put_owned`] / [`put_provenance`], the works
//! contract's shape; an unreadable one is held (its hull reads as no register row) until staff
//! repair it.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ModuleWrite, ObjId, World};
use mush_economy::EconomyStore;
use mush_inventory::LotId;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::TacsimStore;
use crate::port::map::{HullRow, RigRow};

/// The register's document prefix: `tacsim:owned_hull:{hull}`.
pub const OWNED_PREFIX: &str = "tacsim:owned_hull:";
/// The provenance's document prefix: `tacsim:hull_provenance:{order}`.
pub const PROVENANCE_PREFIX: &str = "tacsim:hull_provenance:";
/// The most part lots one provenance names (a design's slots).
pub const MAX_PROVENANCE_PARTS: usize = 16;

/// **What a register row lays up** — a ship (its hull row, the applier's shape) or a rig (its
/// rig row).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedBody {
    /// A ship: docked at the yard's port, its rooms, exits and bays.
    Ship(HullRow),
    /// A rig: a terrain vehicle, its cab and store.
    Rig(RigRow),
}

impl OwnedBody {
    /// The Thing.
    #[must_use]
    pub const fn object(&self) -> ObjId {
        match self {
            Self::Ship(row) => ObjId(row.hull),
            Self::Rig(row) => ObjId(row.tv),
        }
    }

    /// The registry mark.
    #[must_use]
    pub fn mark(&self) -> &str {
        match self {
            Self::Ship(row) => &row.mark,
            Self::Rig(row) => &row.mark,
        }
    }
}

/// **One register row** (`tacsim:owned_hull:{hull}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedHull {
    /// The hull or rig Thing: the key.
    pub hull: ObjId,
    /// The design (a unit definition id).
    pub design: CanonicalId,
    /// The registry mark, fresh at the commission.
    pub mark: String,
    /// The port entry it was built at.
    pub home: CanonicalId,
    /// The provenance record's id: the order's.
    pub provenance: u64,
    /// The lay-up row.
    pub body: OwnedBody,
}

impl OwnedHull {
    /// Bounds: the row names the hull the key names, under the mark it carries.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.body.object() != self.hull || self.body.mark() != self.mark || self.mark.is_empty()
        {
            return Err("a register row names its own hull under its own mark".into());
        }
        Ok(())
    }

    /// The document key.
    #[must_use]
    pub fn key(&self) -> String {
        owned_key(self.hull)
    }

    /// The ship's row, if it is a ship.
    #[must_use]
    pub const fn ship(&self) -> Option<&HullRow> {
        match &self.body {
            OwnedBody::Ship(row) => Some(row),
            OwnedBody::Rig(_) => None,
        }
    }

    /// The rig's row, if it is a rig.
    #[must_use]
    pub const fn rig(&self) -> Option<&RigRow> {
        match &self.body {
            OwnedBody::Rig(row) => Some(row),
            OwnedBody::Ship(_) => None,
        }
    }
}

/// **One provenance record** (`tacsim:hull_provenance:{order}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HullProvenance {
    /// The order: the key (its escrow vault's id, the contract's number).
    pub order: u64,
    /// The work order the kit came from.
    pub work_order: u64,
    /// The hull it made.
    pub hull: ObjId,
    /// The yard.
    pub facility: ObjId,
    /// The design.
    pub design: CanonicalId,
    /// The design's pack digest at the commission.
    pub design_digest: String,
    /// The kit lot consumed.
    pub kit: LotId,
    /// The part lots fitted.
    pub parts: Vec<LotId>,
    /// When, IC seconds.
    pub at_ic: i64,
}

impl HullProvenance {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.parts.len() > MAX_PROVENANCE_PARTS || self.design_digest.is_empty() {
            return Err(format!(
                "a provenance names its design's digest and at most {MAX_PROVENANCE_PARTS} parts"
            ));
        }
        Ok(())
    }

    /// The document key.
    #[must_use]
    pub fn key(&self) -> String {
        provenance_key(self.order)
    }
}

/// A stored record: read, or unreadable (held until staff repair the document).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held<T> {
    /// The record.
    Present(Box<T>),
    /// The document would not read.
    Unreadable,
}

impl<T> Held<T> {
    /// The record, if it read.
    #[must_use]
    pub fn present(&self) -> Option<&T> {
        match self {
            Self::Present(record) => Some(record),
            Self::Unreadable => None,
        }
    }
}

/// **The yard's three record families in RAM** (doc 39 cut IN.9): the register, the provenance
/// and the commissions, each loaded at boot and written whole.
#[derive(Clone, Debug, Default)]
pub struct YardBook {
    /// The register, by hull.
    pub(crate) owned: BTreeMap<ObjId, Held<OwnedHull>>,
    /// The provenance, by order.
    pub(crate) provenance: BTreeMap<u64, Held<HullProvenance>>,
    /// The commissions, by order.
    pub(crate) commissions: BTreeMap<u64, Held<crate::port::commission::YardCommission>>,
}

/// The register key of `hull`.
#[must_use]
pub fn owned_key(hull: ObjId) -> String {
    format!("{OWNED_PREFIX}{}", hull.0)
}

/// The provenance key of `order`.
#[must_use]
pub fn provenance_key(order: u64) -> String {
    format!("{PROVENANCE_PREFIX}{order}")
}

// ---- the store ------------------------------------------------------------------------------

fn book(w: &World) -> Option<&YardBook> {
    w.module_state::<TacsimStore>().map(|store| &store.yard)
}

/// Queue one whole-document write (the works contract's shape: the serialize failure is
/// structurally unreachable, and a minimal body keeps the key claimed).
pub(crate) fn write<T: Serialize>(ctx: &mut Ctx<'_>, key: String, record: &T) {
    let value = serde_json::to_value(record).unwrap_or_else(|error| {
        tracing::error!(
            %key, %error,
            "tacsim: a yard record did not serialize; a minimal document is written so the key \
             stays claimed. Repair the document by hand."
        );
        serde_json::json!({})
    });
    crate::store::push_op(ctx.out, ModuleWrite::JsonSet {
            key: key.into(),
            value,
        });
}

/// Install a register row and queue its document.
pub(crate) fn put_owned(ctx: &mut Ctx<'_>, row: &OwnedHull) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store
            .yard
            .owned
            .insert(row.hull, Held::Present(Box::new(row.clone())));
    }
    write(ctx, row.key(), row);
}

/// Install a provenance record and queue its document.
pub(crate) fn put_provenance(ctx: &mut Ctx<'_>, record: &HullProvenance) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store
            .yard
            .provenance
            .insert(record.order, Held::Present(Box::new(record.clone())));
    }
    write(ctx, record.key(), record);
}

fn decode<T: serde::de::DeserializeOwned>(
    key: &str,
    value: &serde_json::Value,
    check: impl FnOnce(&T) -> Result<(), String>,
) -> Held<T> {
    match serde_json::from_value::<T>(value.clone())
        .map_err(|error| error.to_string())
        .and_then(|record| check(&record).map(|()| record))
    {
        Ok(record) => Held::Present(Box::new(record)),
        Err(error) => {
            tracing::error!(
                %key, %error,
                "tacsim: a yard record is unreadable; it is held until staff repair the document"
            );
            Held::Unreadable
        }
    }
}

/// **Boot: one yard document** — `true` when `key` is one of the three families (read, or held
/// unreadable), `false` for any other key. The identity rule binds each: a record must name its
/// own key.
pub(crate) fn load(store: &mut TacsimStore, key: &str, value: &serde_json::Value) -> bool {
    if let Some(rest) = key.strip_prefix(OWNED_PREFIX) {
        let Ok(hull) = rest.parse::<u32>().map(ObjId) else {
            tracing::warn!(%key, "tacsim: a register row has an unreadable key; skipped");
            return true;
        };
        let held = decode::<OwnedHull>(key, value, |row| {
            row.check()?;
            (row.hull == hull)
                .then_some(())
                .ok_or_else(|| "the record disagrees with its own key".to_owned())
        });
        store.yard.owned.insert(hull, held);
        return true;
    }
    if let Some(rest) = key.strip_prefix(PROVENANCE_PREFIX) {
        let Ok(order) = rest.parse::<u64>() else {
            tracing::warn!(%key, "tacsim: a provenance has an unreadable key; skipped");
            return true;
        };
        let held = decode::<HullProvenance>(key, value, |record| {
            record.check()?;
            (record.order == order)
                .then_some(())
                .ok_or_else(|| "the record disagrees with its own key".to_owned())
        });
        store.yard.provenance.insert(order, held);
        return true;
    }
    if let Some(rest) = key.strip_prefix(crate::port::commission::COMMISSION_PREFIX) {
        let Ok(order) = rest.parse::<u64>() else {
            tracing::warn!(%key, "tacsim: a commission has an unreadable key; skipped");
            return true;
        };
        let held = decode::<crate::port::commission::YardCommission>(key, value, |record| {
            record.check()?;
            (record.order == order)
                .then_some(())
                .ok_or_else(|| "the record disagrees with its own key".to_owned())
        });
        store.yard.commissions.insert(order, held);
        return true;
    }
    false
}

// ---- the readers ----------------------------------------------------------------------------

/// **The register row of `obj`** (a ship or a rig), if it read.
#[must_use]
pub fn owned(w: &World, obj: ObjId) -> Option<&OwnedHull> {
    book(w)?.owned.get(&obj)?.present()
}

/// **Every register row that read**, in hull order.
pub fn rows(w: &World) -> impl Iterator<Item = &OwnedHull> {
    book(w)
        .into_iter()
        .flat_map(|book| book.owned.values().filter_map(Held::present))
}

/// **Every commissioned ship**, in hull order: what the carrier scans add after the programme
/// and yard rows.
#[must_use]
pub fn ships(w: &World) -> Vec<ObjId> {
    rows(w)
        .filter(|row| row.ship().is_some())
        .map(|row| row.hull)
        .collect()
}

/// **The ship row of `hull`**, when the register holds it as a ship.
#[must_use]
pub fn ship_row(w: &World, hull: ObjId) -> Option<&HullRow> {
    owned(w, hull)?.ship()
}

/// **The rig row of `tv`**, when the register holds it as a rig.
#[must_use]
pub fn rig_row(w: &World, tv: ObjId) -> Option<&RigRow> {
    owned(w, tv)?.rig()
}

/// **The commissioned ship `room` belongs to**, its row and the room's template id.
#[must_use]
pub fn ship_of_room(w: &World, room: ObjId) -> Option<(&OwnedHull, &HullRow, CanonicalId)> {
    rows(w).find_map(|owned| {
        let row = owned.ship()?;
        row.rooms
            .iter()
            .find(|(_, obj)| **obj == room.0)
            .map(|(tid, _)| (owned, row, tid.clone()))
    })
}

/// **One provenance**, if it read.
#[must_use]
pub fn provenance(w: &World, order: u64) -> Option<&HullProvenance> {
    book(w)?.provenance.get(&order)?.present()
}

/// **Whether `pilot` holds `obj`'s title** (§4.13): its asset `Active` and titled to the pilot,
/// or to an organization the pilot `acts_for`.
#[must_use]
pub fn titled_for(w: &World, obj: ObjId, pilot: ObjId) -> bool {
    let Some(asset) = w
        .module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(obj))
    else {
        return false;
    };
    if asset.lifecycle != mush_economy::AssetLifecycle::Active {
        return false;
    }
    match &asset.title_owner {
        PrincipalId::Object(owner) => *owner == pilot,
        owner @ PrincipalId::Organization(_) => mush_economy::acts_for(w, pilot, owner),
        _ => false,
    }
}

/// **The commissioned ships `pilot` may helm** (§4.13), lowest id first.
#[must_use]
pub fn helm_hulls(w: &World, pilot: ObjId) -> Vec<ObjId> {
    ships(w)
        .into_iter()
        .filter(|hull| titled_for(w, *hull, pilot))
        .collect()
}

/// **A bay slot's mass limit**, kg, when it declares one.
#[must_use]
pub fn slot_mass_limit_kg(bay: &crate::definitions::BayDef) -> Option<u64> {
    bay.slot
        .limits
        .iter()
        .find(|(id, _)| id.as_str() == crate::definitions::logistics::MASS_KG)
        .map(|(_, limit)| *limit)
}

/// **`obj`'s catalogue mass**, kg: its design's dry mass (`dry_mass_kg_for_spec`).
#[must_use]
pub fn catalogue_mass_kg(w: &World, obj: ObjId) -> Option<u64> {
    let unit = mush_tactics::module::unit_of(w, obj)?;
    let wanted = unit.schema().key().id.as_str();
    let store = w.module_state::<TacsimStore>()?;
    store
        .definitions()
        .specs
        .iter()
        .find(|(id, _)| id.as_str() == wanted)
        .map(|(_, spec)| crate::field_ops::dry_mass_kg_for_spec(spec))
}

// ---- the title view for rigs ----------------------------------------------------------------

/// **The carrier whose bay `tv` rides**, and that bay's room, when the rig is placed in one.
fn carrier_bay_of(w: &World, tv: ObjId) -> Option<(ObjId, ObjId)> {
    let mush_sim::data::SpatialParent::Interior { space, .. } = w
        .module_state::<mush_sim::data::SpatialState>()?
        .parent_of(tv)
    else {
        return None;
    };
    let (view, _) = crate::port::ship::hull_of_room(w, space)?;
    crate::port::vehicle::bay_rooms(w, &view)
        .contains(&space)
        .then(|| (view.hull(), space))
}

/// **Who has the run of `carrier`** (doc 36a P1-2's predicate, as the rig's view reads it): her
/// helm holder, else the character her register title names.
fn runner_of(w: &World, carrier: ObjId) -> Option<ObjId> {
    if let Some(holder) = crate::port::coop::helm_holder(w, carrier) {
        return Some(holder);
    }
    ship_row(w, carrier)?;
    match &w
        .module_state::<EconomyStore>()?
        .snapshot()
        .asset(carrier)?
        .title_owner
    {
        PrincipalId::Object(owner) => Some(*owner),
        _ => None,
    }
}

/// **A titled rig's view for `pilot`** (doc 39 cut IN.9, §4.13): a commissioned rig riding the
/// bay of a carrier `pilot` has the run of, titled to `pilot` or an organization they act for —
/// shaped like a lease with no term and no deposit (`TvLease::house_view`'s shape, its hold the
/// carrier's), in the lowest bay slot no stored lease on that carrier holds. Never stored, so
/// `repossess` never ends it.
#[must_use]
pub fn title_view_for(
    w: &World,
    tv: ObjId,
    pilot: ObjId,
) -> Option<crate::port::tv::TvLease> {
    let rig = rig_row(w, tv)?;
    let (carrier, _) = carrier_bay_of(w, tv)?;
    if !titled_for(w, tv, pilot) {
        return None;
    }
    let runs = runner_of(w, carrier) == Some(pilot)
        || (ship_row(w, carrier).is_some() && titled_for(w, carrier, pilot));
    if !runs {
        return None;
    }
    let view = crate::port::ship::hull_view(w, carrier)?;
    let hold = crate::port::ship::hold_vault(w, &view).map_or(0, |vault| vault.0);
    let taken: Vec<u8> = w
        .module_state::<TacsimStore>()
        .map(|store| {
            store
                .tv_leases_live()
                .filter(|record| record.hull == carrier)
                .map(|record| record.slot)
                .collect()
        })
        .unwrap_or_default();
    let mut slot = 1u8;
    while taken.contains(&slot) {
        slot = slot.saturating_add(1);
    }
    let mut record = crate::port::tv::TvLease::house_view(tv, &rig.mark, pilot, carrier, slot);
    record.hold = hold;
    Some(record)
}

/// **A titled rig's view** for whoever has the run of its carrier ([`title_view_for`]).
#[must_use]
pub fn title_view(w: &World, tv: ObjId) -> Option<crate::port::tv::TvLease> {
    let (carrier, _) = carrier_bay_of(w, tv)?;
    title_view_for(w, tv, runner_of(w, carrier)?)
}

/// **Every titled rig's view on `carrier`** — `live_on_hull`'s register arm.
#[must_use]
pub fn title_views_on(w: &World, carrier: ObjId) -> Vec<crate::port::tv::TvLease> {
    rows(w)
        .filter_map(|row| row.rig().map(|_| row.hull))
        .filter(|tv| carrier_bay_of(w, *tv).is_some_and(|(on, _)| on == carrier))
        .filter_map(|tv| title_view(w, tv))
        .collect()
}

/// **Every titled rig `actor` may board** — the boarding search's register arm: the views for
/// `actor` of every commissioned rig riding a bay.
#[must_use]
pub fn title_views_for(w: &World, actor: ObjId) -> Vec<crate::port::tv::TvLease> {
    rows(w)
        .filter_map(|row| row.rig().map(|_| row.hull))
        .filter_map(|tv| title_view_for(w, tv, actor))
        .collect()
}
