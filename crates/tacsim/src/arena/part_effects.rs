//! **What a hull's parts do while jammed or damaged** (doc 40 cut FR.5, §4.1 item 4; Q-40-2 to
//! Q-40-6, ruled 2026-11-03 as proposed).
//!
//! One function, [`Arena::apply_part_effects`], derives a unit's live rows from its parts'
//! condition words (`mush_tactics::field_ops::ComponentCondition::classify_condition_id`): once
//! at admission (`Arena::admit_unit`, which serves the check-out and `Arena::join` alike, so a
//! condition the bay left is honoured for every kind) and once after a round's part op lands
//! (`Arena::apply_hit`). The numbers are the unit's [`ComponentEffects`]
//! ([`crate::arena::resolution::UnitProfile::effects`]); the draw-label set is unchanged, since
//! the effect is a function of the word and not a second roll.
//!
//! **Jammed is off until unjammed; damaged is degraded until repaired** (Q-40-3):
//!
//! - **the sight** — jammed, `UnitState::sensor` is `None` (no detection and no lock; its
//!   contacts age out through `Arena::sensor_pass`'s loss band, never cleared at once); damaged,
//!   `max_range_cells` at `sensor.damaged_range_permille` of the **authored** row. The authored
//!   row is on no cell, so `UnitState::sensor_authored` keeps it while the live row differs and
//!   every derivation starts from it: a second derivation never compounds;
//! - **the pod and the filter** — jammed (and, by the ruling, damaged), `jammer` / `eccm` is
//!   `None`; restored, each is re-read from its cells (`Jammer::read`, `Eccm::read`), and the
//!   field's widest bubble (`Arena::jam_radius_cm`) is taken again;
//! - **the plant** — jammed, `Power::Off` and a scheduled `StartupComplete` taken off the heap;
//!   `crate::arena::intents::resolve_startup` refuses a plant jammed or destroyed by name
//!   (`Refusal::Plant`, Q-40-4); damaged, the heat ceiling and the dissipation at
//!   `power.damaged_heat_permille`, read-side (`UnitState::heat_max`, `UnitState::heat_per_s`),
//!   the cell untouched;
//! - **the core** — jammed or damaged, its effective class below the cell's, derived read-side
//!   by `crate::arena::core::read_core_under` and never written to the `core_class` cell; the
//!   condition row is cut to the class's slots here, as `degrade_core` cuts it;
//! - **the mount** — GC.2's rule, retired into this derivation (Q-40-6): a gun whose part reads
//!   jammed, destroyed or missing is off the fire table, its rounds kept ([`part_works`]);
//! - **the relay** — the host's (Q-40-5): `crate::field::orders` holds or delays a party's
//!   delivered orders by its carrier's relay word; no kernel row.
//!
//! A wreck is past all of this: its sight, guns and power went with `Arena::destroy_unit`, and
//! nothing here restores them.

use mush_core::ObjId;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{CellAddr, CellId, ComponentId, TacticalUnit};

use crate::arena::perception::{Eccm, Jammer, SensorProfile};
use crate::arena::resolution::{ComponentEffects, PodEffect};
use crate::arena::state::{Arena, DueKind, Power, UnitState};

/// **The condition word of part `id` on `unit`, classified**, or `None` for a part the unit does
/// not hold (a hand-made rig's sight or gun).
#[must_use]
pub fn part_condition(unit: &TacticalUnit, id: &str) -> Option<ComponentCondition> {
    let id = ComponentId::new(id).ok()?;
    unit.component(&id)
        .map(|part| ComponentCondition::classify_condition_id(&part.condition))
}

/// **Whether the part `id` on `unit` can work** (doc 36 cut GC.2, Q-36-38; retired here by doc
/// 40 cut FR.5, Q-40-6): its record's condition word reads intact or damaged. A part the unit
/// does not hold (a hand-made rig's gun) works.
#[must_use]
pub fn part_works(unit: &TacticalUnit, id: &str) -> bool {
    part_condition(unit, id).is_none_or(ComponentCondition::is_operational)
}

/// **The condition of the hull's power plant** (doc 40 cut FR.5, Q-40-4): the part the
/// `tacsim.power` capability names as its source, or `None` for a hull whose capability names
/// none (every hand-made rig) or that holds no plant at all (a wreck's was revoked).
#[must_use]
pub fn plant_condition(unit: &TacticalUnit) -> Option<ComponentCondition> {
    capability_part(unit, crate::arena::damage::POWER_CAPABILITY)
        .and_then(|part| unit.component(part))
        .map(|part| ComponentCondition::classify_condition_id(&part.condition))
}

