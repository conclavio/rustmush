//! TacticalUnit acceptance (M32): construction/validation from a ruleset schema,
//! and the atomic ruleset-authored damage pipeline (plan → validate → apply →
//! journal). All pure: `apply` takes the unit value + `now`; no Engine.

use mush_core::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, GOD, ObjId, OwnerId,
};
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellAddr, CellSpec, ComponentId, ComponentSpec,
    ConditionId, DefinitionRef, FacetId, FacetSpec, InstallPoint, InstallSlotId, JournalEntry,
    LegacyUnitV1Error, LocomotionId, MAX_UNIT_MUTATION_OPERATIONS, MAX_UNIT_SECTIONS, MediumId,
    MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary, ModeRules, RulesValue,
    SectionId, SectionSpec, TacticalUnit, UnitCatalogBuilder, UnitError, UnitMutationOp,
    UnitMutationPlan, UnitSpec, UnitSpecBody, apply, decode_legacy_unit_v1,
};
use serde_json::json;

macro_rules! unit_plan {
    (
        ops: $ops:expr,
        expected_revision: $expected_revision:expr,
        cause: $cause:expr $(,)?
    ) => {
        UnitMutationPlan::new($ops, $expected_revision, $cause).expect("bounded test plan")
    };
    (
        expected_revision: $expected_revision:expr,
        ..$base:expr $(,)?
    ) => {
        ($base).with_expected_revision($expected_revision)
    };
}

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
    CellSpec {
        id: cellid(id),
        min,
        max,
        init,
    }
}
fn rv() -> RulesValue {
    RulesValue::new(dref_as("schema", "tacsim.x"), json!({})).unwrap()
}
fn unit_ref(id: &str) -> DefinitionRef {
    dref_as("unit", id)
}
fn component_ref(id: &str) -> DefinitionRef {
    dref_as("component", id)
}
fn dref_as(kind: &str, id: &str) -> DefinitionRef {
    dref_owned("tacsim", kind, id)
}
fn dref_owned(owner: &str, kind: &str, id: &str) -> DefinitionRef {
    let mut definitions = DefinitionRegistryBuilder::new();
    definitions
        .register(
            OwnerId::new(owner).unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind).unwrap(),
                DefinitionId::new(id).unwrap(),
                1,
                json!({}),
            )
            .unwrap(),
        )
        .unwrap()
}

fn a_profile() -> MobilityProfile {
    let reg = MobilityVocabulary::base().freeze();
    let m = MobilityMode::new(
        MobilityModeId::new("ground.biped").unwrap(),
        [MediumId::new("ground").unwrap()],
        LocomotionId::new("legged").unwrap(),
        ModeRules::new(dref_as("schema", "tacsim.kin"), json!({})).unwrap(),
    );
    MobilityProfile::new(vec![m], MobilityModeId::new("ground.biped").unwrap(), &reg).unwrap()
}

/// A small mech-like unit: a center torso with front/rear armor facets + a
/// structure cell + an engine slot, a unit heat pool, an engine component, and a
/// mobility capability sourced from the engine.
fn mech_spec() -> UnitSpec {
    UnitSpec {
        definition: unit_ref("tacsim.mech.locust"),
        sections: vec![SectionSpec {
            id: sid("center_torso"),
            condition: cond("ok"),
            facets: vec![
                FacetSpec {
                    id: fid("front"),
                    cells: vec![cspec("armor", 0, 10, 10)],
                },
                FacetSpec {
                    id: fid("rear"),
                    cells: vec![cspec("armor", 0, 6, 6)],
                },
            ],
            slots: vec![slot("engine_slot")],
            cells: vec![cspec("structure", 0, 8, 8)],
        }],
        unit_cells: vec![cspec("heat", 0, 30, 0)],
        components: vec![ComponentSpec {
            id: compid("engine"),
            definition: component_ref("tacsim.eq.fusion"),
            install: InstallPoint::Slot {
                section: sid("center_torso"),
                slot: slot("engine_slot"),
            },
            condition: cond("ok"),
            cells: vec![],
            asset: None,
        }],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey {
                id: capid("tactics.mobility"),
                source: Some(compid("engine")),
            },
            rules: rv(),
        }],
        mobility: Some(a_profile()),
        rules: rv(),
    }
}

fn registered_spec(body: UnitSpecBody, version: u32) -> UnitSpec {
    registered_spec_for(body, "tacsim", "tacsim.mech.locust", version)
}

fn registered_spec_for(body: UnitSpecBody, owner: &str, id: &str, version: u32) -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new(id).unwrap(),
            version,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new(owner).unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

fn mech_body_for_owner(owner: &str) -> UnitSpecBody {
    let mut body = mech_spec().body();
    body.rules = RulesValue::new(
        dref_owned(owner, "schema", "tacsim.x"),
        body.rules.data().clone(),
    )
    .unwrap();
    for component in &mut body.components {
        component.definition = dref_owned(owner, "component", "tacsim.eq.fusion");
    }
    for capability in &mut body.capabilities {
        capability.rules = RulesValue::new(
            dref_owned(owner, "schema", "tacsim.x"),
            capability.rules.data().clone(),
        )
        .unwrap();
    }
    if let Some(profile) = &mut body.mobility {
        let mut modes = profile.modes().to_vec();
        for mode in &mut modes {
            mode.rules = RulesValue::new(
                dref_owned(owner, "schema", "tacsim.kin"),
                mode.rules.data().clone(),
            )
            .unwrap();
        }
        *profile = MobilityProfile::new(
            modes,
            MobilityModeId::new("ground.biped").unwrap(),
            &MobilityVocabulary::base().freeze(),
        )
        .unwrap();
    }
    body
}

fn mech() -> TacticalUnit {
    TacticalUnit::from_spec(&mech_spec(), GOD).unwrap()
}

