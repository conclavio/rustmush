//! **The programme hulls** (doc 18 cut IC4; doc 18 §4.2 "Object model", §4.7
//! part (f), D-18-22, D-18-29; 18a §3 and §4; lead rulings L8–L18 of the IC4
//! brief).
//!
//! For a port entry whose `programme` names a pool (Ceres: eight Drays), the
//! `PortApplier` instantiates every hull from the programme unit's `interior`
//! template, in pool order, each act recorded in the port map before the next
//! (`lay_up`):
//!
//! 1. **the Thing** — named from `hull_names`, aliased with its registry mark
//!    ([`registry_mark`]), owned by the applying content owner (never a pilot,
//!    so `perm::controls` cannot bypass the port verbs), standing in the berth
//!    gallery;
//! 2. **the unit** — `tactics.unit` bound from the programme's unit entry,
//!    only for a hull no unit record (a held one included) claims;
//! 3. **the rooms** — the six walkable rooms and the store, dressed every pass
//!    with the template's 18a §3.1 text: `desc`, the tree's `short_desc`
//!    attribute and the details on tacsim's own detail shelf
//!    ([`HULL_DETAIL_PREFIX`]; hull rooms are not atlas rooms, L14);
//! 4. **the exits** — the template's pairs, in `open_exit`'s shape; an exit
//!    already made is re-dressed every pass, its name and aliases following the
//!    template (cut IC8): **`interior.exits` are append-only by position** — an
//!    exit is the one the map recorded at its index, never re-made, so a later
//!    template may change a pair's words but never reorder or drop a pair;
//! 5. **the store** — a single-room interior space anchored on the hull (the
//!    hold vault's backing, cut IC6);
//! 6. **the airlock** — the interior space whose `Transition::Members` are the
//!    six walkable rooms, with the `berths` compartment;
//! 7. **the placement** — `Direct` on the port's passive domain, laid up at
//!    `(0, n × HULL_SPACING_CM)`, only for a hull with no placement record
//!    (a held, unreadable one included);
//! 8. **the title** — issued to the programme org (the Co-op First Captain
//!    Trust) under [`REGISTRY_ISSUER`], only for a hull with no asset record.
//!
//! The map records the objects only ([`crate::port::HullRow`]); the unit, the
//! spaces, the members, the compartment, the placement and the title are read
//! from their owners (L9). A re-apply creates nothing; a later version that
//! grows the pool creates only the new hulls; a shrunk pool, a changed prefix,
//! a recorded object gone or an exit the template no longer authors is refused
//! by `preflight` before any write (L10). `hull_line` is the readiness
//! report's hulls line (L17).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    Attr, BINDING_SCHEMA_VERSION, BehaviorBinding, BehaviorId, CanonicalId, Ctx, ObjId, ObjKind,
    World, WorldEvent,
};
use mush_economy::EconomyStore;
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use mush_sim::data::{DomainId, SpaceAnchor, SpatialParent, SpatialState};

use crate::definitions::{InteriorDef, InteriorRoomDef, InteriorRoomKind};
use crate::port::def::{PortDef, ProgrammeDef};
use crate::port::map::{HullRow, PortMap};
use crate::port::readiness::LineState;
use crate::store::{TacsimStore, put_port_map};

/// The system principal hull titles are minted under (doc 18 §4.2 schema
/// addition 3; L13): registered on every world by
/// `crate::composition::server_modules_with`, named by the lane's programme
/// `issuer`.
pub const REGISTRY_ISSUER: &str = "tacsim.registry";

/// The attribute prefix of a hull room's details (L14): tacsim's own `tacsim`
/// namespace, registered with `look` by `TacsimRuleset::register`.
pub const HULL_DETAIL_PREFIX: &str = "tacsim:detail:";

/// Laid-up hulls stand at `(0, n × 10,000 cm)` on the port's passive domain
/// (L16).
pub const HULL_SPACING_CM: i64 = 10_000;