/// **The condition of the hull's relay** (doc 40 cut FR.5, Q-40-5): the part the
/// `tacsim.relay.mobile` capability names as its source, or `None` for a hull with no relay —
/// every hull but the Halcyon.
#[must_use]
pub fn relay_condition(unit: &TacticalUnit) -> Option<ComponentCondition> {
    capability_part(unit, RELAY_CAPABILITY)
        .and_then(|part| unit.component(part))
        .map(|part| ComponentCondition::classify_condition_id(&part.condition))
}

/// The capability a relay part grants (`crate::definitions::unit`'s bind of the relay role).
pub const RELAY_CAPABILITY: &str = "tacsim.relay.mobile";

/// The part that sources the first capability `id` the unit holds.
fn capability_part<'a>(unit: &'a TacticalUnit, id: &str) -> Option<&'a ComponentId> {
    unit.capabilities()
        .find(|key| key.id.as_str() == id)
        .and_then(|key| key.source.as_ref())
}

/// **The part a pod or a filter is read off**: the first component, in component order, that
/// carries `cell` and whose `condition` cell is above nought — the one [`Jammer::read`] and
/// [`Eccm::read`] find.
fn mounted_part(unit: &TacticalUnit, cell: &str) -> Option<ComponentId> {
    let (Ok(cell), Ok(condition)) = (
        CellId::new(cell),
        CellId::new(crate::definitions::unit::CONDITION_CELL),
    ) else {
        return None;
    };
    unit.components().find_map(|(component, _)| {
        let at = |cell: &CellId| {
            unit.cell(&CellAddr::Component {
                component: component.clone(),
                cell: cell.clone(),
            })
        };
        at(&cell)?;
        (at(&condition)?.current > 0).then(|| component.clone())
    })
}

/// Whether a pod or filter in `condition` is off under `row`.
fn pod_off(condition: Option<ComponentCondition>, row: PodEffect) -> bool {
    match condition {
        Some(ComponentCondition::Jammed) => true,
        Some(ComponentCondition::Damaged) => row.damaged_off,
        _ => false,
    }
}

/// `value` at `permille` of itself, rounded down.
fn scaled(value: u16, permille: u16) -> u16 {
    if permille >= 1_000 {
        return value;
    }
    u16::try_from(u32::from(value) * u32::from(permille) / 1_000).unwrap_or(value)
}

/// **What one derivation changed that the arena itself must finish** (the unit's own rows are
/// written in place).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Derived {
    /// The plant is jammed: a scheduled startup must not land.
    pub plant_off: bool,
    /// The core's part reads jammed or damaged: its class is below the cell's.
    pub core_down: bool,
}

/// **Derive one unit's live rows from its parts' condition words** (this module's head), in
/// place. `cell_cm` is the window's, for the pod's bubble. A wreck derives nothing.
pub(crate) fn derive(state: &mut UnitState, cell_cm: u16) -> Derived {
    if state.limits.wreck {
        return Derived::default();
    }
    let effects: ComponentEffects = state.profile.effects();
    // The sight, always from the authored row.
    if let Some(authored) = state
        .sensor_authored
        .take()
        .or_else(|| state.sensor.clone())
    {
        let live = match part_condition(&state.unit, &authored.component) {
            Some(ComponentCondition::Jammed) => None,
            Some(ComponentCondition::Damaged) => Some(SensorProfile {
                max_range_cells: scaled(
                    authored.max_range_cells,
                    effects.sensor.damaged_range_permille,
                ),
                ..authored.clone()
            }),
            _ => Some(authored.clone()),
        };
        state.sensor_authored = (live.as_ref() != Some(&authored)).then_some(authored);
        state.sensor = live;
    }
    // The pod and the filter, from their cells.
    let ecm = mounted_part(&state.unit, crate::definitions::unit::JAM_RADIUS_CELL)
        .and_then(|part| part_condition(&state.unit, part.as_str()));
    state.jammer = if pod_off(ecm, effects.ecm) {
        None
    } else {
        Jammer::read(&state.unit, cell_cm)
    };
    let eccm = mounted_part(&state.unit, crate::definitions::unit::ECCM_ATTENUATION_CELL)
        .and_then(|part| part_condition(&state.unit, part.as_str()));
    state.eccm = if pod_off(eccm, effects.eccm) {
        None
    } else {
        Eccm::read(&state.unit)
    };
    // The plant: off while jammed. Unjamming restores STARTUP, never the power.
    let plant_off = plant_condition(&state.unit) == Some(ComponentCondition::Jammed);
    if plant_off {
        state.power = Power::Off;
    }
    // The core: read-side (`read_core_under`); the slots follow in the arena.
    let core_down = crate::arena::core::read_core(&state.unit)
        .and_then(|core| part_condition(&state.unit, core.component.as_str()))
        .is_some_and(|condition| {
            matches!(
                condition,
                ComponentCondition::Jammed | ComponentCondition::Damaged
            )
        });
    Derived {
        plant_off,
        core_down,
    }
}

