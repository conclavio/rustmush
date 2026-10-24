//! **Occator's rig pool** (doc 20 cut SM.3a; doc 20 §4.1, §4.2, §4.3, Q-20-6;
//! lead rulings 5, 7, 8 and 9 of the SM.3a brief).
//!
//! For a port entry that authors `vehicles` (lane v10: Ceres, Occator Water &
//! Propellant's eight terrain vehicles), the `PortApplier` runs **part (h)**
//! after the hulls of part (f) (`lay_up`), every step idempotent and the map
//! written after every object, in pool order:
//!
//! - **(h0) the shed** — once, a room named `vehicles.shed` with no exits
//!   (nobody reaches it; staff may `@tel`), declared an **unanchored** interior
//!   space (decision D5), so a laid-up rig is `Interior`-placed somewhere and
//!   never "nowhere", and lease and return are two re-placements;
//! - per rig `n`: **the Thing** (`OWP-TV-01` … `OWP-TV-08`, owned by the
//!   applying content owner, as hulls are, created in the shed); **the unit**
//!   from the vehicle unit's binding, only when no unit record claims the Thing;
//!   **the rooms** (`cab`, `hopper`) from the template, dressed; **the spaces**
//!   (the hopper store, then the cab with its one member and its berths
//!   compartment, both riding the TV); **the hopper vault**, owned by the system
//!   principal [`crate::port::site::VEHICLES_PRINCIPAL`] with no scope, so no
//!   player may take from it (doc 20 §4.2); **the placement** in the shed, only
//!   when the TV has no placement record; and **the title** to the lessor org
//!   under the pool's issuer, only when no asset exists.
//!
//! The map records the objects ([`crate::port::RigRow`]); unit, spaces,
//! placement, title and lease are read from their owners. `preflight` refuses
//! every drift before any write. `vehicles_line` is the readiness report's
//! **ungated** vehicles line (mining content never closes the trade-run desk),
//! which also reports a programme hull whose refit bay is not laid up (reviewer
//! F13). The queries below are what `+tv`, the refit, the payload and the
//! recall read ([`rigs_in_bay`], [`hopper_of`], [`rig_row`],
//! [`occator_lease_on`]).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId, ObjKind, World};
use mush_economy::{
    CustodyRef, Deliverable, EconomicContract, EconomyStore, Vault, VaultId, VaultStatus,
};
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use mush_sim::data::{SpaceAnchor, SpatialParent, SpatialState};
use smol_str::SmolStr;

use crate::definitions::{GoodsTable, InteriorDef, LogisticsDef};
use crate::port::def::{PortDef, VehiclesDef};
use crate::port::hull::{declare_space, dress, live, registry_mark, unit_record_claims};
use crate::port::map::{PortMap, RigRow, VehiclesRow};
use crate::port::passage::CarriedRig;
use crate::port::readiness::LineState;
use crate::port::ship::HullView;
use crate::port::site::VEHICLES_PRINCIPAL;
use crate::store::{TacsimStore, put_port_map};

/// **The vehicle unit's interior template, its binding's instantiation shape
/// and its logistics** (doc 20 cut SM.3a), or the refusal naming what is
/// missing.
///
/// # Errors
///
/// A sentence naming the pool and what the catalog lacks.
pub fn template<'a>(
    store: &'a TacsimStore,
    def: &VehiclesDef,
) -> Result<
    (
        &'a InteriorDef,
        &'a mush_tactics::UnitSpec,
        &'a LogisticsDef,
    ),
    String,
> {
    let vid = &def.id;
    let unit = &def.unit;
    let spec = store
        .definitions()
        .specs
        .get(unit)
        .ok_or_else(|| format!("vehicles {vid}: unit {unit} is not fielded"))?;
    let (Some(interior), Some(logistics)) = (spec.interior.as_ref(), spec.logistics.as_ref())
    else {
        return Err(format!(
            "vehicles {vid}: unit {unit} carries no interior template or no logistics"
        ));
    };
    let binding = store
        .definitions()
        .bindings
        .get(unit)
        .ok_or_else(|| format!("vehicles {vid}: no unit schema {unit} is bound"))?;
    Ok((interior, &binding.spec, logistics))
}

