//! **`+coop/refit prospector|standard`** — the Co-op's yard (doc 20 cut SM.3a;
//! doc 20 §4.3, D-20-7; lead ruling 8 and decision D1 of the SM.3a brief).
//!
//! The refit is **a re-dress in place** (decision D1): the hull keeps its Thing,
//! its mark, its rooms, its hold vault, its lease, its enrolment and its name; the
//! yard makes **one write**, `mush_tactics::module::refit_unit`, which re-binds the
//! hull's `tactics.unit` between the programme unit (the Dray) and a refit unit
//! the latched programme authors (`refits`, lane v10: `prospector` → the Dray-P).
//! The bay itself was laid up on every programme hull by the applier (part f, step
//! 6b), so a player command creates no objects; the hold vault keeps its limits
//! from the definition, so the capacity, the payload and the planner read the
//! Dray-P's 86 t hold through the hull's own unit (`HullView::unit`).
//!
//! The checks run in order and nothing is written on a refusal: the word, the
//! charter or title, the refit offered, the berth, nothing in flight, the hold empty but
//! for her tank (provisions refuse: lead ruling 8), the bay empty, not already
//! that unit, no open bay order and nothing to repair (48e review rounds:G1, G2: the re-dress
//! writes the unit fresh from its design, so a damaged hull is sent to the bay's quote rather
//! than mended for free), at the desk, and the binding and its bay laid up.
//!
//! **Her rounds come with her** (48e review rounds:G2, A-14): the same write carries each space
//! magazine's rounds left onto the new unit (`refit_unit_carrying`), capped at the new mount's
//! stowage, and a mount the new design adds starts empty. A refit is never a rearm.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_inventory::record::EndpointRef;
use mush_tactics::UnitMutationOp;

use crate::definitions::Consumable;
use crate::port::bay_work::magazines;
use crate::port::capacity::{format_m3, format_tonnes};
use crate::port::coop::{COUNTERSIGN, name_of, office_of, port_name, programme_port, say};
use crate::port::def::STANDARD_REFIT;
use crate::port::ship::{Berth, HULL_CLASS_PROSPECTOR, HullView, berth_of, hull_view};
use crate::space::custody;
use crate::store::TacsimStore;

/// The grammar.
pub const REFIT_USAGE: &str = "Usage: +coop/refit prospector|standard";
/// The latched programme authors no such refit.
pub const REFIT_NOT_OFFERED: &str = "The Co-op's yard does not refit Drays on this world.";
/// Under way: `{name}`, `{port}`.
pub const REFIT_UNDER_WAY: &str = "The {name} is under way; the Co-op refits her docked at {port}.";
/// Docked elsewhere, or anywhere else: `{name}`, `{port}`.
pub const REFIT_NOT_HERE: &str = "The Co-op refits the {name} docked at {port}.";
/// Cargo or provisions aboard: `{name}`.
pub const REFIT_HOLD: &str =
    "The {name}'s hold must be empty but for her tank before the yard refits her.";
/// A hold pool that would not read: `{name}`.
pub const REFIT_HOLD_DARK: &str = "The {name}'s hold cannot be read right now; nothing changes.";
/// A rig in the bay: `{n}`, `{name}`.
pub const REFIT_BAY: &str = "Rig {n} is still in the {name}'s bay; return it to Occator first.";
/// A titled or unlisted rig still physically in the bay when sealing it (`P2-TSW-31`): `{name}`.
pub const REFIT_BAY_OCCUPIED: &str =
    "The {name}'s bay still holds a rig; berth or stow it ashore before sealing the bay.";
/// Not a programme hull class (`P2-TSW-31`): `{name}`.
pub const REFIT_WRONG_CLASS: &str =
    "The Co-op's yard refits only Drays; the {name} is another class.";