/// **A hull's registry mark** (L12): the programme's prefix and the two-digit
/// pool number from 1, so `("FPC-D-04", 0)` is `"FPC-D-0401"`. `n` is below
/// [`crate::port::def::MAX_PROGRAMME_POOL`] (`ProgrammeDef::check`), so the add never overflows;
/// it is checked anyway.
#[must_use]
pub fn registry_mark(prefix: &str, n: u32) -> String {
    match n.checked_add(1) {
        Some(number) => format!("{prefix}{number:02}"),
        None => format!("{prefix}{n}"),
    }
}

/// Whether `obj` is live: present and not `GOING`.
fn live(w: &World, obj: u32) -> bool {
    w.get(ObjId(obj))
        .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
}

/// The programme unit's interior template and its binding's instantiation
/// shape, or the refusal naming what is missing.
pub(crate) fn template<'a>(
    store: &'a TacsimStore,
    programme: &ProgrammeDef,
) -> Result<(&'a InteriorDef, &'a mush_tactics::UnitSpec), String> {
    let pid = &programme.id;
    let unit = &programme.unit;
    let interior = store
        .definitions()
        .specs
        .get(unit)
        .filter(|spec| spec.logistics.is_some())
        .and_then(|spec| spec.interior.as_ref())
        .ok_or_else(|| format!("programme {pid}: unit {unit} carries no interior template"))?;
    let binding = store
        .definitions()
        .bindings
        .get(unit)
        .ok_or_else(|| format!("programme {pid}: no unit schema {unit} is bound"))?;
    Ok((interior, &binding.spec))
}

/// **The programme unit's interior template and logistics block** (doc 18 cut
/// IC6), both present: what the Co-op's free-hull rule, `+ship` and the gauges
/// read. `None` when the unit carries either block no longer.
pub(crate) fn hull_spec<'a>(
    store: &'a TacsimStore,
    programme: &ProgrammeDef,
) -> Option<(&'a InteriorDef, &'a crate::definitions::LogisticsDef)> {
    let spec = store.definitions().specs.get(&programme.unit)?;
    Some((spec.interior.as_ref()?, spec.logistics.as_ref()?))
}

/// The berths compartment's simulation slot id, converted once (a template id
/// `SlotId::new` refuses is a preflight refusal, never an `expect`).
pub(crate) fn berths_slot(
    programme: &ProgrammeDef,
    interior: &InteriorDef,
) -> Result<mush_sim::SlotId, String> {
    let id = &interior.berths_slot.id;
    mush_sim::SlotId::new(id.as_str()).ok_or_else(|| {
        format!(
            "programme {}: unit {}'s berths compartment '{id}' is not a simulation slot id",
            programme.id, programme.unit
        )
    })
}

