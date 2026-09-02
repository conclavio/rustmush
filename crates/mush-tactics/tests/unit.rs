//! TacticalUnit acceptance (M32): construction/validation from a ruleset schema,
//! and the atomic ruleset-authored damage pipeline (plan → validate → apply →
//! journal). All pure: `apply` takes the unit value + `now`; no Engine.

use mush_core::{GOD, ObjId};
use mush_tactics::{
    apply, CapabilityId, CapabilityInit, CapabilityKey, CellAddr, CellSpec, ComponentId,
    ComponentSpec, ConditionId, DefinitionRef, FacetId, FacetSpec, InstallPoint, InstallSlotId,
    JournalEntry, LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile,
    MobilityVocabulary, ModeRules, RulesValue, SectionId, SectionSpec, TacticalUnit, UnitError,
    UnitMutationOp, UnitMutationPlan, UnitSpec,
};

fn sid(s: &str) -> SectionId {
    SectionId::new(s).unwrap()
}
fn fid(s: &str) -> FacetId {
    FacetId::new(s).unwrap()
}
fn cellid(s: &str) -> mush_tactics::CellId {
    mush_tactics::CellId::new(s).unwrap()
}
fn compid(s: &str) -> ComponentId {
    ComponentId::new(s).unwrap()
}
fn capid(s: &str) -> CapabilityId {
    CapabilityId::new(s).unwrap()
}
fn cond(s: &str) -> ConditionId {
    ConditionId::new(s).unwrap()
}
fn slot(s: &str) -> InstallSlotId {
    InstallSlotId::new(s).unwrap()
}
fn cspec(id: &str, min: i64, max: i64, init: i64) -> CellSpec {
    CellSpec { id: cellid(id), min, max, init }
}
fn rv() -> RulesValue {
    RulesValue::empty("tacsim.x")
}
fn dref(id: &str) -> DefinitionRef {
    DefinitionRef::new(id, 1).unwrap()
}

fn a_profile() -> MobilityProfile {
    let reg = MobilityVocabulary::base().freeze();
    let m = MobilityMode::new(
        MobilityModeId::new("ground.biped").unwrap(),
        [MediumId::new("ground").unwrap()],
        LocomotionId::new("legged").unwrap(),
        ModeRules::empty("tacsim.kin"),
    );
    MobilityProfile::new(vec![m], MobilityModeId::new("ground.biped").unwrap(), &reg).unwrap()
}

/// A small mech-like unit: a center torso with front/rear armor facets + a
/// structure cell + an engine slot, a unit heat pool, an engine component, and a
/// mobility capability sourced from the engine.
fn mech_spec() -> UnitSpec {
    UnitSpec {
        definition: dref("tacsim.mech.locust"),
        sections: vec![SectionSpec {
            id: sid("center_torso"),
            condition: cond("ok"),
            facets: vec![
                FacetSpec { id: fid("front"), cells: vec![cspec("armor", 0, 10, 10)] },
                FacetSpec { id: fid("rear"), cells: vec![cspec("armor", 0, 6, 6)] },
            ],
            slots: vec![slot("engine_slot")],
            cells: vec![cspec("structure", 0, 8, 8)],
        }],
        unit_cells: vec![cspec("heat", 0, 30, 0)],
        components: vec![ComponentSpec {
            id: compid("engine"),
            definition: dref("tacsim.eq.fusion"),
            install: InstallPoint::Slot { section: sid("center_torso"), slot: slot("engine_slot") },
            condition: cond("ok"),
            cells: vec![],
            asset: None,
        }],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey { id: capid("tactics.mobility"), source: Some(compid("engine")) },
            rules: rv(),
        }],
        mobility: Some(a_profile()),
        rules: rv(),
    }
}

fn mech() -> TacticalUnit {
    TacticalUnit::from_spec(&mech_spec(), GOD).unwrap()
}

fn rear_armor() -> CellAddr {
    CellAddr::Facet { section: sid("center_torso"), facet: fid("rear"), cell: cellid("armor") }
}
fn structure() -> CellAddr {
    CellAddr::Section { section: sid("center_torso"), cell: cellid("structure") }
}
fn heat() -> CellAddr {
    CellAddr::Unit { cell: cellid("heat") }
}
fn mobility_key() -> CapabilityKey {
    CapabilityKey { id: capid("tactics.mobility"), source: Some(compid("engine")) }
}

// ----------------------------------------------------------------------

