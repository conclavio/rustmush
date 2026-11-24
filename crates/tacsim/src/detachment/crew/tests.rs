#![cfg(test)]
use super::*;
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
fn contract_binds_exact_terms_and_world_ic_clock_without_movement_history() -> TestResult {
    let terms = HireTerms {
        schema: 1,
        port: CanonicalId::new("ceres")?,
        mission_scope: NpcMissionScope::SubordinateOperator,
        duties: BTreeSet::from([CrewDuty::Mining]),
    };
    let mut contract = CrewContract {
        schema: 1,
        npc: ObjId(7),
        employer: ObjId(8),
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
        terms_hash: terms.digest()?,
        terms,
    };
    contract.validate()?;
    let value = serde_json::to_value(&contract)?;
    let decoded: CrewContract = serde_json::from_value(value.clone())?;
    check_eq!(decoded, contract);
    let mut forged = value;
    forged["embarkation_interval"] = json!(1);
    check!(serde_json::from_value::<CrewContract>(forged).is_err());
    contract.terms.duties.insert(CrewDuty::Combat);
    check!(
        contract.validate().is_err(),
        "a changed mission scope needs a new acceptance"
    );
    contract.terms_hash = contract.terms.digest()?;
    contract.hired_at.clock = ClockRef::WorldIc { generation: 0 };
    check!(contract.validate().is_err());
    Ok(())
}
#[test]
fn malformed_and_wrong_identity_contracts_cannot_make_a_crew_member_available() -> TestResult {
    let terms = HireTerms {
        schema: 1,
        port: CanonicalId::new("ceres")?,
        mission_scope: NpcMissionScope::DetachmentCommander,
        duties: BTreeSet::from([CrewDuty::Mining, CrewDuty::Combat]),
    };
    let contract = CrewContract {
        schema: 1,
        npc: ObjId(7),
        employer: ObjId(8),
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
        terms_hash: terms.digest()?,
        terms,
    };
    check!(runtime::check_contract(ObjId(7), Some(&contract))?);
    check!(runtime::check_contract(ObjId(9), Some(&contract)).is_err());
    check!(!runtime::check_contract(ObjId(7), None)?);
    let mut invalid = contract;
    invalid.terms.duties.clear();
    check!(invalid.validate().is_err());
    Ok(())
}

#[test]
fn m28_npc_exit_scopes_contracts_and_generates_distinct_frozen_clock_keys() -> TestResult {
    let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
        crate::TacsimRuleset::with_config(crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        }),
    )]);
    let npc = ObjId(7);
    let employer_a = ObjId(8);
    let employer_b = ObjId(9);
    let terms = HireTerms {
        schema: 1,
        port: CanonicalId::new("ceres")?,
        mission_scope: NpcMissionScope::SubordinateOperator,
        duties: BTreeSet::from([CrewDuty::Mining]),
    };
    let contract_old = CrewContract {
        schema: 1,
        npc,
        employer: employer_a,
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
        terms_hash: terms.digest()?,
        terms: terms.clone(),
    };
    let contract_new_same = CrewContract {
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 250,
        },
        ..contract_old.clone()
    };
    let contract_other_employer = CrewContract {
        employer: employer_b,
        ..contract_old.clone()
    };
    let mut out = mush_core::Outbox::new();
    {
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: employer_a,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        crate::field::crew::owe_npc_exit(
            &mut ctx,
            crate::field::NpcExit {
                format: crate::field::crew::NPC_EXIT_FORMAT,
                npc,
                organizer: employer_a,
                recovered_ic: 200,
            },
        );
    }
    check!(crate::field::crew::exit_applies(tw.world(), &contract_old));
    check!(
        !crate::field::crew::exit_applies(tw.world(), &contract_new_same),
        "stale exit must not destroy or hide a newer contract from the same employer"
    );
    check!(
        !crate::field::crew::exit_applies(tw.world(), &contract_other_employer),
        "stale exit must not hide or block a contract held by a different employer"
    );

    // Frozen IC & wall clocks must still get distinct monotonic sequences for idempotency keys.
    let s1 = crate::field::crew::next_exit_seq(tw.world_mut());
    let s2 = crate::field::crew::next_exit_seq(tw.world_mut());
    check!(s1 > 0 && s2 > s1);
    Ok(())
}

#[test]
fn p2_tsc_13_pending_npc_exit_blocks_personnel_check() -> TestResult {
    let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
        crate::TacsimRuleset::with_config(crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        }),
    )]);
    let carrier = tw.world_mut().create(
        mush_core::ObjKind::Thing,
        "Carrier",
        mush_core::GOD,
        Some(mush_core::LIMBO),
    )?;
    let npc = tw.world_mut().create(
        mush_core::ObjKind::Npc,
        "Mendes",
        mush_core::GOD,
        Some(mush_core::LIMBO),
    )?;
    let employer_a = tw.world_mut().create(
        mush_core::ObjKind::Player,
        "CaptainA",
        mush_core::GOD,
        Some(mush_core::LIMBO),
    )?;
    let employer_b = tw.world_mut().create(
        mush_core::ObjKind::Player,
        "CaptainB",
        mush_core::GOD,
        Some(mush_core::LIMBO),
    )?;
    let terms = HireTerms {
        schema: 1,
        port: CanonicalId::new("ceres")?,
        mission_scope: NpcMissionScope::SubordinateOperator,
        duties: BTreeSet::from([CrewDuty::Mining]),
    };
    let contract_old = CrewContract {
        schema: 1,
        npc,
        employer: employer_a,
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
        terms_hash: terms.digest()?,
        terms: terms.clone(),
    };
    let contract_new_same = CrewContract {
        hired_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 250,
        },
        ..contract_old.clone()
    };
    let contract_other_employer = CrewContract {
        employer: employer_b,
        ..contract_old.clone()
    };

    // Before an exit is owed, `contract_old` passes employment verification and
    // proceeds to the physical-containment check (`NotAboard`).
    check_eq!(
        personnel::check(
            tw.world(),
            employer_a,
            carrier,
            npc,
            false,
            Some(contract_old.clone()),
        ),
        Err(personnel::PersonnelIssue::NotAboard)
    );

    // Record an applicable `NpcExit` for `employer_a` at `recovered_ic = 200`.
    let mut out = mush_core::Outbox::new();
    {
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: employer_a,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        crate::field::crew::owe_npc_exit(
            &mut ctx,
            crate::field::NpcExit {
                format: crate::field::crew::NPC_EXIT_FORMAT,
                npc,
                organizer: employer_a,
                recovered_ic: 200,
            },
        );
    }

    // Applicable `NpcExit` immediately treats `contract_old` as ended BEFORE
    // checking whereabouts (`NotContracted`, not `NotAboard`).
    check_eq!(
        personnel::check(
            tw.world(),
            employer_a,
            carrier,
            npc,
            false,
            Some(contract_old),
        ),
        Err(personnel::PersonnelIssue::NotContracted)
    );
    // Newer rehire or different employer still passes the employment gate.
    check_eq!(
        personnel::check(
            tw.world(),
            employer_a,
            carrier,
            npc,
            false,
            Some(contract_new_same),
        ),
        Err(personnel::PersonnelIssue::NotAboard)
    );
    check_eq!(
        personnel::check(
            tw.world(),
            employer_b,
            carrier,
            npc,
            false,
            Some(contract_other_employer),
        ),
        Err(personnel::PersonnelIssue::NotAboard)
    );
    Ok(())
}