/// **Every hull refusal decidable before a write** (doc 18 §4.7 "re-applies by
/// map row, never by name"; L10, L13, L15). Answers how many hulls still need
/// a title, for the apply's frozen-economy check. Each refusal is prefixed
/// `"{id}: "`.
pub(crate) fn preflight(
    reg: &mush_core::Registry,
    w: &World,
    pack: &CanonicalId,
    id: &CanonicalId,
    def: &PortDef,
    map: Option<&PortMap>,
) -> Result<usize, String> {
    let key = crate::port::port_map_key(pack, id);
    let recorded: &[HullRow] = map.map_or(&[], |m| m.hulls.as_slice());
    let Some(programme) = &def.programme else {
        if !recorded.is_empty() {
            return Err(format!(
                "{id}: the port map {key} records {} hulls but the entry has no programme; hulls \
                 are never retired by a port update",
                recorded.len()
            ));
        }
        return Ok(0);
    };
    let pid = &programme.id;
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
    let (interior, _) = template(store, programme).map_err(|e| format!("{id}: {e}"))?;
    let registered = SystemPrincipalId::new(&programme.issuer)
        .is_some_and(|issuer| economy.is_title_issuer(&issuer));
    if !registered {
        return Err(format!(
            "{id}: programme {pid}: the title issuer {} is not registered on this world",
            programme.issuer
        ));
    }
    for exit in &interior.exits {
        for segment in exit.names.iter().flat_map(|name| name.split(';')) {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            if let Some(command) = reg.kernel_command_named(segment) {
                return Err(format!(
                    "{id}: the hull exit '{segment}' is the command '{command}', which works in \
                     every room, and an exit would swallow it"
                ));
            }
        }
    }
    berths_slot(programme, interior).map_err(|e| format!("{id}: {e}"))?;
    // Drift against the recorded rows (L10).
    let authored_exits = interior.exits.len().saturating_mul(2);
    let mut to_title = 0_usize;
    for row in recorded {
        let mark = &row.mark;
        if row.n >= programme.pool {
            return Err(format!(
                "{id}: hull {mark} is in the port map {key} but the programme's pool is {}; hulls \
                 are never retired by a port update",
                programme.pool
            ));
        }
        let expected = registry_mark(&programme.registry_prefix, row.n);
        if *mark != expected {
            return Err(format!(
                "{id}: hull {mark} in the port map {key} is not {expected}; registry marks are \
                 never re-issued"
            ));
        }
        let objects = std::iter::once(row.hull)
            .chain(row.rooms.values().copied())
            .chain(row.exits.iter().copied());
        for obj in objects {
            if !live(w, obj) {
                return Err(format!(
                    "{id}: hull {mark} in the port map {key} names #{obj}, which does not exist; \
                     it is never re-created — repair the map by hand"
                ));
            }
        }
        if let Some(room) = row.rooms.keys().find(|room| interior.room(room).is_none()) {
            return Err(format!(
                "{id}: hull {mark} in the port map {key} records room {room}, which the template \
                 does not author"
            ));
        }
        if row.exits.len() > authored_exits {
            return Err(format!(
                "{id}: hull {mark} in the port map {key} records {} exits but the template \
                 authors {authored_exits}; exits are never retired by a port update",
                row.exits.len()
            ));
        }
        if economy.snapshot().asset(ObjId(row.hull)).is_none() {
            to_title = to_title.saturating_add(1);
        }
    }
    let absent = (0..programme.pool)
        .filter(|n| !recorded.iter().any(|row| row.n == *n))
        .count();
    Ok(to_title.saturating_add(absent))
}

/// The row index of pool number `n`.
fn row_of(map: &PortMap, n: u32) -> Option<usize> {
    map.hulls.iter().position(|row| row.n == n)
}

/// **Part (f): lay up the programme's hulls** (doc 18 §4.7; L8–L16). Per hull
/// `n` in pool order (so the first hull's Thing has the lowest id: cut IC6's
/// `next_loaner` takes the lowest), each step idempotent, the map written after
/// every object created (L11). `preflight` has run: every refusal here is one a
/// write in this pass could not have foreseen.
pub(crate) fn lay_up(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    programme: &ProgrammeDef,
    map: &mut PortMap,
    domain: DomainId,
    gallery: ObjId,
) -> Result<(), String> {
    let (interior, spec) = {
        let store = ctx
            .w
            .module_state::<TacsimStore>()
            .ok_or_else(|| format!("{id}: the TacSim layer is not composed"))?;
        let (interior, spec) = template(store, programme).map_err(|e| format!("{id}: {e}"))?;
        (interior.clone(), spec.clone())
    };
    let slot = berths_slot(programme, &interior).map_err(|e| format!("{id}: {e}"))?;
    let issuer = SystemPrincipalId::new(&programme.issuer).ok_or_else(|| {
        format!(
            "{id}: programme {}: the title issuer {} is not a system principal",
            programme.id, programme.issuer
        )
    })?;
    let trust = map.orgs.get(&programme.org).cloned().ok_or_else(|| {
        format!(
            "{id}: programme {}: the org {} has no map row",
            programme.id, programme.org
        )
    })?;
    for n in 0..programme.pool {
        lay_up_one(
            ctx,
            id,
            map,
            &HullPlan {
                programme,
                interior: &interior,
                spec: &spec,
                slot: &slot,
                n,
            },
            gallery,
        )?;
        let hull = row_of(map, n)
            .and_then(|i| map.hulls.get(i))
            .map(|row| ObjId(row.hull))
            .ok_or_else(|| format!("{id}: hull {n} has no map row"))?;
        place_and_title(ctx, id, hull, n, domain, &issuer, &trust)?;
    }
    Ok(())
}