/// Already that unit: `{name}`, `{class}`.
pub const REFIT_ALREADY: &str = "The {name} is already a {class}.";
/// Away from the desk: `{office}`, `{port}`.
pub const REFIT_WHERE: &str = "Refits are signed at the {office} in {port}.";
/// A core fitted (doc 47 CO.6): `{name}`. The re-dress rebinds her unit, which would take the
/// core's mount with it.
pub const REFIT_CORED: &str =
    "The {name} carries a fitted core; the Co-op's yard re-dresses only a coreless Dray.";
/// An open bay order (48e review rounds:G1): `{name}`. The order works the hull as it stands, and
/// a re-dress under it would strand the goods its draw took.
pub const REFIT_BAY_WORK: &str =
    "The {name} has an open bay order; the yard refits her once it is done or canceled.";
/// Damage or a crit aboard (48e review rounds:G2): `{name}`. The re-dress builds her unit sound,
/// so it would repair her for nothing; the bay repairs her for its fee.
pub const REFIT_DAMAGED: &str =
    "The {name} needs repair before the yard refits her; the bay quotes the work.";
/// A step that should not fail did: `{name}`.
pub const REFIT_FAILED: &str = "The yard could not refit the {name}; nothing changed.";
/// The refit to a Dray-P: `{name}`, `{mark}`, `{tonnes}`, `{m3}`, `{countersign}`.
pub const REFITTED_PROSPECTOR: &str = "The Co-op's yard refits the {name} ({mark}) as a Dray-P: a two-rig vehicle bay aft of the hold, which now takes {tonnes} t and {m3} m³ of bulk or ore. {countersign} countersigns.";
/// The refit back: `{name}`, `{mark}`, `{tonnes}`, `{m3}`, `{countersign}`.
pub const REFITTED_STANDARD: &str = "The Co-op's yard refits the {name} ({mark}) as a standard Dray: the bay is sealed, and the hold takes {tonnes} t and {m3} m³ again. {countersign} countersigns.";

/// `{class}` for the two units.
const CLASS_STANDARD: &str = "standard Dray";

/// Everything the refit resolved before its one write.
struct RefitPlan {
    hull: ObjId,
    name: String,
    mark: String,
    target: CanonicalId,
    standard: bool,
    spec: mush_tactics::UnitSpec,
    tonnes: String,
    m3: String,
}

/// The target unit's cargo limits as the reply prints them: the compartment that
/// takes a trade good, its kilograms as whole tonnes and its litres as cubic
/// metres to one decimal, truncated (the load cells' render).
fn cargo_words(store: &TacsimStore, unit: &CanonicalId) -> Option<(String, String)> {
    let goods = store.goods()?;
    let logistics = store.definitions().specs.get(unit)?.logistics.as_ref()?;
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    let volume = CanonicalId::new(crate::definitions::logistics::VOLUME_L).ok()?;
    let cargo = logistics.stowage.values().find(|compartment| {
        goods
            .goods
            .iter()
            .any(|good| good.consumable.is_none() && compartment.accepts.contains(&good.stowage))
    })?;
    Some((
        format_tonnes(*cargo.limits.get(&mass)?),
        format_m3(*cargo.limits.get(&volume)?),
    ))
}