/// The cab's berths compartment as a simulation slot id (a template id
/// `SlotId::new` refuses is a preflight refusal, never an `expect`).
fn cab_slot(def: &VehiclesDef, interior: &InteriorDef) -> Result<mush_sim::SlotId, String> {
    let id = &interior.berths_slot.id;
    mush_sim::SlotId::new(id.as_str()).ok_or_else(|| {
        format!(
            "vehicles {}: unit {}'s berths compartment '{id}' is not a simulation slot id",
            def.id, def.unit
        )
    })
}

/// The hopper vault's owner: the system principal, never a player (§4.2).
fn hopper_owner() -> Option<PrincipalId> {
    SystemPrincipalId::new(VEHICLES_PRINCIPAL).map(PrincipalId::System)
}

/// **Every rig refusal decidable before a write** (doc 20 cut SM.3a; the hull
/// preflight's sentences): the template missing, the issuer unregistered, a
/// recorded rig past the pool, a changed mark, a recorded object gone, a
/// recorded vault gone or not owned by `tacsim.vehicles`. Answers how many rigs
/// this pass would title, for the apply's frozen-economy check. Each refusal is
/// prefixed `"{id}: "`.
pub(crate) fn preflight(
    w: &World,
    pack: &CanonicalId,
    id: &CanonicalId,
    def: &PortDef,
    map: Option<&PortMap>,
) -> Result<usize, String> {
    let key = crate::port::port_map_key(pack, id);
    let recorded: &[RigRow] = map
        .and_then(|m| m.vehicles.as_ref())
        .map_or(&[], |row| row.rigs.as_slice());
    let Some(vehicles) = &def.vehicles else {
        if !recorded.is_empty() {
            return Err(format!(
                "{id}: the port map {key} records {} rigs but the entry has no vehicles block; \
                 rigs are never retired by a port update",
                recorded.len()
            ));
        }
        return Ok(0);
    };
    let vid = &vehicles.id;
    if w.module_state::<mush_tactics::UnitStore>().is_none() {
        return Err(format!(
            "{id}: tactical units are not composed on this world"
        ));
    }
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| format!("{id}: the TacSim layer is not composed on this world"))?;
    let economy = w
        .module_state::<EconomyStore>()
        .ok_or_else(|| format!("{id}: the economy is not composed on this world"))?;
    let (interior, _, _) = template(store, vehicles).map_err(|e| format!("{id}: {e}"))?;
    cab_slot(vehicles, interior).map_err(|e| format!("{id}: {e}"))?;
    let registered = SystemPrincipalId::new(&vehicles.issuer)
        .is_some_and(|issuer| economy.is_title_issuer(&issuer));
    if !registered {
        return Err(format!(
            "{id}: vehicles {vid}: the title issuer {} is not registered on this world",
            vehicles.issuer
        ));
    }
    let owner = hopper_owner();
    if let Some(shed) = map
        .and_then(|m| m.vehicles.as_ref())
        .and_then(|row| row.shed)
        && !live(w, shed)
    {
        return Err(format!(
            "{id}: the port map {key} names the shed #{shed}, which does not exist; it is never \
             re-created — repair the map by hand"
        ));
    }
    let mut to_title = 0_usize;
    for row in recorded {
        let mark = &row.mark;
        if row.n >= vehicles.pool {
            return Err(format!(
                "{id}: rig {mark} is in the port map {key} but the pool is {}; rigs are never \
                 retired by a port update",
                vehicles.pool
            ));
        }
        let expected = registry_mark(&vehicles.registry_prefix, row.n);
        if *mark != expected {
            return Err(format!(
                "{id}: rig {mark} in the port map {key} is not {expected}; registry marks are \
                 never re-issued"
            ));
        }
        let objects = std::iter::once(row.tv).chain(row.rooms.values().copied());
        for obj in objects {
            if !live(w, obj) {
                return Err(format!(
                    "{id}: rig {mark} in the port map {key} names #{obj}, which does not exist; it \
                     is never re-created — repair the map by hand"
                ));
            }
        }
        if let Some(room) = row.rooms.keys().find(|room| interior.room(room).is_none()) {
            return Err(format!(
                "{id}: rig {mark} in the port map {key} records room {room}, which the template \
                 does not author"
            ));
        }
        if let Some(vault) = row.vault {
            let ours = economy
                .snapshot()
                .vault(VaultId(vault))
                .is_some_and(|v| Some(&v.owner) == owner.as_ref());
            if !ours {
                return Err(format!(
                    "{id}: rig {mark} in the port map {key} names hopper vault #{vault}, which is \
                     gone or not owned by {VEHICLES_PRINCIPAL}; it is never re-created — repair \
                     the map by hand"
                ));
            }
        }
        if economy.snapshot().asset(ObjId(row.tv)).is_none() {
            to_title = to_title.saturating_add(1);
        }
    }
    let absent = (0..vehicles.pool)
        .filter(|n| !recorded.iter().any(|row| row.n == *n))
        .count();
    Ok(to_title.saturating_add(absent))
}