/// What one hull is made from.
struct HullPlan<'a> {
    programme: &'a ProgrammeDef,
    interior: &'a InteriorDef,
    spec: &'a mush_tactics::UnitSpec,
    slot: &'a mush_sim::SlotId,
    n: u32,
}

/// Steps 1–6 for hull `plan.n`: the Thing, the unit, the rooms (dressed), the
/// exits, the store space and the airlock space with its members and berths.
fn lay_up_one(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    map: &mut PortMap,
    plan: &HullPlan<'_>,
    gallery: ObjId,
) -> Result<(), String> {
    let HullPlan {
        programme,
        interior,
        spec,
        slot,
        n,
    } = *plan;
    let mark = registry_mark(&programme.registry_prefix, n);
    // 1. The Thing.
    if row_of(map, n).is_none() {
        let name = usize::try_from(n)
            .ok()
            .and_then(|i| programme.hull_names.get(i))
            .ok_or_else(|| format!("{id}: hull {mark} has no name"))?;
        let hull = ctx
            .w
            .create(ObjKind::Thing, name, ctx.actor, Some(gallery))
            .map_err(|e| format!("{id}: hull {mark}: {e}"))?;
        ctx.w
            .add_alias(hull, &mark)
            .map_err(|e| format!("{id}: hull {mark}: {e}"))?;
        map.hulls.push(HullRow {
            n,
            mark: mark.clone(),
            hull: hull.0,
            rooms: BTreeMap::new(),
            exits: Vec::new(),
        });
        put_port_map(ctx, map);
    }
    let index = row_of(map, n).ok_or_else(|| format!("{id}: hull {mark} has no map row"))?;
    let hull = map
        .hulls
        .get(index)
        .map(|row| ObjId(row.hull))
        .ok_or_else(|| format!("{id}: hull {mark} has no map row"))?;
    // 2. The unit — only for a hull no unit record claims at all. A record
    // boot held (frozen, migrating or quarantined: a document that did not
    // decode as a live unit) is still the hull's record: it is staff's to
    // repair, never shadowed by a second unit, and it never stops the pass
    // for the other hulls (L10, the held-placement rule of step 7).
    let has_unit =
        ctx.w
            .module_state::<mush_tactics::UnitStore>()
            .is_some_and(|store| {
                store.units.contains_key(&hull)
                    || store.records.iter().any(|record| {
                        record.key_object == Some(hull) || record.object == Some(hull)
                    })
            });
    if !has_unit {
        mush_tactics::module::create_unit(ctx, hull, spec)
            .map_err(|e| format!("{id}: hull {mark}: {e}"))?;
    }
    // 3. The rooms, template order; then every pass, the dress.
    for room in &interior.rooms {
        let made = map
            .hulls
            .get(index)
            .is_some_and(|row| row.rooms.contains_key(&room.id));
        if !made {
            let obj = ctx
                .w
                .create(ObjKind::Room, &room.name, ctx.actor, None)
                .map_err(|e| format!("{id}: hull {mark} room {}: {e}", room.id))?;
            if let Some(row) = map.hulls.get_mut(index) {
                row.rooms.insert(room.id.clone(), obj.0);
            }
            put_port_map(ctx, map);
        }
    }
    let rooms: BTreeMap<CanonicalId, ObjId> = map
        .hulls
        .get(index)
        .map(|row| {
            row.rooms
                .iter()
                .map(|(room, obj)| (room.clone(), ObjId(*obj)))
                .collect()
        })
        .unwrap_or_default();
    for room in &interior.rooms {
        if let Some(obj) = rooms.get(&room.id) {
            dress(ctx, *obj, room)
                .map_err(|e| format!("{id}: hull {mark} room {}: {e}", room.id))?;
        }
    }
    // 4. The exits, template order: pair i is 2i (names[0], from → to) and
    // 2i+1 (names[1], to → from).
    for (i, pair) in interior.exits.iter().enumerate() {
        let from = *rooms
            .get(&pair.from)
            .ok_or_else(|| format!("{id}: hull {mark} has no room {}", pair.from))?;
        let to = *rooms
            .get(&pair.to)
            .ok_or_else(|| format!("{id}: hull {mark} has no room {}", pair.to))?;
        for (side, (src, dest)) in [(from, to), (to, from)].into_iter().enumerate() {
            let at = i.saturating_mul(2).saturating_add(side);
            let made = map.hulls.get(index).map_or(0, |row| row.exits.len());
            let spec = pair
                .names
                .get(side)
                .ok_or_else(|| format!("{id}: hull {mark} exit pair {i} lacks a name"))?;
            // Doc 18 cut IC8 (lead 2b(i)): an exit already made is re-dressed the
            // way `dress` re-dresses a room — its name and aliases follow the
            // template. **`interior.exits` are append-only by position**: exit `at`
            // is the one the map recorded at `at`, never re-made or re-linked, so a
            // template may change an exit's words but never reorder or drop a pair.
            if at < made {
                let exit = map
                    .hulls
                    .get(index)
                    .and_then(|row| row.exits.get(at))
                    .map(|obj| ObjId(*obj))
                    .ok_or_else(|| format!("{id}: hull {mark} exit {at} has no map row"))?;
                redress_exit(ctx, exit, spec)
                    .map_err(|e| format!("{id}: hull {mark} exit {spec}: {e}"))?;
                continue;
            }
            if made != at {
                continue;
            }
            let exit = open_exit(ctx, src, spec, dest)
                .map_err(|e| format!("{id}: hull {mark} exit {spec}: {e}"))?;
            if let Some(row) = map.hulls.get_mut(index) {
                row.exits.push(exit.0);
            }
            put_port_map(ctx, map);
        }
    }
    // 5. The store, then 6. the airlock (members and berths).
    let store = *rooms
        .get(&interior.store)
        .ok_or_else(|| format!("{id}: hull {mark} has no store room"))?;
    let space = *rooms
        .get(&interior.space)
        .ok_or_else(|| format!("{id}: hull {mark} has no airlock room"))?;
    declare_space(ctx, store, hull).map_err(|e| format!("{id}: hull {mark} store: {e}"))?;
    declare_space(ctx, space, hull).map_err(|e| format!("{id}: hull {mark} airlock: {e}"))?;
    let walkable: BTreeSet<ObjId> = interior
        .rooms
        .iter()
        .filter(|room| room.kind == InteriorRoomKind::Walkable)
        .filter_map(|room| rooms.get(&room.id).copied())
        .collect();
    let (members_ok, berths_ok) = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|st| st.spaces.get(&space))
        .map_or((false, false), |rec| {
            (
                rec.rooms == walkable,
                rec.compartments.get(slot).is_some_and(|cap| {
                    cap.maximum_items == Some(interior.berths_slot.maximum_items)
                }),
            )
        });
    if !members_ok {
        mush_sim::run(
            ctx,
            &mush_sim::Transition::Members {
                space,
                rooms: walkable,
            },
        )
        .map_err(|e| format!("{id}: hull {mark} airlock members: {e}"))?;
    }
    if !berths_ok {
        mush_sim::declare_compartment(
            ctx,
            space,
            slot.clone(),
            Some(interior.berths_slot.maximum_items),
        )
        .map_err(|e| format!("{id}: hull {mark} berths: {e}"))?;
    }
    Ok(())
}