/// **The refit's checks, in order**, read-only. `Err` carries the refusal.
#[allow(clippy::too_many_lines)] // one ordered check list
fn plan_refit(ctx: &Ctx<'_>) -> Result<RefitPlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let port = programme_port(w).ok_or_else(|| crate::port::coop::NOT_OPEN_HERE.to_owned())?;
    // 1. The word.
    let mut words = ctx
        .args
        .raw()
        .split_whitespace()
        .map(str::to_ascii_lowercase);
    let word = match (words.next(), words.next()) {
        (Some(word), None) => word,
        _ => return Err(REFIT_USAGE.to_owned()),
    };
    let authored = port.programme.refits.keys().any(|key| key.as_str() == word);
    if word != STANDARD_REFIT && word != "prospector" && !authored {
        return Err(REFIT_USAGE.to_owned());
    }
    // 2. The current charter or title, never graduation alone.
    let hull = crate::port::coop::mining_hull(ctx)?;
    let name = name_of(w, hull);
    let refused = |template: &str| template.replace("{name}", &name);
    // 3. The latched programme authors the refit (and so a way back).
    if port.programme.refits.is_empty() || (word != STANDARD_REFIT && !authored) {
        return Err(REFIT_NOT_OFFERED.to_owned());
    }
    let standard = word == STANDARD_REFIT;
    let target = if standard {
        port.programme.unit.clone()
    } else {
        port.programme
            .refits
            .iter()
            .find(|(key, _)| key.as_str() == word)
            .map(|(_, unit)| unit.clone())
            .ok_or_else(|| REFIT_NOT_OFFERED.to_owned())?
    };
    // 4. The berth.
    let port_label = port_name(port.def);
    match berth_of(w, hull) {
        Berth::UnderWay => {
            return Err(refused(REFIT_UNDER_WAY).replace("{port}", &port_label));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Err(refused(REFIT_NOT_HERE).replace("{port}", &port_label));
        }
    }
    // Doc 44 unit MW.13: a mind sealed aboard holds her as she is. The unit's owner would
    // refuse the re-dress; the yard says why first.
    if crate::port::ship_mind::holds(ctx, hull) {
        return Err(refused(crate::port::ship_mind::MIND_HOLDS_HER));
    }
    // Doc 47 CO.6: a Dray the fitting shop cored keeps her core; the re-dress would drop it.
    if mush_tactics::module::unit_of(w, hull)
        .and_then(crate::arena::core::read_core)
        .is_some()
    {
        return Err(refused(REFIT_CORED));
    }
    let view: HullView<'_> = hull_view(w, hull).ok_or_else(|| refused(REFIT_FAILED))?;
    let vault =
        crate::port::coop::mining_hold(w, actor, &view).ok_or_else(|| refused(REFIT_HOLD_DARK))?;
    // 5. Nothing in flight, against this hull's actual hold and current pilot.
    if let Some((_, line)) = crate::port::buyout::in_flight_on(w, hull, actor, vault, &name) {
        return Err(line);
    }
    // 6. The hold: every good but the propellant reads 0 (provisions refuse, lead
    // ruling 8).
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| crate::port::coop::NOT_OPEN_HERE.to_owned())?;
    let goods = store.goods().ok_or_else(|| refused(REFIT_HOLD_DARK))?;
    let hold = EndpointRef(vault);
    for good in goods
        .goods
        .iter()
        .filter(|good| good.consumable != Some(Consumable::Propellant))
    {
        match crate::port_tick::read_held(ctx, hold, &good.id) {
            Ok(0) => {}
            Ok(_) => return Err(refused(REFIT_HOLD)),
            Err(error) => {
                tracing::warn!(hull = hull.0, good = %good.id, ?error, "tacsim: +coop/refit could not read the hold");
                return Err(refused(REFIT_HOLD_DARK));
            }
        }
    }
    // 7. The bay is empty, and no live lease record names the hull (reviewer finding
    // correctness/F2: a record whose rig is out of the bay still holds the refit, so a
    // `standard` refit never strands a live lease behind `NO_BAY`).
    let failed = |what: &str| {
        tracing::error!(
            what,
            hull = hull.0,
            pilot = actor.0,
            "tacsim: +coop/refit could not resolve a step"
        );
        refused(REFIT_FAILED)
    };
    if let Some(n) = crate::port::tv::bay_slot_word(w, &view) {
        return Err(refused(REFIT_BAY).replace("{n}", &n));
    }
    // P2-TSW-31: for bayless targets (`standard`), also require the physical bay to be completely
    // empty including titled rigs (which `bay_slot_word` intentionally excludes).
    let target_bay_cap: usize = store
        .definitions()
        .specs
        .get(&target)
        .and_then(|spec| spec.interior.as_ref())
        .map(|interior| {
            interior
                .bays
                .iter()
                .map(|bay| usize::try_from(bay.slot.maximum_items).unwrap_or(usize::MAX))
                .fold(0usize, usize::saturating_add)
        })
        .unwrap_or(0);
    if crate::port::vehicle::rigs_in_bay(w, &view).len() > target_bay_cap {
        return Err(refused(REFIT_BAY_OCCUPIED));
    }
    // 8. Not already the target (a held record answers at step 10's REFIT_FAILED,
    // never "already": reviewer F6).
    if view.unit_held {
        return Err(failed("the hull's unit record is held"));
    }
    // P2-TSW-31: only hulls belonging to the programme's own unit family (base or an authored
    // refit) may be re-dressed here, preventing irreversible overwrites of other commissioned
    // classes (such as `tycho_lifter`).
    if view.unit != port.programme.unit && !port.programme.refits.values().any(|u| u == &view.unit)
    {
        return Err(refused(REFIT_WRONG_CLASS));
    }
    if view.unit == target {
        let class = if standard {
            CLASS_STANDARD
        } else {
            HULL_CLASS_PROSPECTOR
        };
        return Err(refused(REFIT_ALREADY).replace("{class}", class));
    }
    // 8b. (48e review rounds:G1) An open bay order holds the hull as it stands; a re-dress
    // under it would cancel the order after its draw had taken the goods.
    if crate::port::bay_work::under_work(w, hull) {
        return Err(refused(REFIT_BAY_WORK));
    }
    // 48e review rounds:G2: the re-dress builds her unit sound, so a hull the bay would repair is
    // refused: the yard repairs nothing for free. (Her magazines are carried across, at
    // `refit_cmd`.)
    match crate::port::bay_work::needs_repair_here(w, hull) {
        Ok(false) => {}
        Ok(true) => return Err(refused(REFIT_DAMAGED)),
        Err(error) => {
            tracing::error!(hull = hull.0, %error, "tacsim: +coop/refit could not read the hull's repair");
            return Err(refused(REFIT_FAILED));
        }
    }
    // 9. At the desk.
    let (office, office_name, port_label) = office_of(w, &port);
    let here = w.get(actor).and_then(|o| o.location);
    if office.is_none() || here != office {
        return Err(REFIT_WHERE
            .replace("{office}", &office_name)
            .replace("{port}", &port_label));
    }
    // 10. The binding, the compatibility and, for a target with bays, the bay laid up.
    let binding = store
        .definitions()
        .bindings
        .get(&target)
        .ok_or_else(|| failed("the target unit's binding"))?;
    let (Some(base), Some(current), Some(spec)) = (
        store.definitions().specs.get(&port.programme.unit),
        store.definitions().specs.get(&view.unit),
        store.definitions().specs.get(&target),
    ) else {
        return Err(failed("the units' specs"));
    };
    if view.unit != port.programme.unit {
        crate::port::hull::refit_compatible(base, current).map_err(|why| failed(&why))?;
    }
    if !standard {
        crate::port::hull::refit_compatible(base, spec).map_err(|why| failed(&why))?;
    }
    if let Some(interior) = &spec.interior {
        for bay in &interior.bays {
            if !crate::port::hull::bay_laid_up(w, view.row, bay) {
                return Err(failed("the bay is not laid up"));
            }
        }
    }
    let (tonnes, m3) = cargo_words(store, &target).ok_or_else(|| failed("the target's hold"))?;
    Ok(RefitPlan {
        hull,
        name,
        mark: view.row.mark.clone(),
        target,
        standard,
        spec: binding.spec.clone(),
        tonnes,
        m3,
    })
}