#[test]
fn a_unit_builds_from_its_schema() {
    let u = mech();
    assert_eq!(u.object(), GOD);
    assert_eq!(u.revision(), 0, "fresh unit has no applied plans");
    assert_eq!(u.cell(&rear_armor()).unwrap().current, 6);
    assert_eq!(u.cell(&structure()).unwrap().current, 8);
    assert_eq!(u.cell(&heat()).unwrap().current, 0);
    assert_eq!(u.section(&sid("center_torso")).unwrap().condition, cond("ok"));
    assert_eq!(u.component(&compid("engine")).unwrap().condition, cond("ok"));
    assert!(u.has_capability(&capid("tactics.mobility")));
    assert!(u.mobility().is_some());
}

#[test]
fn construction_rejects_structural_violations() {
    // No sections.
    let mut empty = mech_spec();
    empty.sections.clear();
    assert_eq!(TacticalUnit::from_spec(&empty, GOD).unwrap_err(), UnitError::EmptyUnit);

    // Duplicate section id.
    let mut dup = mech_spec();
    dup.sections.push(dup.sections[0].clone());
    assert_eq!(TacticalUnit::from_spec(&dup, GOD).unwrap_err(), UnitError::DuplicateSection(sid("center_torso")));

    // A cell spec with init out of range.
    let mut badcell = mech_spec();
    badcell.unit_cells[0].init = 99;
    assert!(matches!(TacticalUnit::from_spec(&badcell, GOD).unwrap_err(), UnitError::CellBounds { .. }));

    // A component installed in a slot the section does not declare.
    let mut badinstall = mech_spec();
    badinstall.components[0].install = InstallPoint::Slot { section: sid("center_torso"), slot: slot("ghost_slot") };
    assert_eq!(
        TacticalUnit::from_spec(&badinstall, GOD).unwrap_err(),
        UnitError::UnresolvedInstall(compid("engine"))
    );

    // A capability sourced from a component that does not exist.
    let mut badcap = mech_spec();
    badcap.capabilities[0].key.source = Some(compid("nonesuch"));
    assert_eq!(
        TacticalUnit::from_spec(&badcap, GOD).unwrap_err(),
        UnitError::UnresolvedCapabilitySource(compid("nonesuch"))
    );
}

#[test]
fn a_damage_plan_applies_atomically_and_journals_actual_effects() {
    let u = mech();
    let plan = UnitMutationPlan {
        ops: vec![
            UnitMutationOp::AdjustCell { addr: rear_armor(), delta: -6 },
            UnitMutationOp::AdjustCell { addr: structure(), delta: -3 },
            UnitMutationOp::SetComponentCondition { component: compid("engine"), condition: cond("destroyed") },
            UnitMutationOp::RevokeCapability { key: mobility_key() },
            UnitMutationOp::AdjustCell { addr: heat(), delta: 5 },
        ],
        expected_revision: None,
        cause: Some("alpha strike".into()),
    };
    let (post, journal) = apply(&u, &plan, 1000).unwrap();

    // The mutated unit reflects every op; the original is untouched.
    assert_eq!(post.revision(), 1);
    assert_eq!(post.cell(&rear_armor()).unwrap().current, 0);
    assert_eq!(post.cell(&structure()).unwrap().current, 5);
    assert_eq!(post.cell(&heat()).unwrap().current, 5);
    assert_eq!(post.component(&compid("engine")).unwrap().condition, cond("destroyed"));
    assert!(!post.has_capability(&capid("tactics.mobility")), "mobility revoked");
    assert_eq!(u.revision(), 0, "original untouched");
    assert_eq!(u.cell(&rear_armor()).unwrap().current, 6, "original untouched");
    assert!(u.has_capability(&capid("tactics.mobility")), "original untouched");

    // The journal records the actual deltas in order.
    assert_eq!(journal.from_revision, 0);
    assert_eq!(journal.to_revision, 1);
    assert_eq!(journal.now, 1000);
    assert_eq!(journal.cause.as_deref(), Some("alpha strike"));
    assert_eq!(journal.entries.len(), 5);
    assert_eq!(journal.entries[0], JournalEntry::Cell { addr: rear_armor(), from: 6, to: 0 });
    assert!(matches!(&journal.entries[3], JournalEntry::CapabilityRevoked { was_present: true, .. }));
}