impl Arena {
    /// **Derive `unit`'s live rows from its parts' condition words** (doc 40 cut FR.5, §4.1
    /// item 4, Q-40-3; this module's head): [`derive`] over the row, then the arena's half —
    /// a gun whose part no longer works off the fire table (GC.2's rule, Q-40-6), a jammed
    /// plant's scheduled `StartupComplete` off the heap (the retain `Arena::destroy_unit` runs,
    /// Q-40-4), a lowered core's condition row cut to its slots (`Arena::truncate_slots`), and
    /// the field's widest bubble and the jamming memo taken again. Idempotent: a second call
    /// over the same words changes nothing.
    pub(crate) fn apply_part_effects(&mut self, unit: ObjId) {
        let cell_cm = self.definition.terrain.cell_cm();
        let Some(state) = self.unit_mut(unit) else {
            return;
        };
        let derived = derive(state, cell_cm);
        let idle: Vec<smol_str::SmolStr> = state
            .weapons
            .iter()
            .filter(|weapon| !part_works(&state.unit, &weapon.id))
            .map(|weapon| weapon.id.clone())
            .collect();
        if !idle.is_empty() {
            self.drop_weapons(unit, |weapon| idle.contains(&weapon.id));
        }
        if derived.plant_off {
            self.due.retain(
                |kind| !matches!(kind, DueKind::StartupComplete { unit: due } if *due == unit),
            );
        }
        if derived.core_down {
            self.truncate_slots(unit);
        }
        self.jam_radius_cm = crate::arena::state::jam_radius_cm(&self.units);
        self.jam_fold.clear();
    }
}

#[cfg(test)]
mod tests {
    use mush_tactics::{
        CapabilityId, CapabilityInit, CapabilityKey, CellSpec, ComponentSpec, ConditionId,
        InstallPoint, SectionId, UnitMutationOp, UnitMutationPlan,
    };

    use super::*;
    use crate::arena::core::CoreClass;
    use crate::arena::intents::{Refusal, resolve_startup};
    use crate::arena::state::{CustodyUnit, PartyId};
    use crate::arena::test_support::{
        arena_of, dref, pose, rules, tactical_unit_spec, tank, visual, with_core,
    };

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// A one-section hull whose plant (`plant`, the `tacsim.power` source), sight
    /// (`visual_sensor`) and Class II core (`core`) read `plant`, `sight` and `core`.
    fn hull(id: ObjId, plant: &str, sight: &str, core: &str) -> TacticalUnit {
        let part = |name: &str, word: &str, cells: Vec<CellSpec>| ComponentSpec {
            id: ComponentId::new(name).expect("a component"),
            definition: dref("component", &format!("tacsim.unit.line_tank.{name}")),
            install: InstallPoint::Section(SectionId::new("hull").expect("a section")),
            condition: ConditionId::new(word).expect("a condition"),
            cells,
            asset: None,
        };
        let mut spec = tactical_unit_spec(rules());
        spec.components.push(part("plant", plant, Vec::new()));
        spec.components.push(part(
            "visual_sensor",
            sight,
            vec![CellSpec {
                id: CellId::new("condition").expect("a cell"),
                min: 0,
                max: 100,
                init: 100,
            }],
        ));
        spec.capabilities.push(CapabilityInit {
            key: CapabilityKey {
                id: CapabilityId::new("tacsim.power").expect("a capability"),
                source: Some(ComponentId::new("plant").expect("a component")),
            },
            rules: rules(),
        });
        with_core(&mut spec, "hull", CoreClass::II);
        for component in &mut spec.components {
            if component.id.as_str() == "core" {
                component.condition = ConditionId::new(core).expect("a condition");
            }
        }
        TacticalUnit::from_spec(&spec, id).expect("the hull")
    }