/// Steps 7 and 8 for one hull: the laid-up placement (only for a hull with no
/// placement record at all) and the title (only for a hull with no asset
/// record). A docked-elsewhere, under-way or quarantined hull is never
/// re-placed; neither is one whose `placement:` document boot held as
/// unreadable (it is a placement record; staff repair it, and `place` would
/// refuse it mid-pass); a bought-out hull is never re-titled (L10).
fn place_and_title(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    hull: ObjId,
    n: u32,
    domain: DomainId,
    issuer: &SystemPrincipalId,
    trust: &crate::port::PortOrgRow,
) -> Result<(), String> {
    let placed = ctx
        .w
        .module_state::<SpatialState>()
        .is_some_and(|st| st.placements.contains_key(&hull) || st.held_placement(hull).is_some());
    if !placed {
        let pose = mush_sim::data::Pose {
            x_cm: 0,
            y_cm: i64::from(n).saturating_mul(HULL_SPACING_CM),
            z_cm: 0,
            heading_mdeg: 0,
            posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
                .map_err(|e| format!("{id}: {e}"))?,
        };
        mush_sim::place(ctx, hull, domain, pose)
            .map_err(|e| format!("{id}: hull #{}: {e}", hull.0))?;
    }
    let titled = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|store| store.snapshot().asset(hull).is_some());
    if !titled {
        mush_economy::issue_title(
            ctx,
            hull,
            PrincipalId::Organization(OrgId(trust.org)),
            Some(OrgScopeId(trust.root_scope)),
            issuer.clone(),
            None,
        )
        .map_err(|e| {
            format!(
                "{id}: hull {} could not be titled: {e}",
                mush_core::output::display_name(ctx.w, hull)
            )
        })?;
    }
    Ok(())
}

