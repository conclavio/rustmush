//! Structural execution custody only: the actual host still supplies its
//! audited active owner record. These tests cannot manufacture a host permit.
use super::*;
use crate::arena::{
    Arena, Controller, EventKind, PartyId,
    core::CoreClass,
    mind::{DropCause, MemValue, Memory, encode_memory},
    motor::MotorCounters,
    profile_seal::{FiledCertificate, MindIdentity, MindOrigin},
    runtime_card::stock_definition,
    state::Power,
    test_support::{arena_of, pose, tank},
};
use crate::arena_task::MatchId;
use crate::match_::SealTerms;
use mush_core::{CommitToken, ObjId};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const UNIT: ObjId = ObjId(4);
const OLD_ABI: &str = "rhai=1.26.0;api=4;stock=2";
const SOURCE: &str = "fn think(v,c) { this.marker = 73; }";

fn fixture(abi: &str, source: &str) -> Result<(Execution, Arena), Box<dyn std::error::Error>> {
    let mut hull = tank(UNIT, PartyId::RED, pose(1500, 1500));
    hull.controller = Controller::Mind;
    let mut terms = SealTerms::fixture();
    terms.match_id = MatchId(7);
    let definition = stock_definition(&[("Line".into(), terms.card.clone())], CoreClass::I)?;
    let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    let certificate = FiledCertificate {
        revision_character: ObjId(77),
        revision_mind: 1,
        revision_number: 1,
        revision_digest: DefinitionDigest::from_bytes([1; 32]),
        source_digest: DefinitionDigest::from_bytes(digest),
        rating_job: CommitToken::from_bytes([4; 16]),
        rating_id: DefinitionDigest::from_bytes([2; 32]),
        rating_schema: 1,
        rating_digest: DefinitionDigest::from_bytes([3; 32]),
        compatibility: DefinitionDigest::from_bytes([4; 32]),
        definition: definition.validate()?,
        declared: CoreClass::I,
        minimum: CoreClass::I,
    };
    terms.authored = Some(Arc::new(definition));
    terms.selected_profiles = vec!["Line".into()];
    terms.identity = MindIdentity {
        mind: "Retained fixture".into(),
        origin: MindOrigin::Filed {
            charter: "1".into(),
            line: "1".into(),
            revision: 1,
            licence: "1".into(),
            certificate: Box::new(certificate),
        },
        source_sha256: digest,
    };
    terms.rated = CoreClass::I;
    terms.engine_for_minds = SealEngine::Scripted;
    hull.reseal(&terms)?;
    hull.seal.abi = abi.into();
    let execution = Execution {
        seal: hull.seal.clone(),
        source: source.into(),
        arena: ArenaConfig::default(),
        classes: terms.classes,
    };
    execution.validate()?;
    let mut arena = arena_of(vec![hull, tank(ObjId(9), PartyId::BLUE, pose(6500, 4500))]);
    arena.unit_mut(UNIT).ok_or("unit")?.power = Power::Online;
    Ok((execution, arena))
}
fn events(arena: &mut Arena) -> Result<Vec<crate::arena::LedgerEvent>, Box<dyn std::error::Error>> {
    arena.ledger.flush();
    let mut result = Vec::new();
    for chunk in arena.ledger.take_chunks() {
        for line in std::str::from_utf8(&chunk.bytes)?.lines() {
            result.push(serde_json::from_str(line)?);
        }
    }
    Ok(result)
}

