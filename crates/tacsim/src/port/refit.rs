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
//! charter, the refit offered, the berth, nothing in flight, the hold empty but
//! for her tank (provisions refuse: lead ruling 8), the bay empty, not already
//! that unit, at the desk, and the binding and its bay laid up.

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_inventory::record::EndpointRef;

use crate::definitions::Consumable;
use crate::port::capacity::{format_m3, format_tonnes};
use crate::port::coop::{COUNTERSIGN, name_of, office_of, port_name, programme_port, say};
use crate::port::def::STANDARD_REFIT;
use crate::port::ship::{Berth, HULL_CLASS_PROSPECTOR, HullView, berth_of, hull_view};
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
/// Already that unit: `{name}`, `{class}`.
pub const REFIT_ALREADY: &str = "The {name} is already a {class}.";
/// Away from the desk: `{office}`, `{port}`.
pub const REFIT_WHERE: &str = "Refits are signed at the {office} in {port}.";
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
    // 2. The charter.
    let e = crate::port::recall::chartered(ctx)?;
    let hull = e.hull;
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
    // 5. Nothing in flight.
    if let Some((_, line)) = crate::port::buyout::in_flight(w, &e) {
        return Err(line);
    }
    // 6. The hold: every good but the propellant reads 0 (provisions refuse, lead
    // ruling 8).
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| crate::port::coop::NOT_OPEN_HERE.to_owned())?;
    let goods = store.goods().ok_or_else(|| refused(REFIT_HOLD_DARK))?;
    let hold = EndpointRef(mush_economy::VaultId(e.vault));
    for good in goods
        .goods
        .iter()
        .filter(|good| good.consumable != Some(Consumable::Propellant))
    {
        match crate::port_tick::read_pool(ctx, hold, &good.id) {
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
    let view: HullView<'_> = hull_view(w, hull).ok_or_else(|| failed("the hull's view"))?;
    if let Some(n) = crate::port::tv::bay_slot_word(w, &view) {
        return Err(refused(REFIT_BAY).replace("{n}", &n));
    }
    // 8. Not already the target (a held record answers at step 10's REFIT_FAILED,
    // never "already": reviewer F6).
    if view.unit_held {
        return Err(failed("the hull's unit record is held"));
    }
    if view.unit == target {
        let class = if standard {
            CLASS_STANDARD
        } else {
            HULL_CLASS_PROSPECTOR
        };
        return Err(refused(REFIT_ALREADY).replace("{class}", class));
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
    let (Some(base), Some(spec)) = (
        store.definitions().specs.get(&port.programme.unit),
        store.definitions().specs.get(&target),
    ) else {
        return Err(failed("the units' specs"));
    };
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
        mark: e.mark.clone(),
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
    if let Err(error) = mush_tactics::module::refit_unit(ctx, plan.hull, &plan.spec) {
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