/// The rig row index of pool number `n`.
fn rig_index(map: &PortMap, n: u32) -> Option<usize> {
    map.vehicles
        .as_ref()
        .and_then(|row| row.rigs.iter().position(|rig| rig.n == n))
}

/// **Part (h): lay up the lessor's rigs** (doc 20 cut SM.3a). The shed once,
/// then each rig in pool order, every step idempotent, the map written after
/// every object. [`preflight`] has run: every refusal here is one a write in
/// this pass could not have foreseen.
pub(crate) fn lay_up(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    def: &VehiclesDef,
    map: &mut PortMap,
) -> Result<(), String> {
    let (interior, spec) = {
        let store = ctx
            .w
            .module_state::<TacsimStore>()
            .ok_or_else(|| format!("{id}: the TacSim layer is not composed"))?;
        let (interior, spec, _) = template(store, def).map_err(|e| format!("{id}: {e}"))?;
        (interior.clone(), spec.clone())
    };
    let slot = cab_slot(def, &interior).map_err(|e| format!("{id}: {e}"))?;
    let issuer = SystemPrincipalId::new(&def.issuer).ok_or_else(|| {
        format!(
            "{id}: vehicles {}: the title issuer {} is not a system principal",
            def.id, def.issuer
        )
    })?;
    let lessor = map.orgs.get(&def.org).cloned().ok_or_else(|| {
        format!(
            "{id}: vehicles {}: the org {} has no map row",
            def.id, def.org
        )
    })?;
    // (h0) The shed: made once, declared an unanchored interior space every pass.
    let shed = match map.vehicles.as_ref().and_then(|row| row.shed) {
        Some(shed) => ObjId(shed),
        None => {
            let shed = ctx
                .w
                .create(ObjKind::Room, &def.shed, ctx.actor, None)
                .map_err(|e| format!("{id}: the shed: {e}"))?;
            map.vehicles.get_or_insert_with(VehiclesRow::default).shed = Some(shed.0);
            put_port_map(ctx, map);
            shed
        }
    };
    declare_space(ctx, shed, SpaceAnchor::Unanchored)
        .map_err(|e| format!("{id}: the shed: {e}"))?;
    for n in 0..def.pool {
        lay_up_rig(
            ctx,
            id,
            map,
            &RigPlan {
                def,
                interior: &interior,
                spec: &spec,
                slot: &slot,
                shed,
                n,
            },
        )?;
        let tv = rig_index(map, n)
            .and_then(|i| map.vehicles.as_ref().and_then(|row| row.rigs.get(i)))
            .map(|rig| ObjId(rig.tv))
            .ok_or_else(|| format!("{id}: rig {n} has no map row"))?;
        place_and_title(ctx, id, tv, shed, &issuer, &lessor)?;
    }
    Ok(())
}

/// What one rig is made from.
struct RigPlan<'a> {
    def: &'a VehiclesDef,
    interior: &'a InteriorDef,
    spec: &'a mush_tactics::UnitSpec,
    slot: &'a mush_sim::SlotId,
    shed: ObjId,
    n: u32,
}