#[test]
fn retained_ic_abi_drop_preserves_seal_memory_and_cancels_old_answer_once() -> TestResult {
    check_ne!(
        crate::minds::MIND_ABI,
        OLD_ABI,
        "paired API5 stabilization prerequisite"
    );
    // Rhai and stock version changes take exactly the same complete-ABI path.
    for abi in [
        OLD_ABI,
        "rhai=1.25.0;api=4;stock=2",
        "rhai=1.26.0;api=4;stock=1",
    ] {
        let (execution, mut arena) = fixture(abi, SOURCE)?;
        let sealed_bytes = serde_json::to_vec(&execution)?;
        let original_seal = execution.seal.seal_ref();
        let memory = encode_memory(&Memory::from([("kept".into(), MemValue::Int(41))]));
        arena.minds.get_mut(&UNIT).ok_or("mind")?.memory = memory.clone();
        arena.now_ms = 1250;
        arena.collect_think(UNIT, 0, 1001, &mut MotorCounters::default());
        check!(arena.minds.get(&UNIT).ok_or("mind")?.pending.is_some());
        check!(arena.collected.iter().any(|think| think.unit == UNIT));
        execution.install(&mut arena)?;
        let row = arena.minds.get(&UNIT).ok_or("mind")?;
        check_eq!(row.memory, memory);
        check_eq!(row.dropped, Some(DropCause::Abi));
        check!(row.pending.is_none());
        check!(!arena.collected.iter().any(|think| think.unit == UNIT));
        check_eq!(arena.unit(UNIT).ok_or("unit")?.seal, original_seal);
        check_eq!(serde_json::to_vec(&execution)?, sealed_bytes);
        let runtime = arena.profiles.get(&UNIT).ok_or("profile")?;
        check!(runtime.dropped);
        check_eq!(runtime.card.key(), runtime.rom.key());
        check!(runtime.program.rows.is_empty());
        let generation = runtime.generation;
        execution.install(&mut arena)?;
        check_eq!(
            arena.profiles.get(&UNIT).ok_or("profile")?.generation,
            generation
        );
        check_eq!(arena.minds.get(&UNIT).ok_or("mind")?.memory, memory);
        let drops = events(&mut arena)?
            .into_iter()
            .filter(|event| {
                matches!(
                    &event.kind, EventKind::MindDropped { unit, cause, fault: None, ops: None }
                    if *unit == UNIT && cause == "abi"
                )
            })
            .count();
        check_eq!(drops, 1);
        // Current native doctrine really answers the next collected job.
        let seq = arena.minds.get(&UNIT).ok_or("mind")?.think_seq;
        arena.collect_think(UNIT, seq, 2001, &mut MotorCounters::default());
        let next = arena.collected.pop().ok_or("native next think")?;
        check!(next.job().dropped);
        check_eq!(next.job().profile.class, CoreClass::Zero);
        check!(next.think().result.fault.is_none());
    }
    Ok(())
}

#[test]
fn retained_ic_skips_old_compiler_but_keeps_exact_source_and_runtime_affinity() -> TestResult {
    let removed = "fn think(v,c) { missing_old_global; }";
    let (old, mut arena) = fixture(OLD_ABI, removed)?;
    check!(
        crate::minds::RhaiRunner::new(
            &old.arena,
            removed,
            mush_mind::Digest(old.seal.source_sha256)
        )
        .is_err()
    );
    old.compile()?;
    let profile = arena.profiles.get(&UNIT).ok_or("profile")?.clone();
    *arena
        .unit_mut(UNIT)
        .ok_or("unit")?
        .seal
        .digest
        .first_mut()
        .ok_or("digest")? ^= 1;
    check!(old.install(&mut arena).is_err());
    check_eq!(arena.profiles.get(&UNIT), Some(&profile));
    arena.unit_mut(UNIT).ok_or("unit")?.seal = old.seal.seal_ref();
    let mut corrupt = old.clone();
    corrupt.source.push(' ');
    check!(corrupt.compile().is_err());
    check!(corrupt.install(&mut arena).is_err());
    check_eq!(arena.profiles.get(&UNIT), Some(&profile));
    old.install(&mut arena)?;
    // A pre-existing permanent drop is not recast or journalled again.
    arena.minds.get_mut(&UNIT).ok_or("mind")?.dropped = Some(DropCause::Core);
    let generation = arena.profiles.get(&UNIT).ok_or("profile")?.generation;
    old.install(&mut arena)?;
    check_eq!(
        arena.minds.get(&UNIT).ok_or("mind")?.dropped,
        Some(DropCause::Core)
    );
    check_eq!(
        arena.profiles.get(&UNIT).ok_or("profile")?.generation,
        generation
    );
    Ok(())
}

#[test]
fn retained_current_abi_installs_actual_script_without_rom_drop() -> TestResult {
    let (execution, mut arena) = fixture(crate::minds::MIND_ABI, SOURCE)?;
    execution.install(&mut arena)?;
    arena.now_ms = 1250;
    arena.collect_think(UNIT, 0, 1001, &mut MotorCounters::default());
    let next = arena.collected.pop().ok_or("actual scripted think")?;
    check!(!next.job().dropped);
    let result = next.think().result;
    check!(result.fault.is_none());
    let memory = crate::arena::mind::decode_memory(&result.memory);
    check_eq!(memory.get("marker"), Some(&MemValue::Int(73)));
    check_eq!(arena.minds.get(&UNIT).ok_or("mind")?.dropped, None);
    check!(
        !events(&mut arena)?
            .iter()
            .any(|event| matches!(event.kind, EventKind::MindDropped { .. }))
    );
    Ok(())
}