fn rear_armor() -> CellAddr {
    CellAddr::Facet {
        section: sid("center_torso"),
        facet: fid("rear"),
        cell: cellid("armor"),
    }
}
fn structure() -> CellAddr {
    CellAddr::Section {
        section: sid("center_torso"),
        cell: cellid("structure"),
    }
}
fn heat() -> CellAddr {
    CellAddr::Unit {
        cell: cellid("heat"),
    }
}
fn mobility_key() -> CapabilityKey {
    CapabilityKey {
        id: capid("tactics.mobility"),
        source: Some(compid("engine")),
    }
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
    assert_eq!(
        u.section(&sid("center_torso")).unwrap().condition,
        cond("ok")
    );
    assert_eq!(
        u.component(&compid("engine")).unwrap().condition,
        cond("ok")
    );
    assert!(u.has_capability(&capid("tactics.mobility")));
    assert!(u.mobility().is_some());
}

/// 16b cut AE.3: `rules()` answers the unit-level blob `from_spec` was given,
/// byte for byte — the schema reference and the opaque data both — so a game
/// can read its own facts (tacsim's unit class) off a bound unit.
#[test]
fn rules_answers_the_blob_from_spec_was_given() {
    let mut spec = mech_spec();
    spec.rules = RulesValue::new(
        dref_as("schema", "tacsim.x"),
        json!({ "class": "drone", "nested": { "n": 7, "list": [1, 2, 3] } }),
    )
    .unwrap();
    let unit = TacticalUnit::from_spec(&spec, GOD).unwrap();
    assert_eq!(unit.rules(), &spec.rules);
    assert_eq!(unit.rules().schema(), spec.rules.schema());
    assert_eq!(
        serde_json::to_vec(unit.rules()).unwrap(),
        serde_json::to_vec(&spec.rules).unwrap(),
        "the blob is carried byte for byte"
    );
    assert_eq!(unit.rules().data()["class"], json!("drone"));
    // And it survives the unit's own serde round trip unchanged.
    let back: TacticalUnit = serde_json::from_slice(&serde_json::to_vec(&unit).unwrap()).unwrap();
    assert_eq!(back.rules(), &spec.rules);
}