    /// [`hull`] checked out on the stock sight.
    fn unit(id: ObjId, team: PartyId, plant: &str, sight: &str, core: &str) -> CustodyUnit {
        CustodyUnit {
            sensor: Some(visual()),
            unit: hull(id, plant, sight, core),
            ..tank(
                id,
                team,
                if id == RED {
                    pose(1_500, 1_500)
                } else {
                    pose(1_500, 2_500)
                },
            )
        }
        .sealed()
    }

    /// Set part `id` on `unit` to `word` through M32, as a round's op does.
    fn set_word(arena: &mut Arena, unit: ObjId, id: &str, word: &str) {
        let state = arena.unit(unit).expect("the unit");
        let plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetComponentCondition {
                component: ComponentId::new(id).expect("a component"),
                condition: ConditionId::new(word).expect("a condition"),
            }],
            None,
            None,
        )
        .expect("a plan");
        let (post, _) = mush_tactics::apply(&state.unit, &plan, 0).expect("it applies");
        arena.set_unit(unit, post, false);
    }

    /// **Q-40-3's sensor row at admission and mid-match**: a sight the record calls damaged
    /// checks out at 500 ‰ of its authored range, the authored row kept beside it; a second
    /// derivation does not compound; a jam takes the sight away; the repair's word gives the
    /// authored row back and drops the kept one.
    #[test]
    fn fr5_a_damaged_sight_sees_half_as_far_and_never_compounds() {
        let authored = visual();
        let mut arena = arena_of(vec![unit(RED, PartyId::RED, "intact", "damaged", "intact")]);
        let read = |arena: &Arena| {
            let state = arena.unit(RED).expect("red");
            (
                state.sensor.as_ref().map(|sensor| sensor.max_range_cells),
                state.sensor_authored.clone(),
            )
        };
        assert_eq!(
            read(&arena),
            (Some(authored.max_range_cells / 2), Some(authored.clone())),
            "half the authored range, the authored row kept"
        );
        arena.apply_part_effects(RED);
        arena.apply_part_effects(RED);
        assert_eq!(
            read(&arena),
            (Some(authored.max_range_cells / 2), Some(authored.clone())),
            "a second derivation starts from the authored row"
        );
        assert_eq!(
            arena
                .unit(RED)
                .and_then(|state| state.sensor.as_ref())
                .map(|sensor| sensor.share_range_cells),
            Some(authored.share_range_cells),
            "the comms reach is untouched"
        );
        set_word(&mut arena, RED, "visual_sensor", "jammed");
        arena.apply_part_effects(RED);
        assert_eq!(
            read(&arena),
            (None, Some(authored.clone())),
            "jammed: no sight"
        );
        set_word(&mut arena, RED, "visual_sensor", "intact");
        arena.apply_part_effects(RED);
        assert_eq!(
            read(&arena),
            (Some(authored.max_range_cells), None),
            "restored: the authored row, nothing kept"
        );
    }

    /// **Q-40-4's jammed plant**: a startup already scheduled is taken off the heap as the plant
    /// jams, the power is off, and STARTUP is refused by name; a destroyed plant refuses too, and
    /// an unjammed one restores STARTUP and not the power.
    #[test]
    fn fr5_a_jammed_plant_is_off_and_refuses_startup_by_name() {
        let mut arena = arena_of(vec![
            unit(RED, PartyId::RED, "intact", "intact", "intact"),
            unit(BLUE, PartyId::BLUE, "intact", "intact", "intact"),
        ]);
        let pending = |arena: &Arena| {
            arena
                .due
                .iter()
                .filter(|due| matches!(due.kind, DueKind::StartupComplete { unit } if unit == RED))
                .count()
        };
        assert_eq!(resolve_startup(&mut arena, RED, 0), Ok(()));
        assert_eq!(resolve_startup(&mut arena, BLUE, 0), Ok(()));
        assert_eq!(pending(&arena), 1, "red's startup is scheduled");
        set_word(&mut arena, RED, "plant", "jammed");
        arena.apply_part_effects(RED);
        assert_eq!(pending(&arena), 0, "the scheduled startup is gone");
        assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
        assert!(
            arena
                .due
                .iter()
                .any(|due| matches!(due.kind, DueKind::StartupComplete { unit } if unit == BLUE)),
            "blue's startup stands"
        );
        assert_eq!(resolve_startup(&mut arena, RED, 0), Err(Refusal::Plant));
        assert_eq!(Refusal::Plant.as_str(), "plant");
        set_word(&mut arena, RED, "plant", "destroyed");
        arena.apply_part_effects(RED);
        assert_eq!(resolve_startup(&mut arena, RED, 0), Err(Refusal::Plant));
        set_word(&mut arena, RED, "plant", "intact");
        arena.apply_part_effects(RED);
        assert_eq!(
            arena.unit(RED).map(|state| state.power),
            Some(Power::Off),
            "unjam restores STARTUP, never the power"
        );
        assert_eq!(resolve_startup(&mut arena, RED, 0), Ok(()));
        assert_eq!(pending(&arena), 1);
    }

    /// **Q-40-3's core row**: a jammed or damaged core reads one class below its cell, the cell
    /// untouched, and the class returns with the word.
    #[test]
    fn fr5_a_jammed_core_reads_a_class_down_and_the_cell_is_untouched() {
        let mut arena = arena_of(vec![unit(RED, PartyId::RED, "intact", "intact", "intact")]);
        let class = |arena: &Arena| arena.core_of(RED).map(|core| core.class);
        let cell = |arena: &Arena| {
            arena.unit(RED).and_then(|state| {
                state
                    .unit
                    .cell(&CellAddr::Component {
                        component: ComponentId::new("core").expect("a component"),
                        cell: CellId::new(crate::definitions::unit::CORE_CLASS_CELL)
                            .expect("a cell"),
                    })
                    .map(|cell| cell.current)
            })
        };
        assert_eq!(class(&arena), Some(CoreClass::II));
        for word in ["jammed", "damaged"] {
            set_word(&mut arena, RED, "core", word);
            arena.apply_part_effects(RED);
            assert_eq!(class(&arena), Some(CoreClass::I), "{word}: one class down");
            assert_eq!(cell(&arena), Some(2), "{word}: the cell is untouched");
        }
        set_word(&mut arena, RED, "core", "intact");
        arena.apply_part_effects(RED);
        assert_eq!(class(&arena), Some(CoreClass::II), "the class returns");
        // A core the bay left jammed checks out a class down.
        let jammed = arena_of(vec![unit(RED, PartyId::RED, "intact", "intact", "jammed")]);
        assert_eq!(class(&jammed), Some(CoreClass::I));
    }

    /// **The table's bounds and its override** (Q-40-2): the compiled numbers are Q-40-3's,
    /// a unit's own table wins, and a per mille past 1,000 is refused.
    #[test]
    fn fr5_the_compiled_table_is_the_rulings_and_a_unit_may_override_it() {
        let stock = crate::arena::resolution::COMPONENT_EFFECTS;
        assert_eq!(stock.sensor.damaged_range_permille, 500);
        assert_eq!(stock.power.damaged_heat_permille, 500);
        assert_eq!(stock.relay.damaged_delay_ms, 10_000);
        assert!(stock.ecm.damaged_off && stock.eccm.damaged_off);
        assert_eq!(
            (
                stock.core.jammed_classes_lost,
                stock.core.damaged_classes_lost
            ),
            (1, 1)
        );
        assert_eq!(stock.check(), Ok(()));
        let partial: ComponentEffects =
            serde_json::from_value(serde_json::json!({"sensor": {"damaged_range_permille": 250}}))
                .expect("a partial block decodes");
        assert_eq!(partial.sensor.damaged_range_permille, 250);
        assert_eq!(
            partial.power, stock.power,
            "an unnamed row is the compiled one"
        );
        let mut wide = stock;
        wide.sensor.damaged_range_permille = 1_001;
        assert!(wide.check().is_err());
        let mut custody = unit(RED, PartyId::RED, "intact", "damaged", "intact");
        custody.profile.component_effects = Some(partial);
        let arena = arena_of(vec![custody]);
        assert_eq!(
            arena
                .unit(RED)
                .and_then(|state| state.sensor.as_ref())
                .map(|sensor| sensor.max_range_cells),
            Some(visual().max_range_cells / 4),
            "the unit's own table"
        );
    }
}