#[test]
fn retained_ic_abi_journal_capacity_refuses_before_runtime_changes() -> TestResult {
    let (execution, mut arena) = fixture(OLD_ABI, SOURCE)?;
    let profile = arena.profiles.get(&UNIT).ok_or("profile")?.clone();
    let mind = arena.minds.get(&UNIT).ok_or("mind")?.clone();
    arena.ledger = crate::arena::Ledger::new(&crate::config::LedgerConfig {
        max_bytes: 1,
        ..crate::config::LedgerConfig::default()
    });
    check!(execution.install(&mut arena).is_err());
    check_eq!(arena.profiles.get(&UNIT), Some(&profile));
    check_eq!(arena.minds.get(&UNIT), Some(&mind));
    check!(!arena.runners.contains_key(&UNIT));
    check!(events(&mut arena)?.is_empty());
    Ok(())
}

/// The black box's open line for a trip sealed with `facts` (doc 17h cut M8b G11).
fn box_open_line(
    facts: crate::surface::black_box::SealFacts,
) -> Result<crate::surface::black_box::BlackBoxLine, Box<dyn std::error::Error>> {
    let opened = crate::surface::excursion::OpenSortie {
        id: crate::arena::mind::SortieId::ic(UNIT, 500),
        hull: ObjId(20),
        slot: 1,
        site: mush_core::CanonicalId::new("ahuna")?,
        operator: ObjId(10),
        opened_ic: 800,
        abandoned: false,
        abandoned_ic: None,
        unrecorded: false,
        seal: Some(facts),
    };
    Ok(crate::surface::black_box::BlackBoxLine::mind_sealed(
        &opened,
        UNIT,
        None,
        crate::surface::black_box::Admitted {
            class: Some(crate::definitions::UnitClass::Vehicle),
            host: crate::surface::black_box::EpochHost::Surface,
            count: 1,
        },
    ))
}

/// `unit`'s core with its governor's bit off (the capability's payload rewritten in place).
fn ungovern(unit: &mut mush_tactics::TacticalUnit) -> TestResult {
    let core = mush_tactics::ComponentId::new("core")?;
    let key = mush_tactics::CapabilityKey {
        id: mush_tactics::CapabilityId::new(crate::definitions::unit::CORE_CAPABILITY)?,
        source: Some(core),
    };
    let schema = unit.capability(&key).ok_or("a core")?.schema().clone();
    let class = crate::arena::core::read_core(unit).ok_or("a core")?.class;
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::GrantCapability {
            key,
            rules: mush_core::RulesValue::new(
                schema,
                crate::definitions::core::profile_payload(class, false),
            )?,
        }],
        Some(unit.revision()),
        None,
    )?;
    *unit = mush_tactics::apply(unit, &plan, 0)?.0;
    Ok(())
}