/// Steps 1–5 for rig `plan.n`: the Thing, the unit, the rooms (dressed), the
/// spaces (hopper, then cab with its member and its berths) and the hopper
/// vault.
fn lay_up_rig(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    map: &mut PortMap,
    plan: &RigPlan<'_>,
) -> Result<(), String> {
    let RigPlan {
        def,
        interior,
        spec,
        slot,
        shed,
        n,
    } = *plan;
    let mark = registry_mark(&def.registry_prefix, n);
    // 1. The Thing, named by its mark, in the shed.
    if rig_index(map, n).is_none() {
        let tv = ctx
            .w
            .create(ObjKind::Thing, &mark, ctx.actor, Some(shed))
            .map_err(|e| format!("{id}: rig {mark}: {e}"))?;
        map.vehicles
            .get_or_insert_with(VehiclesRow::default)
            .rigs
            .push(RigRow {
                n,
                mark: mark.clone(),
                tv: tv.0,
                rooms: BTreeMap::new(),
                vault: None,
            });
        put_port_map(ctx, map);
    }
    let index = rig_index(map, n).ok_or_else(|| format!("{id}: rig {mark} has no map row"))?;
    let row = |map: &PortMap| -> Option<RigRow> {
        map.vehicles
            .as_ref()
            .and_then(|row| row.rigs.get(index))
            .cloned()
    };
    let tv = row(map)
        .map(|rig| ObjId(rig.tv))
        .ok_or_else(|| format!("{id}: rig {mark} has no map row"))?;
    // 2. The unit — only when no unit record (a held one included) claims it.
    if !unit_record_claims(ctx.w, tv) {
        mush_tactics::module::create_unit(ctx, tv, spec)
            .map_err(|e| format!("{id}: rig {mark}: {e}"))?;
    }
    // 3. The rooms, template order; then every pass, the dress.
    for room in &interior.rooms {
        let made = row(map).is_some_and(|rig| rig.rooms.contains_key(&room.id));
        if !made {
            let obj = ctx
                .w
                .create(ObjKind::Room, &room.name, ctx.actor, None)
                .map_err(|e| format!("{id}: rig {mark} room {}: {e}", room.id))?;
            if let Some(rig) = map
                .vehicles
                .as_mut()
                .and_then(|row| row.rigs.get_mut(index))
            {
                rig.rooms.insert(room.id.clone(), obj.0);
            }
            put_port_map(ctx, map);
        }
    }
    let rooms: BTreeMap<CanonicalId, ObjId> = row(map)
        .map(|rig| {
            rig.rooms
                .iter()
                .map(|(room, obj)| (room.clone(), ObjId(*obj)))
                .collect()
        })
        .unwrap_or_default();
    for room in &interior.rooms {
        if let Some(obj) = rooms.get(&room.id) {
            dress(ctx, *obj, room)
                .map_err(|e| format!("{id}: rig {mark} room {}: {e}", room.id))?;
        }
    }
    // 4. The spaces: the hopper store, then the cab (its one member and its berths),
    // both riding the TV — the hull's shape.
    let hopper = *rooms
        .get(&interior.store)
        .ok_or_else(|| format!("{id}: rig {mark} has no hopper room"))?;
    let cab = *rooms
        .get(&interior.space)
        .ok_or_else(|| format!("{id}: rig {mark} has no cab room"))?;
    let riding = SpaceAnchor::MovingObject { host: tv };
    declare_space(ctx, hopper, riding.clone())
        .map_err(|e| format!("{id}: rig {mark} hopper: {e}"))?;
    declare_space(ctx, cab, riding).map_err(|e| format!("{id}: rig {mark} cab: {e}"))?;
    let members: BTreeSet<ObjId> = interior
        .rooms
        .iter()
        .filter(|room| room.kind == crate::definitions::InteriorRoomKind::Walkable)
        .filter_map(|room| rooms.get(&room.id).copied())
        .collect();
    let (members_ok, berths_ok) = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|st| st.spaces.get(&cab))
        .map_or((false, false), |rec| {
            (
                rec.rooms == members,
                rec.compartments.get(slot).is_some_and(|cap| {
                    cap.maximum_items == Some(interior.berths_slot.maximum_items)
                }),
            )
        });
    if !members_ok {
        mush_sim::run(
            ctx,
            &mush_sim::Transition::Members {
                space: cab,
                rooms: members,
            },
        )
        .map_err(|e| format!("{id}: rig {mark} cab members: {e}"))?;
    }
    if !berths_ok {
        mush_sim::declare_compartment(
            ctx,
            cab,
            slot.clone(),
            Some(interior.berths_slot.maximum_items),
        )
        .map_err(|e| format!("{id}: rig {mark} berths: {e}"))?;
    }
    // 5. The hopper vault (doc 20 §4.2): the system principal's, no scope.
    if row(map).is_some_and(|rig| rig.vault.is_none()) {
        let owner = hopper_owner().ok_or_else(|| {
            format!("{id}: rig {mark}: {VEHICLES_PRINCIPAL} is not a system principal id")
        })?;
        let vault = crate::port::apply::alloc_vault_id(ctx)
            .map_err(|e| format!("{id}: rig {mark} hopper vault: {e}"))?;
        mush_economy::create_vault(
            ctx,
            Vault {
                id: vault,
                owner,
                scope: None,
                name: SmolStr::new(format!("{mark} hopper")),
                backing: CustodyRef::SpatialInterior(hopper),
                status: VaultStatus::Active,
                revision: 1,
            },
        );
        if let Some(rig) = map
            .vehicles
            .as_mut()
            .and_then(|row| row.rigs.get_mut(index))
        {
            rig.vault = Some(vault.0);
        }
        put_port_map(ctx, map);
    }
    Ok(())
}

