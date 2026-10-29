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