/// **`+coop/refit prospector|standard`** (doc 20 §4.3, D-20-7): every check, then
/// one write — the unit re-bound in place.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn refit_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let plan = plan_refit(ctx).map_err(CmdError::msg)?;
    // 48e review rounds:G2 (A-14: a refit conserves rounds): each magazine of the new fit holds
    // what the same mount held before the re-dress, at most what it stows, and a mount the old
    // fit did not carry comes empty. Carried in the re-bind's one write.
    let kept: BTreeMap<String, u32> = mush_tactics::module::unit_of(ctx.w, plan.hull)
        .map(|unit| {
            magazines::magazines(unit)
                .into_iter()
                .map(|magazine| (magazine.mount, magazine.left))
                .collect()
        })
        .unwrap_or_default();
    let carry = |fresh: &mush_tactics::TacticalUnit| -> Result<Vec<UnitMutationOp>, String> {
        let short: BTreeMap<String, u32> = magazines::magazines(fresh)
            .into_iter()
            .filter_map(|magazine| {
                let left = kept
                    .get(&magazine.mount)
                    .copied()
                    .unwrap_or(0)
                    .min(magazine.stow);
                (left < magazine.stow).then_some((magazine.mount, left))
            })
            .collect();
        Ok(custody::magazine_op(fresh, &short)
            .map_err(|error| error.to_string())?
            .into_iter()
            .collect())
    };
    if let Err(error) =
        mush_tactics::module::refit_unit_carrying(ctx, plan.hull, &plan.spec, &carry)
    {
        tracing::error!(
            hull = plan.hull.0,
            target = %plan.target,
            %error,
            "tacsim: +coop/refit: the unit was not re-bound"
        );
        return Err(CmdError::msg(REFIT_FAILED.replace("{name}", &plan.name)));
    }
    let template = if plan.standard {
        REFITTED_STANDARD
    } else {
        REFITTED_PROSPECTOR
    };
    say(
        ctx,
        template
            .replace("{name}", &plan.name)
            .replace("{mark}", &plan.mark)
            .replace("{tonnes}", &plan.tonnes)
            .replace("{m3}", &plan.m3)
            .replace("{countersign}", COUNTERSIGN),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **P2-TSW-31**: non-programme hull classes (`tycho_lifter`) are neither in the programme
    /// refit family nor `refit_compatible` with the base Dray, and bayless `standard` refits
    /// inspect `vehicle::rigs_in_bay` directly so titled rigs inside `"bay"` cannot be smuggled
    /// or stranded behind `bay_slot_word`.
    #[test]
    fn p2_tsw_31_non_programme_class_and_titled_rig_in_bay_are_blocked() {
        let catalog =
            crate::definitions::Catalog::build_fielded(&[], crate::NS).expect("fielded catalog");
        let dray = CanonicalId::new("tacsim.unit.dray").expect("dray");
        let dray_p = CanonicalId::new("tacsim.unit.dray_p").expect("dray_p");
        let lifter = CanonicalId::new("tacsim.unit.tycho_lifter").expect("lifter");
        let base_spec = catalog.specs.get(&dray).expect("dray spec");
        let dray_p_spec = catalog.specs.get(&dray_p).expect("dray_p spec");
        let lifter_spec = catalog.specs.get(&lifter).expect("lifter spec");

        // Programme refit family: dray <-> dray_p is compatible; tycho_lifter is rejected.
        assert!(crate::port::hull::refit_compatible(base_spec, dray_p_spec).is_ok());
        assert!(
            crate::port::hull::refit_compatible(base_spec, lifter_spec).is_err(),
            "tycho_lifter must fail refit_compatible against dray"
        );
        let target_bay_cap = base_spec
            .interior
            .as_ref()
            .map(|interior| {
                interior
                    .bays
                    .iter()
                    .map(|bay| usize::try_from(bay.slot.maximum_items).unwrap_or(usize::MAX))
                    .fold(0usize, usize::saturating_add)
            })
            .unwrap_or(0);
        assert_eq!(target_bay_cap, 0, "standard Dray has zero bay capacity");

        // A titled rig sitting in a Prospector Dray's bay has no TvLease (`bay_slot_word` is
        // `None`), yet `vehicle::rigs_in_bay` finds it and exceeds `target_bay_cap` (0).
        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let carrier = tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                "DrayP",
                mush_core::ObjId(1),
                None,
            )
            .expect("carrier");
        let bay_room = tw
            .world_mut()
            .create(mush_core::ObjKind::Room, "Bay", mush_core::ObjId(1), None)
            .expect("bay");
        let titled_rig = tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                "TitledRig",
                mush_core::ObjId(1),
                None,
            )
            .expect("rig");
        let lane_id = CanonicalId::new(crate::definitions::content::LANE_PACK).expect("lane id");
        let ceres_entry = CanonicalId::new("ceres_port").expect("ceres");
        let lane_pack =
            crate::pack::load_pack(crate::definitions::content::LANE_JSON).expect("lane pack");
        let lane_ver = lane_pack.manifest().version();
        let row = crate::port::HullRow {
            n: 1,
            mark: "FPC-P-01".to_owned(),
            hull: carrier.0,
            rooms: BTreeMap::from([(CanonicalId::new("bay").expect("bay id"), bay_room.0)]),
            exits: Vec::new(),
        };
        {
            let store = tw
                .world_mut()
                .module_state_mut::<TacsimStore>()
                .expect("store");
            store.cfg.enabled = true;
            store
                .stage_pack(crate::LoadedPack {
                    source: "<test>".to_owned(),
                    pack: lane_pack,
                })
                .expect("stage");
            store.seeded_keys.insert((lane_id.clone(), lane_ver));
            store.applied.insert(lane_id.clone(), lane_ver);
            store.port_maps.insert(
                (lane_id.clone(), ceres_entry.clone()),
                crate::port::PortMap::new(lane_id, ceres_entry.clone()),
            );
            store.yard.owned.insert(
                carrier,
                crate::port::owned::Held::Present(Box::new(crate::port::owned::OwnedHull {
                    hull: carrier,
                    design: dray_p,
                    mark: "FPC-P-01".to_owned(),
                    home: ceres_entry.clone(),
                    provenance: 1,
                    body: crate::port::owned::OwnedBody::Ship(row),
                })),
            );
            store.yard.owned.insert(
                titled_rig,
                crate::port::owned::Held::Present(Box::new(crate::port::owned::OwnedHull {
                    hull: titled_rig,
                    design: CanonicalId::new("tacsim.unit.tv_prospector").expect("rig unit"),
                    mark: "TIT-TV-01".to_owned(),
                    home: ceres_entry,
                    provenance: 2,
                    body: crate::port::owned::OwnedBody::Rig(crate::port::RigRow {
                        n: 1,
                        mark: "TIT-TV-01".to_owned(),
                        tv: titled_rig.0,
                        rooms: BTreeMap::new(),
                        vault: None,
                    }),
                })),
            );
        }
        {
            let rigs_slot = mush_sim::SlotId::new("rigs").expect("slot");
            let engine = tw.engine_mut();
            let mut out = mush_core::Outbox::default();
            let mut ctx = Ctx {
                w: &mut engine.world,
                reg: &engine.registry,
                out: &mut out,
                actor: mush_core::GOD,
                args: Default::default(),
                switches: Default::default(),
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            crate::port::hull::declare_space(
                &mut ctx,
                bay_room,
                mush_sim::SpaceAnchor::MovingObject { host: carrier },
            )
            .expect("bay space");
            mush_sim::declare_compartment(&mut ctx, bay_room, rigs_slot.clone(), Some(1))
                .expect("rigs compartment");
            mush_sim::place_interior(&mut ctx, titled_rig, bay_room, Some(rigs_slot))
                .expect("place titled rig in bay");
        }
        let view = hull_view(tw.world(), carrier).expect("view");
        assert!(
            crate::port::tv::bay_slot_word(tw.world(), &view).is_none(),
            "titled rig without TvLease is invisible to bay_slot_word"
        );
        let rigs = crate::port::vehicle::rigs_in_bay(tw.world(), &view);
        assert_eq!(
            rigs.iter().map(|r| r.tv).collect::<Vec<_>>(),
            vec![titled_rig]
        );
        assert!(rigs.len() > target_bay_cap);
    }
}