/// Steps 6 and 7 for one rig: the placement in the shed (only for a TV with no
/// placement record at all, held ones included: the hull step-7 rule) and the
/// title to the lessor (only for a TV with no asset record).
fn place_and_title(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    tv: ObjId,
    shed: ObjId,
    issuer: &SystemPrincipalId,
    lessor: &crate::port::PortOrgRow,
) -> Result<(), String> {
    let placed = ctx
        .w
        .module_state::<SpatialState>()
        .is_some_and(|st| st.placements.contains_key(&tv) || st.held_placement(tv).is_some());
    if !placed {
        mush_sim::place_interior(ctx, tv, shed, None)
            .map_err(|e| format!("{id}: rig #{}: {e}", tv.0))?;
    }
    let titled = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|store| store.snapshot().asset(tv).is_some());
    if !titled {
        mush_economy::issue_title(
            ctx,
            tv,
            PrincipalId::Organization(OrgId(lessor.org)),
            Some(OrgScopeId(lessor.root_scope)),
            issuer.clone(),
            None,
        )
        .map_err(|e| format!("{id}: rig #{} could not be titled: {e}", tv.0))?;
    }
    Ok(())
}

/// Whether `vault` is a live hopper vault: present and owned by the system
/// principal (§4.2).
fn hopper_vault_ok(economy: Option<&EconomyStore>, vault: Option<u64>) -> bool {
    let owner = hopper_owner();
    vault.is_some_and(|vault| {
        economy.is_some_and(|economy| {
            economy.snapshot().vault(VaultId(vault)).is_some_and(|v| {
                Some(&v.owner) == owner.as_ref() && v.status == VaultStatus::Active
            })
        })
    })
}

/// **The readiness report's vehicles line** (doc 20 cut SM.3a; reported, never
/// gated — mining content never closes the trade-run desk): the first failing
/// check, else ready. `None` for an entry with no `vehicles` block.
pub(crate) fn vehicles_line(
    w: &World,
    store: &TacsimStore,
    entry: &CanonicalId,
    def: &PortDef,
    map: &PortMap,
) -> Option<(LineState, String)> {
    let vehicles = def.vehicles.as_ref()?;
    let pool = vehicles.pool;
    let org_name = def
        .orgs
        .iter()
        .find(|org| org.id == vehicles.org)
        .map_or_else(|| vehicles.org.to_string(), |org| org.name.clone());
    let missing = |text: String| Some((LineState::Missing, format!("{entry} vehicles: {text}")));
    let empty: &[RigRow] = &[];
    let rigs = map
        .vehicles
        .as_ref()
        .map_or(empty, |row| row.rigs.as_slice());
    let made = rigs.iter().filter(|rig| rig.n < pool).count();
    if made < usize::try_from(pool).unwrap_or(usize::MAX) {
        return missing(format!("{made} of {pool} are made."));
    }
    let shed = map.vehicles.as_ref().and_then(|row| row.shed);
    let interior = template(store, vehicles)
        .ok()
        .map(|(interior, _, _)| interior);
    let spatial = w.module_state::<SpatialState>();
    let economy = w.module_state::<EconomyStore>();
    let lessor = map
        .orgs
        .get(&vehicles.org)
        .map(|row| PrincipalId::Organization(OrgId(row.org)));
    let mut in_shed = 0_usize;
    let mut leased = 0_usize;
    for rig in rigs {
        let mark = &rig.mark;
        let gone = std::iter::once(rig.tv)
            .chain(rig.rooms.values().copied())
            .chain(shed)
            .find(|obj| !live(w, *obj));
        if let Some(obj) = gone {
            return missing(format!("{mark} (#{obj}) does not exist."));
        }
        let complete = interior.is_some_and(|interior| {
            interior
                .rooms
                .iter()
                .all(|room| rig.rooms.contains_key(&room.id))
                && [&interior.space, &interior.store].iter().all(|room| {
                    rig.rooms.get(*room).is_some_and(|obj| {
                        spatial.is_some_and(|st| st.spaces.contains_key(&ObjId(*obj)))
                    })
                })
        });
        if !complete {
            return missing(format!("{mark}'s interior is incomplete."));
        }
        if !hopper_vault_ok(economy, rig.vault) {
            return missing(format!("{mark} has no hopper vault."));
        }
        let tv = ObjId(rig.tv);
        let titled = lessor
            .as_ref()
            .is_some_and(|lessor| crate::port::coop::titled_to(w, tv, lessor));
        if !titled {
            return missing(format!("{mark} is not titled to {org_name}."));
        }
        let parked = shed.is_some_and(|shed| {
            spatial.is_some_and(|st| {
                matches!(st.parent_of(tv), SpatialParent::Interior { space, .. } if space.0 == shed)
            })
        });
        if parked {
            in_shed = in_shed.saturating_add(1);
        }
        if lessor
            .as_ref()
            .and_then(|lessor| occator_lease_on(w, lessor, tv))
            .is_some()
        {
            leased = leased.saturating_add(1);
        }
    }
    // Reviewer F13: the first programme hull, in row order, whose refit bay is not
    // laid up — reported here, never on the gated hulls line.
    if let Some(programme) = &def.programme {
        let bays = crate::port::hull::refit_bays(store, programme);
        if let Some(hull) = map.hulls.iter().find(|row| {
            bays.iter()
                .any(|(bay, _)| !crate::port::hull::bay_laid_up(w, row, bay))
        }) {
            return missing(format!("hull {}'s bay is not laid up.", hull.mark));
        }
    }
    Some((
        LineState::Ready,
        format!(
            "{entry} vehicles: {pool} of {pool} made and titled to {org_name}; {in_shed} in the \
             shed, {leased} leased."
        ),
    ))
}