#[test]
fn out_of_bounds_refuses_the_whole_plan_and_never_clamps() {
    let u = mech();
    // structure -3 (fine) THEN rear armor -1000 (out of bounds) — the later
    // failure must discard the whole plan, not partially apply the first op.
    let plan = UnitMutationPlan {
        ops: vec![
            UnitMutationOp::AdjustCell { addr: structure(), delta: -3 },
            UnitMutationOp::AdjustCell { addr: rear_armor(), delta: -1000 },
        ],
        expected_revision: None,
        cause: None,
    };
    let err = apply(&u, &plan, 0).unwrap_err();
    assert!(matches!(err, UnitError::OutOfBounds { min: 0, max: 6, value: -994, .. }));
    // Zero mutation: the earlier op did NOT land, and armor was NOT clamped to 0.
    assert_eq!(u.cell(&structure()).unwrap().current, 8);
    assert_eq!(u.cell(&rear_armor()).unwrap().current, 6);
    assert_eq!(u.revision(), 0);
}

#[test]
fn ops_compose_sequentially_on_one_cell_and_overflow_is_refused() {
    let u = mech();
    // Two adjusts on rear armor (6 -> 2 -> 0) compose in authored order.
    let ok = UnitMutationPlan {
        ops: vec![
            UnitMutationOp::AdjustCell { addr: rear_armor(), delta: -4 },
            UnitMutationOp::AdjustCell { addr: rear_armor(), delta: -2 },
        ],
        expected_revision: None,
        cause: None,
    };
    let (post, _) = apply(&u, &ok, 0).unwrap();
    assert_eq!(post.cell(&rear_armor()).unwrap().current, 0);

    // A delta that exceeds a cell's bounds is refused as OutOfBounds...
    let oob = UnitMutationPlan {
        ops: vec![UnitMutationOp::AdjustCell { addr: heat(), delta: i64::MAX }],
        expected_revision: None,
        cause: None,
    };
    assert!(matches!(apply(&u, &oob, 0).unwrap_err(), UnitError::OutOfBounds { .. }));

    // ...and arithmetic that would overflow i64 is refused as Overflow (a cell
    // pinned at i64::MAX, adjusted up) — checked, never wrapping.
    let maxed = TacticalUnit::from_spec(
        &UnitSpec {
            definition: dref("t.big"),
            sections: vec![SectionSpec {
                id: sid("chassis"),
                condition: cond("ok"),
                facets: vec![],
                slots: vec![],
                cells: vec![cspec("big", 0, i64::MAX, i64::MAX)],
            }],
            unit_cells: vec![],
            components: vec![],
            capabilities: vec![],
            mobility: None,
            rules: rv(),
        },
        GOD,
    )
    .unwrap();
    let over = UnitMutationPlan {
        ops: vec![UnitMutationOp::AdjustCell {
            addr: CellAddr::Section { section: sid("chassis"), cell: cellid("big") },
            delta: 1,
        }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(apply(&maxed, &over, 0).unwrap_err(), UnitError::Overflow);
}

#[test]
fn capability_grant_and_revoke_are_idempotent() {
    let u = mech();
    let key = CapabilityKey { id: capid("tactics.observation.sensor"), source: None };
    let plan = UnitMutationPlan {
        ops: vec![
            UnitMutationOp::GrantCapability { key: key.clone(), rules: rv() },
            UnitMutationOp::GrantCapability { key: key.clone(), rules: rv() }, // replaced
            UnitMutationOp::RevokeCapability { key: CapabilityKey { id: capid("tactics.nope"), source: None } }, // absent no-op
        ],
        expected_revision: None,
        cause: None,
    };
    let (post, journal) = apply(&u, &plan, 0).unwrap();
    assert!(post.has_capability(&capid("tactics.observation.sensor")));
    assert_eq!(journal.entries[0], JournalEntry::CapabilityGranted { key: key.clone(), replaced: false });
    assert_eq!(journal.entries[1], JournalEntry::CapabilityGranted { key, replaced: true });
    assert!(matches!(journal.entries[2], JournalEntry::CapabilityRevoked { was_present: false, .. }));
}

#[test]
fn expected_revision_guards_a_stale_plan() {
    let u = mech(); // revision 0
    let stale = UnitMutationPlan {
        ops: vec![UnitMutationOp::AdjustCell { addr: heat(), delta: 1 }],
        expected_revision: Some(7),
        cause: None,
    };
    assert_eq!(apply(&u, &stale, 0).unwrap_err(), UnitError::StaleRevision { expected: 7, actual: 0 });

    let fresh = UnitMutationPlan { expected_revision: Some(0), ..stale.clone() };
    let (post, _) = apply(&u, &fresh, 0).unwrap();
    assert_eq!(post.revision(), 1);
    // The same expected-0 plan is now stale against the bumped unit.
    assert_eq!(apply(&post, &fresh, 0).unwrap_err(), UnitError::StaleRevision { expected: 0, actual: 1 });
}

#[test]
fn unit_round_trips_and_rejects_poisoned_documents() {
    // Round-trip a damaged unit field-for-field.
    let (u, _) = apply(
        &mech(),
        &UnitMutationPlan {
            ops: vec![UnitMutationOp::AdjustCell { addr: rear_armor(), delta: -2 }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    let json = serde_json::to_value(&u).unwrap();
    let back: TacticalUnit = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, u, "unit round-trips field-for-field");

    // Poison: no sections.
    let mut no_sections = json.clone();
    no_sections["sections"] = serde_json::json!({});
    assert!(serde_json::from_value::<TacticalUnit>(no_sections).is_err(), "empty unit rejected");

    // Poison: a cell out of its own bounds.
    let mut bad_cell = json.clone();
    if let Some(map) = bad_cell["cells"].as_array_mut() {
        // cells serialize as an array of [addr, cell] pairs (BTreeMap w/ enum key).
        if let Some(pair) = map.get_mut(0) {
            pair[1]["current"] = serde_json::json!(9_999);
        }
    }
    assert!(serde_json::from_value::<TacticalUnit>(bad_cell).is_err(), "out-of-bounds cell rejected");

    // Poison: a capability sourced from a component that does not exist.
    let mut bad_src = json.clone();
    bad_src["capabilities"][0][0]["source"] = serde_json::json!("ghost");
    assert!(serde_json::from_value::<TacticalUnit>(bad_src).is_err(), "unresolved capability source rejected");

    // Poison: a facet cell address naming an undeclared facet.
    let mut bad_facet = json.clone();
    if let Some(arr) = bad_facet["cells"].as_array_mut() {
        for pair in arr.iter_mut() {
            if pair[0].get("Facet").is_some() {
                pair[0]["Facet"]["facet"] = serde_json::json!("ghost");
                break;
            }
        }
    }
    assert!(serde_json::from_value::<TacticalUnit>(bad_facet).is_err(), "undeclared facet in cell address rejected");

    // Poison: a component installed in a slot the section does not declare.
    let mut bad_slot = json.clone();
    bad_slot["components"]["engine"]["install"]["Slot"]["slot"] = serde_json::json!("ghost");
    assert!(serde_json::from_value::<TacticalUnit>(bad_slot).is_err(), "unresolved install slot rejected");

    // Poison: a duplicate cell pair in the Vec-repr.
    let mut dup_cell = json;
    if let Some(arr) = dup_cell["cells"].as_array_mut() {
        let first = arr[0].clone();
        arr.push(first);
    }
    assert!(serde_json::from_value::<TacticalUnit>(dup_cell).is_err(), "duplicate cell rejected");
}

#[test]
fn granting_a_capability_from_a_missing_source_is_refused() {
    let u = mech();
    let bad = UnitMutationPlan {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey { id: capid("tactics.action.attack"), source: Some(compid("ghost")) },
            rules: rv(),
        }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(apply(&u, &bad, 0).unwrap_err(), UnitError::UnresolvedCapabilitySource(compid("ghost")));
    assert_eq!(u.revision(), 0, "refused grant is zero-mutation");

    // A grant from an EXISTING source succeeds, and apply's output always
    // re-deserializes (the invariant the missing-source grant would have broken).
    let ok = UnitMutationPlan {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey { id: capid("tactics.action.attack"), source: Some(compid("engine")) },
            rules: rv(),
        }],
        expected_revision: None,
        cause: None,
    };
    let (post, _) = apply(&u, &ok, 0).unwrap();
    let back: TacticalUnit = serde_json::from_value(serde_json::to_value(&post).unwrap()).unwrap();
    assert_eq!(back, post);
}

#[test]
fn duplicate_facet_or_slot_ids_in_a_section_are_rejected() {
    let mut dupf = mech_spec();
    dupf.sections[0].facets.push(FacetSpec { id: fid("rear"), cells: vec![] });
    assert_eq!(
        TacticalUnit::from_spec(&dupf, GOD).unwrap_err(),
        UnitError::DuplicateFacet { section: sid("center_torso"), facet: fid("rear") }
    );
    let mut dups = mech_spec();
    dups.sections[0].slots.push(slot("engine_slot"));
    assert_eq!(
        TacticalUnit::from_spec(&dups, GOD).unwrap_err(),
        UnitError::DuplicateSlot { section: sid("center_torso"), slot: slot("engine_slot") }
    );
}

#[test]
fn set_cell_writes_absolute_value_and_guards_bounds() {
    let u = mech();
    let (post, journal) = apply(
        &u,
        &UnitMutationPlan {
            ops: vec![UnitMutationOp::SetCell { addr: rear_armor(), value: 3 }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(post.cell(&rear_armor()).unwrap().current, 3);
    assert_eq!(journal.entries[0], JournalEntry::Cell { addr: rear_armor(), from: 6, to: 3 });
    // The max endpoint is allowed.
    let at_max = UnitMutationPlan { ops: vec![UnitMutationOp::SetCell { addr: heat(), value: 30 }], expected_revision: None, cause: None };
    assert!(apply(&u, &at_max, 0).is_ok());
    // Over the max is refused, zero mutation.
    let over = UnitMutationPlan { ops: vec![UnitMutationOp::SetCell { addr: heat(), value: 31 }], expected_revision: None, cause: None };
    assert!(matches!(apply(&u, &over, 0).unwrap_err(), UnitError::OutOfBounds { .. }));
    assert_eq!(u.cell(&heat()).unwrap().current, 0);
}

#[test]
fn set_section_condition_and_its_unknown_section_error() {
    let u = mech();
    let (post, journal) = apply(
        &u,
        &UnitMutationPlan {
            ops: vec![UnitMutationOp::SetSectionCondition { section: sid("center_torso"), condition: cond("destroyed") }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(post.section(&sid("center_torso")).unwrap().condition, cond("destroyed"));
    assert_eq!(
        journal.entries[0],
        JournalEntry::SectionCondition { section: sid("center_torso"), from: cond("ok"), to: cond("destroyed") }
    );
    let bad = UnitMutationPlan {
        ops: vec![UnitMutationOp::SetSectionCondition { section: sid("ghost"), condition: cond("x") }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(apply(&u, &bad, 0).unwrap_err(), UnitError::UnknownSection(sid("ghost")));
}

#[test]
fn apply_refuses_unknown_referents_with_zero_mutation() {
    let u = mech();
    // A later op naming an unknown cell discards the earlier op too (atomic).
    let plan = UnitMutationPlan {
        ops: vec![
            UnitMutationOp::AdjustCell { addr: structure(), delta: -1 },
            UnitMutationOp::AdjustCell { addr: CellAddr::Unit { cell: cellid("nonesuch") }, delta: -1 },
        ],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(apply(&u, &plan, 0).unwrap_err(), UnitError::UnknownCell(CellAddr::Unit { cell: cellid("nonesuch") }));
    assert_eq!(u.cell(&structure()).unwrap().current, 8, "the earlier op did not land");
    // An unknown component condition is refused too.
    let badc = UnitMutationPlan {
        ops: vec![UnitMutationOp::SetComponentCondition { component: compid("ghost"), condition: cond("x") }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(apply(&u, &badc, 0).unwrap_err(), UnitError::UnknownComponent(compid("ghost")));
}

#[test]
fn apply_is_deterministic_and_a_coarse_unit_works() {
    // Purity: identical inputs reproduce identical outputs.
    let u = mech();
    let plan = UnitMutationPlan {
        ops: vec![UnitMutationOp::AdjustCell { addr: structure(), delta: -2 }],
        expected_revision: None,
        cause: Some("x".into()),
    };
    assert_eq!(apply(&u, &plan, 42).unwrap(), apply(&u, &plan, 42).unwrap());

    // A coarse WoD-style unit: one chassis section + unit pools, no facets,
    // no components, no mobility.
    let car = TacticalUnit::from_spec(
        &UnitSpec {
            definition: dref("wod.vehicle.sedan"),
            sections: vec![SectionSpec {
                id: sid("chassis"),
                condition: cond("intact"),
                facets: vec![],
                slots: vec![],
                cells: vec![cspec("integrity", 0, 20, 20)],
            }],
            unit_cells: vec![cspec("fuel", 0, 100, 100)],
            components: vec![],
            capabilities: vec![],
            mobility: None,
            rules: rv(),
        },
        ObjId(42),
    )
    .unwrap();
    assert!(car.mobility().is_none());
    let (post, _) = apply(
        &car,
        &UnitMutationPlan {
            ops: vec![UnitMutationOp::AdjustCell { addr: CellAddr::Unit { cell: cellid("fuel") }, delta: -25 }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(post.cell(&CellAddr::Unit { cell: cellid("fuel") }).unwrap().current, 75);
}