/// Write a hull room's source-owned fields (L14): the name when it differs,
/// `desc` and `short_desc` (removed when blank), and exactly the template's
/// details on [`HULL_DETAIL_PREFIX`] under `look`'s keyword fold.
fn dress(ctx: &mut Ctx<'_>, room: ObjId, def: &InteriorRoomDef) -> Result<(), String> {
    if ctx.w.get(room).is_some_and(|o| o.name != def.name) {
        ctx.w.set_name(room, &def.name).map_err(|e| e.to_string())?;
    }
    for (attr, text) in [
        ("desc", def.desc.as_str()),
        (mush_atlas::def::SHORT_DESC_ATTR, def.short_desc.as_str()),
    ] {
        if text.trim().is_empty() {
            ctx.w.remove_attr(room, attr).map_err(|e| e.to_string())?;
        } else {
            ctx.w
                .set_attr(room, attr, Attr::text(text, ctx.actor))
                .map_err(|e| e.to_string())?;
        }
    }
    let stale: Vec<String> = ctx
        .w
        .get(room)
        .map(|o| {
            o.attrs
                .iter_namespace("tacsim")
                .map(|(k, _)| k.as_str().to_string())
                .filter(|k| {
                    k.to_ascii_lowercase()
                        .starts_with(&HULL_DETAIL_PREFIX.to_ascii_lowercase())
                })
                .collect()
        })
        .unwrap_or_default();
    for attr in stale {
        ctx.w.remove_attr(room, &attr).map_err(|e| e.to_string())?;
    }
    for (keyword, text) in &def.details {
        let attr = format!(
            "{HULL_DETAIL_PREFIX}{}",
            mush_core::fold_detail_keyword(keyword)
        );
        ctx.w
            .set_attr(room, &attr, Attr::text(text.as_str(), ctx.actor))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// **Re-dress one exit already made** (doc 18 cut IC8, lead 2b(i)): its name set to
/// `spec`'s first segment when it differs, every alias not among the remaining
/// segments removed, and every missing one added in the case authored. A live exit
/// only; a gone one is skipped (the hull's own drift check reports it).
fn redress_exit(ctx: &mut Ctx<'_>, exit: ObjId, spec: &str) -> Result<(), String> {
    let Some(obj) = ctx.w.get(exit) else {
        return Ok(());
    };
    let mut segments = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = segments
        .next()
        .ok_or_else(|| "an exit needs a name".to_string())?;
    let wanted: Vec<&str> = segments.collect();
    let current_name = obj.name.clone();
    let stale: Vec<String> = obj
        .aliases
        .iter()
        .filter(|alias| {
            !wanted
                .iter()
                .any(|w| w.eq_ignore_ascii_case(alias.as_str()))
        })
        .map(ToString::to_string)
        .collect();
    if current_name != name {
        ctx.w.set_name(exit, name).map_err(|e| e.to_string())?;
    }
    for alias in stale {
        ctx.w
            .remove_alias(exit, &alias)
            .map_err(|e| e.to_string())?;
    }
    for alias in wanted {
        // A no-op for an alias the exit already has (case-insensitively).
        ctx.w.add_alias(exit, alias).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One exit `spec` (`"Name <A>;a;alias"`) from `src` to `dest`, in
/// `mush-atlas::apply::open_exit`'s shape (L15): the first segment is the
/// name, every further one an alias.
fn open_exit(ctx: &mut Ctx<'_>, src: ObjId, spec: &str, dest: ObjId) -> Result<ObjId, String> {
    let mut segments = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = segments
        .next()
        .ok_or_else(|| "an exit needs a name".to_string())?;
    let aliases: Vec<String> = segments.map(str::to_string).collect();
    let exit = ctx
        .w
        .create(ObjKind::Exit, name, ctx.actor, Some(src))
        .map_err(|e| e.to_string())?;
    for alias in aliases {
        ctx.w.add_alias(exit, &alias).map_err(|e| e.to_string())?;
    }
    ctx.w
        .link_exit(exit, Some(dest))
        .map_err(|e| e.to_string())?;
    let creator = ctx.actor;
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: exit,
        kind: ObjKind::Exit,
        creator,
    });
    Ok(exit)
}

/// Declare `room` an interior space riding `hull`, the way
/// `mush-atlas::apply::make_interior` declares one (L1): attach `sim.interior`
/// when absent, then `Transition::Space` — only when the space has no record,
/// so a re-apply writes nothing.
fn declare_space(ctx: &mut Ctx<'_>, room: ObjId, hull: ObjId) -> Result<(), String> {
    let recorded = ctx
        .w
        .module_state::<SpatialState>()
        .ok_or_else(|| "the simulation is not composed".to_string())?
        .spaces
        .contains_key(&room);
    if recorded {
        return Ok(());
    }
    let behavior = BehaviorId::new("sim.interior").map_err(|e| e.to_string())?;
    if !ctx.w.has_behavior(room, &behavior) {
        let binding = BehaviorBinding {
            schema_version: BINDING_SCHEMA_VERSION,
            attached_by: ctx.actor,
            attached_at: ctx.w.now(),
        };
        ctx.w
            .attach_behavior(room, behavior, binding)
            .map_err(|e| e.to_string())?;
    }
    mush_sim::run(
        ctx,
        &mush_sim::Transition::Space {
            space: room,
            anchor: SpaceAnchor::MovingObject { host: hull },
        },
    )
    .map_err(|e| e.to_string())
}

/// **The readiness report's hulls line** (L17; cut IC6's gate reads the typed
/// state, never the text): the first failing check, else ready. `None` for an
/// entry with no programme.
pub(crate) fn hull_line(
    w: &World,
    store: &TacsimStore,
    entry: &CanonicalId,
    def: &PortDef,
    map: &PortMap,
) -> Option<(LineState, String)> {
    let programme = def.programme.as_ref()?;
    let pool = programme.pool;
    let org_name = def
        .orgs
        .iter()
        .find(|org| org.id == programme.org)
        .map_or_else(|| programme.org.to_string(), |org| org.name.clone());
    let missing = |text: String| Some((LineState::Missing, format!("{entry} hulls: {text}")));
    let made = map.hulls.iter().filter(|row| row.n < pool).count();
    if made < usize::try_from(pool).unwrap_or(usize::MAX) {
        return missing(format!("{made} of {pool} are made."));
    }
    let interior = template(store, programme)
        .ok()
        .map(|(interior, _)| interior);
    let spatial = w.module_state::<SpatialState>();
    let economy = w.module_state::<EconomyStore>();
    let trust = map
        .orgs
        .get(&programme.org)
        .map(|row| PrincipalId::Organization(OrgId(row.org)));
    // The domains some port map records: "laid up" is `Direct` on one of them.
    // Doc 18 cut IC6 (lead ruling L2): an under-way hull counts as under way
    // (cut IC9's planner path first exercises the arm), a leased one as
    // leased, and neither is Missing — the line reads "the programme was
    // made", never "every hull idle", so the first lease never closes the desk.
    let port_domains: BTreeSet<u64> = store.port_maps().filter_map(|m| m.domain).collect();
    let journeys = w.module_state::<mush_sim::JourneyBook>();
    let mut free = 0_usize;
    let mut under_way = 0_usize;
    // Doc 18 cut IC9 (lead ruling L18): hulls with a filed clearance, read from
    // the voyage pendings (World-only; the ticket is typed state).
    let mut departing = 0_usize;
    let mut leased: Vec<String> = Vec::new();
    for row in &map.hulls {
        let mark = &row.mark;
        let gone = std::iter::once(row.hull)
            .chain(row.rooms.values().copied())
            .chain(row.exits.iter().copied())
            .find(|obj| !live(w, *obj));
        if let Some(obj) = gone {
            return missing(format!("{mark} (#{obj}) does not exist."));
        }
        let complete = interior.is_some_and(|interior| {
            interior
                .rooms
                .iter()
                .all(|room| row.rooms.contains_key(&room.id))
                && row.exits.len() == interior.exits.len().saturating_mul(2)
                && [&interior.space, &interior.store].iter().all(|room| {
                    row.rooms.get(*room).is_some_and(|obj| {
                        spatial.is_some_and(|st| st.spaces.contains_key(&ObjId(*obj)))
                    })
                })
        });
        if !complete {
            return missing(format!("{mark}'s interior is incomplete."));
        }
        let titled = economy
            .and_then(|e| {
                e.snapshot()
                    .asset(ObjId(row.hull))
                    .map(|a| a.title_owner.clone())
            })
            .is_some_and(|owner| Some(owner) == trust);
        if !titled {
            return missing(format!("{mark} is not titled to {org_name}."));
        }
        let hull = ObjId(row.hull);
        let moving = journeys.is_some_and(|book| book.under_way(hull));
        let laid_up = spatial.is_some_and(|st| {
            matches!(st.parent_of(hull),
                SpatialParent::Direct { domain, .. } if port_domains.contains(&domain.0))
        });
        if !moving && !laid_up {
            return missing(format!("{mark} is not laid up at a port."));
        }
        if moving {
            under_way = under_way.saturating_add(1);
        }
        if crate::port::voyage::pending_of(w, hull).is_some() {
            departing = departing.saturating_add(1);
        }
        let lease = trust
            .as_ref()
            .and_then(|trust| crate::port::coop::trust_lease_on(w, trust, hull));
        match lease {
            Some(lease) => {
                // The tenant: the first object party that is not the issuer.
                let tenant = crate::port::coop::lease_tenant(lease).map_or_else(
                    || "nobody".to_owned(),
                    |obj| mush_core::output::display_name(w, obj),
                );
                leased.push(format!("{mark} to {tenant}"));
            }
            // Doc 18 cut IC9 (the IC6 residual): a moving hull is never free.
            None if laid_up && !moving => free = free.saturating_add(1),
            None => {}
        }
    }
    if leased.is_empty() && under_way == 0 && departing == 0 {
        return Some((
            LineState::Ready,
            format!("{entry} hulls: {pool} of {pool} made, titled to {org_name} and laid up."),
        ));
    }
    let leases = if leased.is_empty() {
        String::new()
    } else {
        format!(" ({})", leased.join(", "))
    };
    let under = if under_way > 0 {
        format!(", {under_way} under way")
    } else {
        String::new()
    };
    let leaving = if departing > 0 {
        format!(", {departing} departing")
    } else {
        String::new()
    };
    Some((
        LineState::Ready,
        format!(
            "{entry} hulls: {pool} of {pool} made and titled to {org_name}; {free} laid up and \
             free, {} leased{leases}{under}{leaving}.",
            leased.len()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ic4_registry_marks_are_the_prefix_and_two_digits() {
        assert_eq!(registry_mark("FPC-D-04", 0), "FPC-D-0401");
        assert_eq!(registry_mark("FPC-D-04", 7), "FPC-D-0408");
        assert_eq!(registry_mark("X", 15), "X16");
    }
}