// ---- the queries the next cuts build on ------------------------------------------

/// **One rig in a hull's bay** (doc 20 cut SM.3a).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigInBay {
    /// Its slot number: the slot of its **live** lease record (Active or
    /// Forfeit), or `0` for a rig placed in the bay with no live record — which
    /// `+tv` reports as unlisted.
    pub slot: u8,
    /// The terrain vehicle.
    pub tv: ObjId,
    /// Its registry mark (the rig row's), else its name.
    pub mark: String,
    /// Its hopper vault.
    pub hopper: Option<VaultId>,
    /// Its live lease record's lease contract, `ContractId.0`.
    pub lease: Option<u64>,
}

/// **The bay rooms a hull's row records** (doc 20 cut SM.3a): its own unit's bays
/// and, so a held or standard view still sees them, every refit unit's — the bay
/// room is the row's, not the unit's (D-20-7).
pub(crate) fn bay_rooms(w: &World, view: &HullView<'_>) -> Vec<ObjId> {
    let mut rooms: Vec<ObjId> = view
        .interior
        .bays
        .iter()
        .filter_map(|bay| view.room(&bay.room))
        .collect();
    if let Some(store) = w.module_state::<TacsimStore>() {
        for (bay, _) in crate::port::hull::refit_bays(store, view.port.programme) {
            if let Some(room) = view.room(&bay.room)
                && !rooms.contains(&room)
            {
                rooms.push(room);
            }
        }
    }
    rooms
}

/// **The rigs placed in `view`'s bay** (doc 20 cut SM.3a): every placement
/// `Interior` in one of `bay_rooms`, by slot number — the numbered first, an
/// unlisted rig (slot `0`) after them.
#[must_use]
pub fn rigs_in_bay(w: &World, view: &HullView<'_>) -> Vec<RigInBay> {
    let Some(st) = w.module_state::<SpatialState>() else {
        return Vec::new();
    };
    let store = w.module_state::<TacsimStore>();
    let mut out: Vec<RigInBay> = Vec::new();
    for room in bay_rooms(w, view) {
        for obj in st.children_of(room) {
            let SpatialParent::Interior { space, .. } = st.parent_of(*obj) else {
                continue;
            };
            if space != room {
                continue;
            }
            let record = store.and_then(|store| store.tv_lease_on(*obj));
            let mark = rig_row(w, *obj).map_or_else(
                || crate::port::coop::name_of(w, *obj),
                |(_, rig)| rig.mark.clone(),
            );
            out.push(RigInBay {
                slot: record.map_or(0, |record| record.slot),
                tv: *obj,
                mark,
                hopper: hopper_of(w, *obj),
                lease: record.map(|record| record.lease),
            });
        }
    }
    out.sort_by_key(|rig| (rig.slot == 0, rig.slot, rig.tv));
    out
}