/// **Doc 17h cut M8b C4, the gate's test 5** (`unfiled`, `unlicensed`; D-UM-41, D-UM-42):
/// the IC execution refuses a personal draft (no filed certificate) on a governed core and
/// admits it on an ungoverned one, sealed with `authored` the draft's definition and its
/// source digest; the ungoverned seal's lines record `unfiled` and, under the draft's licence
/// word, `unlicensed`. A governed seal records nothing, whatever the word.
#[test]
fn m8b_an_ungoverned_core_admits_an_unfiled_draft_and_records_it() -> TestResult {
    use crate::arena::mind_seal::Admission;
    use crate::minds::licence_state::SealLicenceState;
    for governed in [true, false] {
        let mut hull = tank(UNIT, PartyId::RED, pose(1500, 1500));
        hull.controller = Controller::Mind;
        if !governed {
            ungovern(&mut hull.unit)?;
        }
        let mut terms = SealTerms::fixture();
        terms.match_id = MatchId(7);
        let definition = stock_definition(&[("Line".into(), terms.card.clone())], CoreClass::I)?;
        let digest: [u8; 32] = Sha256::digest(SOURCE.as_bytes()).into();
        terms.authored = Some(Arc::new(definition));
        terms.selected_profiles = vec!["Line".into()];
        terms.identity = MindIdentity {
            mind: "Personal draft".into(),
            origin: MindOrigin::Draft {
                character: ObjId(77),
                mind: 1,
                revision: 1,
                digest: DefinitionDigest::from_bytes([1; 32]),
            },
            source_sha256: digest,
        };
        terms.rated = CoreClass::I;
        terms.engine_for_minds = SealEngine::Scripted;
        hull.reseal(&terms)?;
        check_eq!(hull.seal.governed, governed);
        let execution = Execution {
            seal: hull.seal.clone(),
            source: SOURCE.into(),
            arena: ArenaConfig::default(),
            classes: terms.classes,
        };
        if governed {
            check!(execution.validate().is_err(), "a governed core refuses it");
            check!(
                !super::super::audit::unfiled_matches(
                    &hull.seal,
                    &DefinitionDigest::from_bytes(digest)
                ),
                "the audit refuses it on a governed seal"
            );
            check!(hull.seal.admissions(SealLicenceState::Draft).is_empty());
        } else {
            execution.validate()?;
            check_eq!(
                hull.seal.admissions(SealLicenceState::Draft),
                vec![Admission::Unfiled, Admission::Unlicensed]
            );
            // G11: the seal's own lines carry it — the arena line, the facts and the box's
            // open line built from them.
            let expected = vec![Admission::Unfiled, Admission::Unlicensed];
            let line_admissions = match hull.seal.sealed_line(SealLicenceState::Draft) {
                crate::arena::EventKind::MindSealed { admissions, .. } => admissions,
                _ => Vec::new(),
            };
            check_eq!(line_admissions, expected);
            let facts =
                crate::surface::black_box::SealFacts::of(&hull.seal, SealLicenceState::Draft);
            check_eq!(facts.admissions, expected);
            let open = box_open_line(facts)?;
            check!(open.check(UNIT).is_ok());
            let value = serde_json::to_value(&open)?;
            check_eq!(
                value["admissions"],
                serde_json::json!(["unfiled", "unlicensed"])
            );
            // G4: the audit admits the uncertified seal for its own source only.
            check!(super::super::audit::unfiled_matches(
                &hull.seal,
                &DefinitionDigest::from_bytes(digest)
            ));
            check!(!super::super::audit::unfiled_matches(
                &hull.seal,
                &DefinitionDigest::from_bytes([9; 32])
            ));
            check_eq!(
                hull.seal.admissions(SealLicenceState::Active),
                vec![Admission::Unfiled]
            );
        }
    }
    Ok(())
}

/// **Doc 17h cut M8b C4, the floor's first point** (D-UM-17, D-UM-42; the gate's test 5,
/// `under_floor`, and the over-class admission), on an armed Class I hull: the Line card under
/// a floor of 200 is refused
/// `Unlawful` at the seal on a governed core and sealed on an ungoverned one — the seal keeps
/// the captured floor, holds its cards to none, and records `under_floor`; a mind rated over
/// the core is refused `OverClass` on a governed core and sealed at the core's class on an
/// ungoverned one, recording `over_class`.
#[test]
fn m8b_an_ungoverned_core_seals_under_the_floor_and_over_its_class() -> TestResult {
    use crate::arena::mind_seal::Admission;
    use crate::match_::SealError;
    use crate::minds::licence_state::SealLicenceState;
    for governed in [true, false] {
        // The fixture tank's Class I core, and a cannon: a fire station, so the floor applies.
        let mut hull = tank(UNIT, PartyId::RED, pose(1500, 1500));
        hull.weapons.push(crate::arena::test_support::cannon());
        hull.controller = Controller::Mind;
        if !governed {
            ungovern(&mut hull.unit)?;
        }
        let floored = SealTerms {
            odds_floor_permille: 200,
            ..SealTerms::fixture()
        };
        let sealed = hull.reseal(&floored);
        if governed {
            check_eq!(sealed, Err(SealError::Unlawful { floor: 200 }));
        } else {
            sealed?;
            check!(hull.seal.lifts_floor());
            check_eq!(hull.seal.odds_floor_permille, 200);
            check_eq!(hull.seal.effective_floor(), 0);
            check_eq!(
                hull.seal.admissions(SealLicenceState::Doctrine),
                vec![Admission::UnderFloor]
            );
            let facts =
                crate::surface::black_box::SealFacts::of(&hull.seal, SealLicenceState::Doctrine);
            check_eq!(facts.admissions, vec![Admission::UnderFloor]);
            let value = serde_json::to_value(box_open_line(facts)?)?;
            check_eq!(value["admissions"], serde_json::json!(["under_floor"]));
        }
        let over = SealTerms {
            rated: CoreClass::II,
            ..SealTerms::fixture()
        };
        let sealed = hull.reseal(&over);
        if governed {
            check!(
                matches!(sealed, Err(SealError::OverClass { .. })),
                "{sealed:?}"
            );
        } else {
            sealed?;
            check_eq!(hull.seal.class, CoreClass::I);
            check_eq!(
                hull.seal.admissions(SealLicenceState::Doctrine),
                vec![Admission::OverClass]
            );
        }
    }
    Ok(())
}