#[test]
fn construction_rejects_structural_violations() {
    // No sections.
    let mut empty = mech_spec();
    empty.sections.clear();
    assert_eq!(
        TacticalUnit::from_spec(&empty, GOD).unwrap_err(),
        UnitError::EmptyUnit
    );

    // Duplicate section id.
    let mut dup = mech_spec();
    dup.sections.push(dup.sections[0].clone());
    assert_eq!(
        TacticalUnit::from_spec(&dup, GOD).unwrap_err(),
        UnitError::DuplicateSection(sid("center_torso"))
    );

    // A cell spec with init out of range.
    let mut badcell = mech_spec();
    badcell.unit_cells[0].init = 99;
    assert!(matches!(
        TacticalUnit::from_spec(&badcell, GOD).unwrap_err(),
        UnitError::CellBounds { .. }
    ));

    // A component installed in a slot the section does not declare.
    let mut badinstall = mech_spec();
    badinstall.components[0].install = InstallPoint::Slot {
        section: sid("center_torso"),
        slot: slot("ghost_slot"),
    };
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
    let plan = unit_plan! {
        ops: vec![
            UnitMutationOp::AdjustCell {
                addr: rear_armor(),
                delta: -6,
            },
            UnitMutationOp::AdjustCell {
                addr: structure(),
                delta: -3,
            },
            UnitMutationOp::SetComponentCondition {
                component: compid("engine"),
                condition: cond("destroyed"),
            },
            UnitMutationOp::RevokeCapability {
                key: mobility_key(),
            },
            UnitMutationOp::AdjustCell {
                addr: heat(),
                delta: 5,
            },
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
    assert_eq!(
        post.component(&compid("engine")).unwrap().condition,
        cond("destroyed")
    );
    assert!(
        !post.has_capability(&capid("tactics.mobility")),
        "mobility revoked"
    );
    assert_eq!(u.revision(), 0, "original untouched");
    assert_eq!(
        u.cell(&rear_armor()).unwrap().current,
        6,
        "original untouched"
    );
    assert!(
        u.has_capability(&capid("tactics.mobility")),
        "original untouched"
    );

    // The journal records the actual deltas in order.
    assert_eq!(journal.from_revision, 0);
    assert_eq!(journal.to_revision, 1);
    assert_eq!(journal.now, 1000);
    assert_eq!(journal.cause.as_deref(), Some("alpha strike"));
    assert_eq!(journal.entries.len(), 5);
    assert_eq!(
        journal.entries[0],
        JournalEntry::Cell {
            addr: rear_armor(),
            from: 6,
            to: 0
        }
    );
    assert!(matches!(
        &journal.entries[3],
        JournalEntry::CapabilityRevoked {
            was_present: true,
            ..
        }
    ));
}

#[test]
fn mutation_plan_operation_limit_accepts_boundary_and_rejects_overflow_without_mutation() {
    assert_eq!(
        MAX_UNIT_MUTATION_OPERATIONS,
        mush_core::MAX_CASCADE_OPERATIONS as usize,
        "a tactical plan cannot exceed its enclosing K2 action budget"
    );

    let unit = mech();
    let before = unit.clone();
    let no_change = UnitMutationOp::SetCell {
        addr: heat(),
        value: 0,
    };
    let boundary = UnitMutationPlan::new(
        vec![no_change.clone(); MAX_UNIT_MUTATION_OPERATIONS],
        Some(0),
        Some("operation boundary".into()),
    )
    .unwrap();
    assert_eq!(boundary.ops().len(), MAX_UNIT_MUTATION_OPERATIONS);

    // The persisted/wire path accepts the exact boundary too.
    let encoded = serde_json::to_value(&boundary).unwrap();
    let decoded: UnitMutationPlan = serde_json::from_value(encoded.clone()).unwrap();
    let (post, journal) = apply(&unit, &decoded, 77).unwrap();
    assert_eq!(journal.entries.len(), MAX_UNIT_MUTATION_OPERATIONS);
    assert_eq!(post.revision(), 1);
    assert_eq!(post.cell(&heat()).unwrap().current, 0);
    assert_eq!(unit, before, "boundary apply never mutates its input value");

    let error = UnitMutationPlan::new(
        vec![no_change.clone(); MAX_UNIT_MUTATION_OPERATIONS + 1],
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(
        error,
        UnitError::LimitExceeded {
            collection: "unit mutation plan operations",
            count: MAX_UNIT_MUTATION_OPERATIONS + 1,
            max: MAX_UNIT_MUTATION_OPERATIONS,
        }
    );
    assert_eq!(unit, before, "constructor refusal is zero-mutation");

    // Add one op to the valid serialized shape. The bounded sequence visitor
    // refuses it without ever constructing an oversized plan.
    let mut oversized_wire = encoded;
    oversized_wire["ops"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(no_change).unwrap());
    let error = serde_json::from_value::<UnitMutationPlan>(oversized_wire).unwrap_err();
    let expected = format!(
        "unit mutation plan operations has size {}; maximum is {}",
        MAX_UNIT_MUTATION_OPERATIONS + 1,
        MAX_UNIT_MUTATION_OPERATIONS
    );
    assert!(error.to_string().contains(&expected), "{error}");
    assert_eq!(unit, before, "decode refusal is zero-mutation");
}

#[test]
fn out_of_bounds_refuses_the_whole_plan_and_never_clamps() {
    let u = mech();
    // structure -3 (fine) THEN rear armor -1000 (out of bounds) — the later
    // failure must discard the whole plan, not partially apply the first op.
    let plan = unit_plan! {
        ops: vec![
            UnitMutationOp::AdjustCell {
                addr: structure(),
                delta: -3,
            },
            UnitMutationOp::AdjustCell {
                addr: rear_armor(),
                delta: -1000,
            },
        ],
        expected_revision: None,
        cause: None,
    };
    let err = apply(&u, &plan, 0).unwrap_err();
    assert!(matches!(
        err,
        UnitError::OutOfBounds {
            min: 0,
            max: 6,
            value: -994,
            ..
        }
    ));
    // Zero mutation: the earlier op did NOT land, and armor was NOT clamped to 0.
    assert_eq!(u.cell(&structure()).unwrap().current, 8);
    assert_eq!(u.cell(&rear_armor()).unwrap().current, 6);
    assert_eq!(u.revision(), 0);
}

#[test]
fn ops_compose_sequentially_on_one_cell_and_overflow_is_refused() {
    let u = mech();
    // Two adjusts on rear armor (6 -> 2 -> 0) compose in authored order.
    let ok = unit_plan! {
        ops: vec![
            UnitMutationOp::AdjustCell {
                addr: rear_armor(),
                delta: -4,
            },
            UnitMutationOp::AdjustCell {
                addr: rear_armor(),
                delta: -2,
            },
        ],
        expected_revision: None,
        cause: None,
    };
    let (post, _) = apply(&u, &ok, 0).unwrap();
    assert_eq!(post.cell(&rear_armor()).unwrap().current, 0);

    // A delta that exceeds a cell's bounds is refused as OutOfBounds...
    let oob = unit_plan! {
        ops: vec![UnitMutationOp::AdjustCell {
            addr: heat(),
            delta: i64::MAX,
        }],
        expected_revision: None,
        cause: None,
    };
    assert!(matches!(
        apply(&u, &oob, 0).unwrap_err(),
        UnitError::OutOfBounds { .. }
    ));

    // ...and arithmetic that would overflow i64 is refused as Overflow (a cell
    // pinned at i64::MAX, adjusted up) — checked, never wrapping.
    let maxed = TacticalUnit::from_spec(
        &UnitSpec {
            definition: unit_ref("t.big"),
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
    let over = unit_plan! {
        ops: vec![UnitMutationOp::AdjustCell {
            addr: CellAddr::Section {
                section: sid("chassis"),
                cell: cellid("big"),
            },
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
    let key = CapabilityKey {
        id: capid("tactics.observation.sensor"),
        source: None,
    };
    let plan = unit_plan! {
        ops: vec![
            UnitMutationOp::GrantCapability {
                key: key.clone(),
                rules: rv(),
            },
            UnitMutationOp::GrantCapability {
                key: key.clone(),
                rules: rv(),
            }, // replaced
            UnitMutationOp::RevokeCapability {
                key: CapabilityKey {
                    id: capid("tactics.nope"),
                    source: None,
                },
            }, // absent no-op
        ],
        expected_revision: None,
        cause: None,
    };
    let (post, journal) = apply(&u, &plan, 0).unwrap();
    assert!(post.has_capability(&capid("tactics.observation.sensor")));
    assert_eq!(
        journal.entries[0],
        JournalEntry::CapabilityGranted {
            key: key.clone(),
            replaced: false
        }
    );
    assert_eq!(
        journal.entries[1],
        JournalEntry::CapabilityGranted {
            key,
            replaced: true
        }
    );
    assert!(matches!(
        journal.entries[2],
        JournalEntry::CapabilityRevoked {
            was_present: false,
            ..
        }
    ));
}

#[test]
fn expected_revision_guards_a_stale_plan() {
    let u = mech(); // revision 0
    let stale = unit_plan! {
        ops: vec![UnitMutationOp::AdjustCell {
            addr: heat(),
            delta: 1,
        }],
        expected_revision: Some(7),
        cause: None,
    };
    assert_eq!(
        apply(&u, &stale, 0).unwrap_err(),
        UnitError::StaleRevision {
            expected: 7,
            actual: 0
        }
    );

    let fresh = unit_plan! {
        expected_revision: Some(0),
        ..stale.clone()
    };
    let (post, _) = apply(&u, &fresh, 0).unwrap();
    assert_eq!(post.revision(), 1);
    // The same expected-0 plan is now stale against the bumped unit.
    assert_eq!(
        apply(&post, &fresh, 0).unwrap_err(),
        UnitError::StaleRevision {
            expected: 0,
            actual: 1
        }
    );
}

#[test]
fn unit_round_trips_and_rejects_poisoned_documents() {
    // Round-trip a damaged unit field-for-field.
    let (u, _) = apply(
        &mech(),
        &unit_plan! {
            ops: vec![UnitMutationOp::AdjustCell {
                addr: rear_armor(),
                delta: -2,
            }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    let json = serde_json::to_value(&u).unwrap();
    let back: TacticalUnit = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, u, "unit round-trips field-for-field");

    let mut unknown = json.clone();
    unknown["unexpected"] = json!(true);
    assert!(
        serde_json::from_value::<TacticalUnit>(unknown).is_err(),
        "the current unit wire shape is strict"
    );

    // Poison: no sections.
    let mut no_sections = json.clone();
    no_sections["sections"] = serde_json::json!({});
    assert!(
        serde_json::from_value::<TacticalUnit>(no_sections).is_err(),
        "empty unit rejected"
    );

    // Poison: a cell out of its own bounds.
    let mut bad_cell = json.clone();
    if let Some(map) = bad_cell["cells"].as_array_mut() {
        // cells serialize as an array of [addr, cell] pairs (BTreeMap w/ enum key).
        if let Some(pair) = map.get_mut(0) {
            pair[1]["current"] = serde_json::json!(9_999);
        }
    }
    assert!(
        serde_json::from_value::<TacticalUnit>(bad_cell).is_err(),
        "out-of-bounds cell rejected"
    );

    // Poison: a capability sourced from a component that does not exist.
    let mut bad_src = json.clone();
    bad_src["capabilities"][0][0]["source"] = serde_json::json!("ghost");
    assert!(
        serde_json::from_value::<TacticalUnit>(bad_src).is_err(),
        "unresolved capability source rejected"
    );

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
    assert!(
        serde_json::from_value::<TacticalUnit>(bad_facet).is_err(),
        "undeclared facet in cell address rejected"
    );

    // Poison: a component installed in a slot the section does not declare.
    let mut bad_slot = json.clone();
    bad_slot["components"]["engine"]["install"]["Slot"]["slot"] = serde_json::json!("ghost");
    assert!(
        serde_json::from_value::<TacticalUnit>(bad_slot).is_err(),
        "unresolved install slot rejected"
    );

    // Poison: a duplicate cell pair in the Vec-repr.
    let mut dup_cell = json;
    if let Some(arr) = dup_cell["cells"].as_array_mut() {
        let first = arr[0].clone();
        arr.push(first);
    }
    assert!(
        serde_json::from_value::<TacticalUnit>(dup_cell).is_err(),
        "duplicate cell rejected"
    );
}

#[test]
fn granting_a_capability_from_a_missing_source_is_refused() {
    let u = mech();
    let bad = unit_plan! {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey {
                id: capid("tactics.action.attack"),
                source: Some(compid("ghost")),
            },
            rules: rv(),
        }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(
        apply(&u, &bad, 0).unwrap_err(),
        UnitError::UnresolvedCapabilitySource(compid("ghost"))
    );
    assert_eq!(u.revision(), 0, "refused grant is zero-mutation");

    // A grant from an EXISTING source succeeds, and apply's output always
    // re-deserializes (the invariant the missing-source grant would have broken).
    let ok = unit_plan! {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey {
                id: capid("tactics.action.attack"),
                source: Some(compid("engine")),
            },
            rules: rv(),
        }],
        expected_revision: None,
        cause: None,
    };
    let (post, _) = apply(&u, &ok, 0).unwrap();
    let back: TacticalUnit = serde_json::from_value(serde_json::to_value(&post).unwrap()).unwrap();
    assert_eq!(back, post);

    let wrong_kind = unit_plan! {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey {
                id: capid("tactics.action.scan"),
                source: None,
            },
            rules: RulesValue::new(dref_as("action", "tacsim.scan"), json!({})).unwrap(),
        }],
        expected_revision: None,
        cause: None,
    };
    assert!(matches!(
        apply(&u, &wrong_kind, 0),
        Err(UnitError::DefinitionKindMismatch {
            field: "capability rules",
            ..
        })
    ));
    assert_eq!(u.revision(), 0, "wrong-kind grants are zero-mutation");

    let cross_owner = unit_plan! {
        ops: vec![UnitMutationOp::GrantCapability {
            key: CapabilityKey {
                id: capid("tactics.action.scan"),
                source: None,
            },
            rules: RulesValue::new(dref_owned("vehicles", "schema", "scan"), json!({})).unwrap(),
        }],
        expected_revision: None,
        cause: None,
    };
    assert!(matches!(
        apply(&u, &cross_owner, 0),
        Err(UnitError::CrossOwnerDefinitionDependency {
            field: "capability rules",
            ..
        })
    ));
    assert_eq!(u.revision(), 0, "cross-owner grants are zero-mutation");
}

#[test]
fn duplicate_facet_or_slot_ids_in_a_section_are_rejected() {
    let mut dupf = mech_spec();
    dupf.sections[0].facets.push(FacetSpec {
        id: fid("rear"),
        cells: vec![],
    });
    assert_eq!(
        TacticalUnit::from_spec(&dupf, GOD).unwrap_err(),
        UnitError::DuplicateFacet {
            section: sid("center_torso"),
            facet: fid("rear")
        }
    );
    let mut dups = mech_spec();
    dups.sections[0].slots.push(slot("engine_slot"));
    assert_eq!(
        TacticalUnit::from_spec(&dups, GOD).unwrap_err(),
        UnitError::DuplicateSlot {
            section: sid("center_torso"),
            slot: slot("engine_slot")
        }
    );
}

#[test]
fn set_cell_writes_absolute_value_and_guards_bounds() {
    let u = mech();
    let (post, journal) = apply(
        &u,
        &unit_plan! {
            ops: vec![UnitMutationOp::SetCell {
                addr: rear_armor(),
                value: 3,
            }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(post.cell(&rear_armor()).unwrap().current, 3);
    assert_eq!(
        journal.entries[0],
        JournalEntry::Cell {
            addr: rear_armor(),
            from: 6,
            to: 3
        }
    );
    // The max endpoint is allowed.
    let at_max = unit_plan! {
        ops: vec![UnitMutationOp::SetCell {
            addr: heat(),
            value: 30,
        }],
        expected_revision: None,
        cause: None,
    };
    assert!(apply(&u, &at_max, 0).is_ok());
    // Over the max is refused, zero mutation.
    let over = unit_plan! {
        ops: vec![UnitMutationOp::SetCell {
            addr: heat(),
            value: 31,
        }],
        expected_revision: None,
        cause: None,
    };
    assert!(matches!(
        apply(&u, &over, 0).unwrap_err(),
        UnitError::OutOfBounds { .. }
    ));
    assert_eq!(u.cell(&heat()).unwrap().current, 0);
}

#[test]
fn set_section_condition_and_its_unknown_section_error() {
    let u = mech();
    let (post, journal) = apply(
        &u,
        &unit_plan! {
            ops: vec![UnitMutationOp::SetSectionCondition {
                section: sid("center_torso"),
                condition: cond("destroyed"),
            }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(
        post.section(&sid("center_torso")).unwrap().condition,
        cond("destroyed")
    );
    assert_eq!(
        journal.entries[0],
        JournalEntry::SectionCondition {
            section: sid("center_torso"),
            from: cond("ok"),
            to: cond("destroyed")
        }
    );
    let bad = unit_plan! {
        ops: vec![UnitMutationOp::SetSectionCondition {
            section: sid("ghost"),
            condition: cond("x"),
        }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(
        apply(&u, &bad, 0).unwrap_err(),
        UnitError::UnknownSection(sid("ghost"))
    );
}

#[test]
fn apply_refuses_unknown_referents_with_zero_mutation() {
    let u = mech();
    // A later op naming an unknown cell discards the earlier op too (atomic).
    let plan = unit_plan! {
        ops: vec![
            UnitMutationOp::AdjustCell {
                addr: structure(),
                delta: -1,
            },
            UnitMutationOp::AdjustCell {
                addr: CellAddr::Unit {
                    cell: cellid("nonesuch"),
                },
                delta: -1,
            },
        ],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(
        apply(&u, &plan, 0).unwrap_err(),
        UnitError::UnknownCell(CellAddr::Unit {
            cell: cellid("nonesuch")
        })
    );
    assert_eq!(
        u.cell(&structure()).unwrap().current,
        8,
        "the earlier op did not land"
    );
    // An unknown component condition is refused too.
    let badc = unit_plan! {
        ops: vec![UnitMutationOp::SetComponentCondition {
            component: compid("ghost"),
            condition: cond("x"),
        }],
        expected_revision: None,
        cause: None,
    };
    assert_eq!(
        apply(&u, &badc, 0).unwrap_err(),
        UnitError::UnknownComponent(compid("ghost"))
    );
}

#[test]
fn apply_is_deterministic_and_a_coarse_unit_works() {
    // Purity: identical inputs reproduce identical outputs.
    let u = mech();
    let plan = unit_plan! {
        ops: vec![UnitMutationOp::AdjustCell {
            addr: structure(),
            delta: -2,
        }],
        expected_revision: None,
        cause: Some("x".into()),
    };
    assert_eq!(apply(&u, &plan, 42).unwrap(), apply(&u, &plan, 42).unwrap());

    // A coarse WoD-style unit: one chassis section + unit pools, no facets,
    // no components, no mobility.
    let car = TacticalUnit::from_spec(
        &UnitSpec {
            definition: unit_ref("wod.vehicle.sedan"),
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
        &unit_plan! {
            ops: vec![UnitMutationOp::AdjustCell {
                addr: CellAddr::Unit {
                    cell: cellid("fuel"),
                },
                delta: -25,
            }],
            expected_revision: None,
            cause: None,
        },
        0,
    )
    .unwrap();
    assert_eq!(
        post.cell(&CellAddr::Unit {
            cell: cellid("fuel")
        })
        .unwrap()
        .current,
        75
    );
}

#[test]
fn unit_definition_body_is_non_recursive_and_lists_every_exact_dependency() {
    let spec = mech_spec();
    let body = spec.body();
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.locust").unwrap(),
            1,
        )
        .unwrap();

    assert!(
        descriptor.payload().get("definition").is_none(),
        "root self-ref is not definition payload"
    );
    let references = descriptor.references();
    assert_eq!(
        references.len(),
        3,
        "component, common rules schema, and mobility schema"
    );
    assert!(references.contains(&spec.components[0].definition));
    assert!(references.contains(spec.rules.schema()));
    assert!(references.contains(spec.mobility.as_ref().unwrap().modes()[0].rules.schema()));
}

#[test]
fn catalog_is_exact_duplicate_rejecting_and_never_guesses_between_versions() {
    let mobility = MobilityVocabulary::base().freeze();
    let body = mech_spec().body();
    let v1 = registered_spec(body.clone(), 1);
    let v2 = registered_spec(body, 2);
    let mut builder = UnitCatalogBuilder::new();
    builder.add(v1.clone(), &mobility).unwrap();
    assert!(matches!(
        builder.add(v1.clone(), &mobility),
        Err(UnitError::DuplicateDefinition(_))
    ));

    let conflicting = DefinitionRef::new(
        v1.definition.key().clone(),
        v1.definition.version(),
        DefinitionDigest::from_bytes([0x55; 32]),
    )
    .unwrap();
    let mut conflicting_spec = v1.clone();
    conflicting_spec.definition = conflicting;
    assert!(matches!(
        builder.add(conflicting_spec, &mobility),
        Err(UnitError::DefinitionDigestConflict { .. })
    ));

    builder.add(v2.clone(), &mobility).unwrap();
    let catalog = builder.freeze();
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.get_exact(&v1.definition), Some(&v1));
    assert_eq!(catalog.get_exact(&v2.definition), Some(&v2));
    assert!(
        catalog.get("tacsim.mech.locust").is_none(),
        "bare id is ambiguous across exact versions"
    );
}

#[test]
fn catalog_runs_full_structure_identity_and_bound_validation_before_freeze() {
    let mobility = MobilityVocabulary::base().freeze();

    let mut empty_body = mech_spec().body();
    empty_body.sections.clear();
    let empty = registered_spec(empty_body, 1);
    assert_eq!(
        UnitCatalogBuilder::new().add(empty, &mobility).unwrap_err(),
        UnitError::EmptyUnit
    );

    let body = mech_spec().body();
    let mut drifted = registered_spec(body, 1);
    drifted.sections[0].condition = cond("body_changed_without_version_or_digest");
    assert!(matches!(
        UnitCatalogBuilder::new().add(drifted, &mobility),
        Err(UnitError::DefinitionIdentityMismatch { .. })
    ));

    let mut oversized = mech_spec();
    oversized.sections = vec![oversized.sections[0].clone(); MAX_UNIT_SECTIONS + 1];
    assert!(matches!(
        TacticalUnit::from_spec(&oversized, GOD),
        Err(UnitError::LimitExceeded {
            collection: "sections",
            ..
        })
    ));
}

#[test]
fn catalog_rejects_definition_refs_from_the_wrong_typed_namespace() {
    let mobility = MobilityVocabulary::base().freeze();

    let mut wrong_root = registered_spec(mech_spec().body(), 1);
    wrong_root.definition = dref_as("component", "tacsim.mech.locust");
    assert!(matches!(
        UnitCatalogBuilder::new().add(wrong_root, &mobility),
        Err(UnitError::DefinitionKindMismatch {
            field: "unit definition",
            ..
        })
    ));

    let mut wrong_component = registered_spec(mech_spec().body(), 1);
    wrong_component.components[0].definition = dref_as("unit", "tacsim.eq.fusion");
    assert!(matches!(
        UnitCatalogBuilder::new().add(wrong_component, &mobility),
        Err(UnitError::DefinitionKindMismatch {
            field: "component definition",
            ..
        })
    ));

    let mut wrong_unit_rules = registered_spec(mech_spec().body(), 1);
    wrong_unit_rules.rules = RulesValue::new(dref_as("action", "tacsim.x"), json!({})).unwrap();
    assert!(matches!(
        UnitCatalogBuilder::new().add(wrong_unit_rules, &mobility),
        Err(UnitError::DefinitionKindMismatch {
            field: "unit rules",
            ..
        })
    ));

    let mut wrong_capability_rules = registered_spec(mech_spec().body(), 1);
    wrong_capability_rules.capabilities[0].rules =
        RulesValue::new(dref_as("action", "tacsim.x"), json!({})).unwrap();
    assert!(matches!(
        UnitCatalogBuilder::new().add(wrong_capability_rules, &mobility),
        Err(UnitError::DefinitionKindMismatch {
            field: "capability rules",
            ..
        })
    ));

    let mut wrong_mobility_rules = registered_spec(mech_spec().body(), 1);
    let bad_mode = MobilityMode::new(
        MobilityModeId::new("ground.biped").unwrap(),
        [MediumId::new("ground").unwrap()],
        LocomotionId::new("legged").unwrap(),
        ModeRules::new(dref_as("action", "tacsim.kin"), json!({})).unwrap(),
    );
    wrong_mobility_rules.mobility = Some(
        MobilityProfile::new(
            vec![bad_mode],
            MobilityModeId::new("ground.biped").unwrap(),
            &mobility,
        )
        .unwrap(),
    );
    assert!(matches!(
        UnitCatalogBuilder::new().add(wrong_mobility_rules, &mobility),
        Err(UnitError::DefinitionKindMismatch {
            field: "mobility mode rules",
            ..
        })
    ));
}

#[test]
fn unit_composition_rejects_cross_owner_dependencies_until_the_body_binds_a_bridge() {
    let mobility = MobilityVocabulary::base().freeze();
    let foreign = |kind: &str, id: &str| dref_owned("vehicles", kind, id);

    let reject = |body: UnitSpecBody, field: &'static str| {
        let spec = registered_spec(body, 1);
        let error = UnitCatalogBuilder::new()
            .add(spec.clone(), &mobility)
            .unwrap_err();
        assert!(matches!(
            &error,
            UnitError::CrossOwnerDefinitionDependency {
                field: actual,
                root,
                dependency,
            } if *actual == field
                && root.key().owner.as_str() == "tacsim"
                && dependency.key().owner.as_str() == "vehicles"
        ));
        assert_eq!(spec.descriptor().unwrap_err(), error);
        assert_eq!(TacticalUnit::from_spec(&spec, GOD).unwrap_err(), error);
    };

    let mut body = mech_spec().body();
    body.rules = RulesValue::new(foreign("schema", "unit.rules"), json!({})).unwrap();
    reject(body, "unit rules");

    let mut body = mech_spec().body();
    body.components[0].definition = foreign("component", "fusion");
    reject(body, "component definition");

    let mut body = mech_spec().body();
    body.capabilities[0].rules =
        RulesValue::new(foreign("schema", "mobility.capability"), json!({})).unwrap();
    reject(body, "capability rules");

    let mut body = mech_spec().body();
    let foreign_mode = MobilityMode::new(
        MobilityModeId::new("ground.biped").unwrap(),
        [MediumId::new("ground").unwrap()],
        LocomotionId::new("legged").unwrap(),
        ModeRules::new(foreign("schema", "kinematics"), json!({})).unwrap(),
    );
    body.mobility = Some(
        MobilityProfile::new(
            vec![foreign_mode],
            MobilityModeId::new("ground.biped").unwrap(),
            &mobility,
        )
        .unwrap(),
    );
    reject(body, "mobility mode rules");

    // The serde/TryFrom path is the persisted-record boundary. It must apply
    // the same guard even when no catalog builder or create command is used.
    let mut persisted = serde_json::to_value(mech()).unwrap();
    persisted["components"]["engine"]["definition"] =
        serde_json::to_value(foreign("component", "fusion")).unwrap();
    let error = serde_json::from_value::<TacticalUnit>(persisted).unwrap_err();
    assert!(
        error.to_string().contains("cross-owner unit composition"),
        "{error}"
    );
}

#[test]
fn same_owner_unit_composition_is_accepted_by_catalog_create_and_persisted_decode() {
    let mobility = MobilityVocabulary::base().freeze();
    let spec = registered_spec(mech_spec().body(), 1);
    let mut builder = UnitCatalogBuilder::new();
    builder.add(spec.clone(), &mobility).unwrap();
    assert_eq!(builder.freeze().get_exact(&spec.definition), Some(&spec));

    let unit = TacticalUnit::from_spec(&spec, GOD).unwrap();
    let decoded: TacticalUnit =
        serde_json::from_value(serde_json::to_value(&unit).unwrap()).unwrap();
    assert_eq!(decoded, unit);
}

#[test]
fn catalog_current_selection_uses_only_explicit_registry_metadata() {
    let mobility = MobilityVocabulary::base().freeze();
    let body = mech_spec().body();
    let mut definitions = DefinitionRegistryBuilder::new();

    // Register the exact dependencies named by the body so the shared registry
    // can freeze and validate the complete reference graph.
    for reference in body.definition_references() {
        let registered = definitions
            .register(
                reference.key().owner.clone(),
                DefinitionDescriptor::new(
                    reference.key().kind.clone(),
                    reference.key().id.clone(),
                    reference.version(),
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(registered, reference);
    }

    let v1_ref = definitions
        .register(
            OwnerId::new("tacsim").unwrap(),
            body.descriptor(
                DefinitionKind::new("unit").unwrap(),
                DefinitionId::new("tacsim.mech.locust").unwrap(),
                1,
            )
            .unwrap()
            .as_current(),
        )
        .unwrap();
    let v2_ref = definitions
        .register(
            OwnerId::new("tacsim").unwrap(),
            body.descriptor(
                DefinitionKind::new("unit").unwrap(),
                DefinitionId::new("tacsim.mech.locust").unwrap(),
                2,
            )
            .unwrap(),
        )
        .unwrap();
    let registry = definitions.freeze().unwrap();

    let v1 = body.clone().with_definition(v1_ref);
    let v2 = body.with_definition(v2_ref);
    let mut catalog = UnitCatalogBuilder::new();
    catalog.add(v1.clone(), &mobility).unwrap();
    catalog.add(v2, &mobility).unwrap();
    let catalog = catalog.freeze();

    assert_eq!(
        catalog.get_current("tacsim.mech.locust", &registry),
        Some(&v1)
    );
    assert!(catalog.get("tacsim.mech.locust").is_none());
    assert!(catalog.get_current("missing", &registry).is_none());
}

#[test]
fn instance_reconciliation_rejects_definition_shape_drift_but_allows_runtime_state() {
    let spec = mech_spec();
    let pristine = TacticalUnit::from_spec(&spec, GOD).unwrap();
    spec.validate_instance(&pristine).unwrap();

    // Runtime state may differ from authoring defaults. Conditions, current
    // values, mode state, opaque data, and baseline capability presence are all
    // intentionally mutable while their schemas/topology remain fixed.
    let mut mutable = serde_json::to_value(&pristine).unwrap();
    mutable["sections"]["center_torso"]["condition"] = json!("disabled");
    mutable["components"]["engine"]["condition"] = json!("damaged");
    mutable["cells"].as_array_mut().unwrap()[0][1]["current"] = json!(1);
    mutable["rules"]["data"] = json!({"runtime": true});
    mutable["capabilities"] = json!([]);
    mutable["mobility"]["modes"][0]["rules"]["data"] = json!({"runtime": 7});
    mutable["mobility"]["mode"] = json!({
        "Switching": {
            "from": "ground.biped",
            "to": "ground.biped",
            "ready_at": 99
        }
    });
    let mutable: TacticalUnit = serde_json::from_value(mutable).unwrap();
    spec.validate_instance(&mutable).unwrap();

    let assert_shape = |mut value: serde_json::Value, expected: &'static str| {
        let unit: TacticalUnit = serde_json::from_value(value.take()).unwrap();
        assert_eq!(
            spec.validate_instance(&unit).unwrap_err(),
            UnitError::DefinitionShapeMismatch { field: expected }
        );
    };

    let mut section = serde_json::to_value(&pristine).unwrap();
    section["sections"]["center_torso"]["facets"] = json!(["front", "rear", "side"]);
    assert_shape(section, "facet topology");

    let mut slot_topology = serde_json::to_value(&pristine).unwrap();
    slot_topology["sections"]["center_torso"]["slots"] = json!(["engine_slot", "invented_slot"]);
    assert_shape(slot_topology, "install-slot topology");

    let mut cell_bounds = serde_json::to_value(&pristine).unwrap();
    cell_bounds["cells"].as_array_mut().unwrap()[0][1]["max"] = json!(999);
    assert_shape(cell_bounds, "cell bounds");

    let mut component_definition = serde_json::to_value(&pristine).unwrap();
    component_definition["components"]["engine"]["definition"] =
        serde_json::to_value(component_ref("tacsim.eq.counterfeit")).unwrap();
    assert_shape(component_definition, "component definition");

    let mut component_install = serde_json::to_value(&pristine).unwrap();
    component_install["components"]["engine"]["install"] =
        serde_json::to_value(InstallPoint::Unit).unwrap();
    assert_shape(component_install, "component install");

    let mut mobility = serde_json::to_value(&pristine).unwrap();
    mobility["mobility"]["modes"][0]["nominal_speed_cms"] = json!(123);
    assert_shape(mobility, "mobility mode structure");

    let mut rules_schema = serde_json::to_value(&pristine).unwrap();
    rules_schema["rules"]["schema"] =
        serde_json::to_value(dref_as("schema", "tacsim.other")).unwrap();
    assert_shape(rules_schema, "unit rules schema");
}

#[test]
fn persisted_tactical_ids_must_already_be_canonical() {
    assert_eq!(
        SectionId::new(" Center_Torso ").unwrap(),
        sid("center_torso")
    );
    let error = serde_json::from_value::<SectionId>(json!("Center_Torso")).unwrap_err();
    assert!(error.to_string().contains("not canonical"));
}

fn legacy_mech_v1() -> serde_json::Value {
    json!({
        "object": GOD,
        "schema": { "id": "tacsim.mech.locust", "version": 1 },
        "sections": {
            "center_torso": {
                "condition": "damaged",
                "facets": ["front", "rear"],
                "slots": ["engine_slot"]
            }
        },
        "cells": [
            [{ "Unit": { "cell": "heat" } }, { "min": 0, "max": 30, "current": 4 }],
            [{ "Section": { "section": "center_torso", "cell": "structure" } },
                { "min": 0, "max": 8, "current": 5 }],
            [{ "Facet": { "section": "center_torso", "facet": "front", "cell": "armor" } },
                { "min": 0, "max": 10, "current": 7 }],
            [{ "Facet": { "section": "center_torso", "facet": "rear", "cell": "armor" } },
                { "min": 0, "max": 6, "current": 2 }]
        ],
        "components": {
            "engine": {
                "definition": { "id": "tacsim.eq.fusion", "version": 1 },
                "install": { "Slot": { "section": "center_torso", "slot": "engine_slot" } },
                "condition": "ok",
                "asset": null
            }
        },
        "capabilities": [[
            { "id": "tactics.mobility", "source": "engine" },
            { "schema": "tacsim.x", "version": 1, "data": { "throttle": 2 } }
        ]],
        "mobility": {
            "modes": [{
                "id": "ground.biped",
                "mediums": ["ground"],
                "locomotion": "legged",
                "tags": [],
                "rules": { "schema": "tacsim.kin", "version": 1, "data": { "walk": 6 } },
                "nominal_speed_cms": 0,
                "transition_s": 0
            }],
            "mode": { "Settled": "ground.biped" }
        },
        "revision": 9,
        "rules": { "schema": "tacsim.x", "version": 1, "data": { "pilot": "npc" } }
    })
}

#[test]
fn legacy_root_owner_is_selected_only_by_an_explicit_alias() {
    let mobility = MobilityVocabulary::base().freeze();
    let tacsim = registered_spec_for(
        mech_body_for_owner("tacsim"),
        "tacsim",
        "tacsim.mech.locust",
        1,
    );
    let imported = registered_spec_for(
        mech_body_for_owner("imported"),
        "imported",
        "tacsim.mech.locust",
        1,
    );

    let mut without_context = UnitCatalogBuilder::new();
    without_context.add(tacsim.clone(), &mobility).unwrap();
    without_context.add(imported.clone(), &mobility).unwrap();
    assert_eq!(
        decode_legacy_unit_v1(&legacy_mech_v1(), &without_context.freeze()).unwrap_err(),
        LegacyUnitV1Error::RootDefinitionUnavailable,
        "catalog membership is not authority to infer an owner"
    );

    let mut explicit = UnitCatalogBuilder::new();
    explicit.add(tacsim, &mobility).unwrap();
    explicit.add(imported.clone(), &mobility).unwrap();
    explicit
        .legacy_v1_alias(
            DefinitionId::new("tacsim.mech.locust").unwrap(),
            1,
            imported.definition.clone(),
        )
        .unwrap();
    assert_eq!(
        decode_legacy_unit_v1(&legacy_mech_v1(), &explicit.freeze())
            .unwrap()
            .unit
            .schema(),
        &imported.definition
    );
}

#[test]
fn strict_legacy_v1_decoder_maps_only_catalog_refs_and_is_pure_and_deterministic() {
    let mobility = MobilityVocabulary::base().freeze();
    let spec = registered_spec(mech_spec().body(), 1);
    let mut catalog = UnitCatalogBuilder::new();
    catalog.add(spec.clone(), &mobility).unwrap();
    catalog
        .legacy_v1_alias(
            DefinitionId::new("tacsim.mech.locust").unwrap(),
            1,
            spec.definition.clone(),
        )
        .unwrap();
    let catalog = catalog.freeze();
    let legacy = legacy_mech_v1();
    let untouched = legacy.clone();

    let first = decode_legacy_unit_v1(&legacy, &catalog).unwrap();
    let second = decode_legacy_unit_v1(&legacy, &catalog).unwrap();
    assert_eq!(legacy, untouched, "the dry-run adapter performs no writes");
    assert_eq!(
        first, second,
        "identical old bytes map to identical v2 bytes"
    );
    assert_eq!(first.unit.schema(), &spec.definition);
    assert_eq!(first.unit.revision(), 9);
    assert_eq!(first.unit.cell(&rear_armor()).unwrap().current, 2);
    let current_body = serde_json::to_value(&first.unit).unwrap();
    assert_eq!(
        current_body["schema"]["key"]["owner"],
        json!("tacsim"),
        "the current body carries explicit owner/kind/id identity"
    );
    assert!(current_body["schema"].get("digest").is_some());

    let refs = first.unit.definition_references();
    assert!(refs.contains(&spec.definition));
    assert!(refs.contains(&spec.components[0].definition));
    assert!(refs.contains(spec.rules.schema()));
    assert!(refs.contains(spec.mobility.as_ref().unwrap().modes()[0].rules.schema()));
}

#[test]
fn strict_legacy_v1_decoder_refuses_unknown_fields_and_context_drift() {
    let mobility = MobilityVocabulary::base().freeze();
    let spec = registered_spec(mech_spec().body(), 1);
    let mut catalog = UnitCatalogBuilder::new();
    catalog.add(spec.clone(), &mobility).unwrap();
    catalog
        .legacy_v1_alias(
            DefinitionId::new("tacsim.mech.locust").unwrap(),
            1,
            spec.definition,
        )
        .unwrap();
    let catalog = catalog.freeze();

    let mut unknown = legacy_mech_v1();
    unknown["extra"] = json!(true);
    assert!(matches!(
        decode_legacy_unit_v1(&unknown, &catalog),
        Err(LegacyUnitV1Error::Malformed(_))
    ));

    let mut drifted = legacy_mech_v1();
    drifted["components"]["engine"]["definition"]["version"] = json!(2);
    assert_eq!(
        decode_legacy_unit_v1(&drifted, &catalog).unwrap_err(),
        LegacyUnitV1Error::DefinitionMismatch {
            field: "component definition"
        }
    );

    let mut drifted_shape = legacy_mech_v1();
    let heat = drifted_shape["cells"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|pair| pair[0].get("Unit").is_some())
        .unwrap();
    heat[1]["max"] = json!(31);
    assert!(matches!(
        decode_legacy_unit_v1(&drifted_shape, &catalog),
        Err(LegacyUnitV1Error::InvalidUnit(error))
            if *error == UnitError::DefinitionShapeMismatch { field: "cell bounds" }
    ));
}