/// **A rig's hopper vault** (doc 20 cut SM.3a): its rig row's `vault`.
#[must_use]
pub fn hopper_of(w: &World, tv: ObjId) -> Option<VaultId> {
    rig_row(w, tv).and_then(|(_, rig)| rig.vault.map(VaultId))
}

/// **A rig's map row** (doc 20 cut SM.3a) and the port map that holds it.
#[must_use]
pub fn rig_row(w: &World, tv: ObjId) -> Option<(&PortMap, &RigRow)> {
    w.module_state::<TacsimStore>()?
        .port_maps()
        .find_map(|map| {
            map.vehicles
                .as_ref()?
                .rigs
                .iter()
                .find(|rig| rig.tv == tv.0)
                .map(|rig| (map, rig))
        })
}

/// **The lessor's live lease on one rig** (doc 20 cut SM.3a; `trust_lease_on`'s
/// shape): non-terminal, issued by `lessor`, pledging `tv` as the lessor's
/// collateral and lease-shaped. It answers "is the economy lease live"; the
/// lease record answers "is the record live" (`TacsimStore::tv_lease_on`), and
/// `+tv` needs both (reviewer F2, F15).
#[must_use]
pub fn occator_lease_on<'a>(
    w: &'a World,
    lessor: &PrincipalId,
    tv: ObjId,
) -> Option<&'a EconomicContract> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contracts()
        .find(|c| {
            !c.state.is_terminal()
                && c.issuer == *lessor
                && c.consent.terms.deliverables.iter().any(|d| {
                    matches!(
                        d,
                        Deliverable::Collateral { object, from } if *object == tv && from == lessor
                    )
                })
                && mush_economy::is_lease_shaped(c)
        })
}

/// **The rigs in `view`'s bay as the payload counts them** (doc 20 cut SM.3a;
/// §4.3, D-20-7): per rig its dry mass (its live unit's `dry_mass_t` × 1,000,
/// else the pool's unit's), its hopper vault and every good its hopper accepts
/// (every goods row whose stowage some compartment of the TV's logistics
/// accepts), in [`rigs_in_bay`]'s order.
///
/// # Errors
///
/// A rig whose unit or logistics cannot be read.
pub(crate) fn carried_rigs(
    w: &World,
    view: &HullView<'_>,
    goods: &GoodsTable,
) -> Result<Vec<CarriedRig>, String> {
    let rigs = rigs_in_bay(w, view);
    if rigs.is_empty() {
        return Ok(Vec::new());
    }
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| "the layer's state is absent".to_owned())?;
    let pool_unit = view.port.def.vehicles.as_ref().map(|v| v.unit.clone());
    let mut out = Vec::with_capacity(rigs.len());
    for rig in rigs {
        if crate::relay_foundry::cargo(w, rig.tv) {
            out.push(CarriedRig {
                tv: rig.tv.0,
                dry_kg: crate::relay_foundry::HALF_KG,
                hopper: None,
                goods: Vec::new(),
            });
            continue;
        }
        if crate::relay_foundry::known_cargo(w, rig.tv) {
            return Err(format!("relay cargo #{} is unavailable", rig.tv.0));
        }
        let unit = crate::port::hull::live_unit_id(w, rig.tv)
            .or_else(|| pool_unit.clone())
            .ok_or_else(|| format!("rig #{} has no unit", rig.tv.0))?;
        let logistics = store
            .definitions()
            .specs
            .get(&unit)
            .and_then(|spec| spec.logistics.as_ref())
            .ok_or_else(|| format!("rig #{}'s unit {unit} carries no logistics", rig.tv.0))?;
        let dry_kg = u64::from(logistics.dry_mass_t)
            .checked_mul(1_000)
            .ok_or_else(|| format!("rig #{}'s dry mass overflows", rig.tv.0))?;
        let accepted: Vec<CanonicalId> = goods
            .goods
            .iter()
            .filter(|good| {
                logistics
                    .stowage
                    .values()
                    .any(|compartment| compartment.accepts.contains(&good.stowage))
            })
            .map(|good| good.id.clone())
            .collect();
        out.push(CarriedRig {
            tv: rig.tv.0,
            dry_kg,
            hopper: rig.hopper.map(|vault| vault.0),
            goods: accepted,
        });
    }
    Ok(out)
}
