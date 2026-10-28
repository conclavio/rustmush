//! Doc 17h cut M8a: the inspection policy's and record's own shape, off any world.
use super::*;
use mush_core::clock::{ClockRef, ClockStamp};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use records::{HullClass, InspectionRecord, Provenance, SealCheck};
use smol_str::SmolStr;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// One mutation of a valid record that must make it invalid.
type Change = Box<dyn Fn(&mut InspectionRecord)>;

fn issuer() -> TestResult<SystemPrincipalId> {
    Ok(SystemPrincipalId::new("roll.registry").ok_or("issuer")?)
}

fn policy() -> TestResult<RegistryInspectionPolicy> {
    Ok(RegistryInspectionPolicy::new(
        issuer()?,
        OrgId(6),
        OrgScopeId(9),
        vec![ObjId(41), ObjId(42)],
    )?)
}

fn record(policy: &RegistryInspectionPolicy) -> TestResult<InspectionRecord> {
    Ok(InspectionRecord {
        format: 1,
        inspection: mush_core::CommitToken::from_bytes([3; 16]),
        action: policy.action()?,
        authority: policy.clone(),
        operator: ObjId(42),
        authorization: authority::Authorization {
            at: 1_000,
            organization_revision: 2,
            scope_revision: 3,
            sources: vec![authority::Source {
                tag: "grant".into(),
                revision: 1,
            }],
        },
        hull: ObjId(77),
        core: Some(ObjId(78)),
        class: HullClass::II,
        governed: Some(true),
        provenance: Provenance::Certified {
            importer: OrgId(5),
            grade: mush_economy::core_integration::CoreClass::II,
        },
        seal_digest: Some(SmolStr::new("a".repeat(64))),
        seal_check: SealCheck::Matches,
        licence_state: Some(super::super::SealLicenceState::Active),
        at_ic: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 1_000,
        },
    })
}

/// **The policy** names three definitions: the authority under the Roll, the action and
/// outcome under tacsim ([`ACTION_OWNER`]), the action referencing the authority; it refuses
/// an empty, unsorted or oversized operator list; it round-trips through its wire shape.
#[test]
fn m8a_the_policy_names_its_definitions_under_two_owners() -> TestResult {
    let p = policy()?;
    let authority = p.authority()?;
    let action = p.action()?;
    let outcome = p.outcome()?;
    check_eq!(
        (
            authority.key().owner.as_str(),
            authority.key().kind.as_str(),
            authority.key().id.as_str()
        ),
        ("roll", "registry_authority", "mind_inspection")
    );
    check_eq!(
        (
            action.key().owner.as_str(),
            action.key().kind.as_str(),
            action.key().id.as_str()
        ),
        (ACTION_OWNER, "action", "registry_mind_inspection")
    );
    check_eq!(
        (outcome.key().owner.as_str(), outcome.key().id.as_str()),
        (ACTION_OWNER, "registry_mind_inspected")
    );
    let [_, action_descriptor, _] = p.definitions()?;
    check_eq!(action_descriptor.references(), &[authority]);
    check_eq!(p.tacsim_descriptors()?.len(), 2);
    check_eq!(p.operators(), [ObjId(41), ObjId(42)]);
    check_eq!(p.scope(), OrgScopeId(9));
    check_eq!(REGISTRY_INSPECTION_CAPABILITY, "roll.registry.inspect");
    check_eq!(INSPECTION_FAMILY, "roll_registry_inspections");
    for operators in [
        vec![],
        vec![ObjId(42), ObjId(41)],
        vec![ObjId(41), ObjId(41)],
    ] {
        check!(
            RegistryInspectionPolicy::new(issuer()?, OrgId(6), OrgScopeId(9), operators).is_err()
        );
    }
    check!(
        RegistryInspectionPolicy::new(
            issuer()?,
            OrgId(6),
            OrgScopeId(9),
            (1..=33).map(ObjId).collect()
        )
        .is_err()
    );
    let wire = serde_json::to_value(&p)?;
    check_eq!(wire["format"], 1);
    check_eq!(wire["operators"], serde_json::json!([41, 42]));
    check_eq!(serde_json::from_value::<RegistryInspectionPolicy>(wire)?, p);
    check!(
        serde_json::from_value::<RegistryInspectionPolicy>(serde_json::json!({
            "format": 1, "issuer": "roll.registry", "organization": 6, "scope": 9,
            "operators": [42, 41]
        }))
        .is_err(),
        "an unsorted wire list is refused at decode"
    );
    Ok(())
}

/// **The record** validates its shape: the action is the authority's, the operator one of
/// its; the ROM carries no core, no governor and no certificate; a non-ROM names its governor;
/// a seal check and a digest come together, the state only with a digest, the digest sixty-four
/// lowercase hex; the wire words are `kind`-tagged snake case; it round-trips.
#[test]
fn m8a_the_record_validates_its_shape_and_words() -> TestResult {
    let p = policy()?;
    let good = record(&p)?;
    check!(good.validate().is_ok());
    check_eq!(good.key(), "03".repeat(16));
    let wire = serde_json::to_value(&good)?;
    check_eq!(wire["class"], "II");
    check_eq!(wire["seal_check"], "matches");
    check_eq!(wire["licence_state"], "active");
    // The grade is the economy's own `CoreClass` word (`snake_case` of `II`), one truth
    // with the certified die's class on the inventory lot.
    check_eq!(
        wire["provenance"],
        serde_json::json!({"kind": "certified", "importer": 5, "grade": "i_i"})
    );
    check_eq!(serde_json::from_value::<InspectionRecord>(wire)?, good);
    let facts = good.facts();
    check_eq!(
        (facts.hull, facts.core, facts.class),
        (ObjId(77), Some(ObjId(78)), HullClass::II)
    );

    let bad: Vec<(&str, Change)> = vec![
        ("format", Box::new(|r| r.format = 2)),
        ("operator", Box::new(|r| r.operator = ObjId(43))),
        ("rom with a core", Box::new(|r| r.class = HullClass::Zero)),
        ("governor missing", Box::new(|r| r.governed = None)),
        (
            "digest without a seal",
            Box::new(|r| r.seal_check = SealCheck::NoSeal),
        ),
        ("seal without a digest", Box::new(|r| r.seal_digest = None)),
        (
            "digest not hex",
            Box::new(|r| r.seal_digest = Some("G".repeat(64).into())),
        ),
        (
            "digest short",
            Box::new(|r| r.seal_digest = Some("a".repeat(63).into())),
        ),
        (
            "state without a digest",
            Box::new(|r| {
                r.seal_digest = None;
                r.seal_check = SealCheck::NoSeal;
            }),
        ),
        (
            "rom certified",
            Box::new(|r| {
                r.class = HullClass::Zero;
                r.core = None;
                r.governed = None;
            }),
        ),
        (
            "a clock that is not the world's",
            Box::new(|r| r.at_ic.clock = ClockRef::WorldIc { generation: 0 }),
        ),
    ];
    for (why, change) in bad {
        let mut r = good.clone();
        change(&mut r);
        check!(r.validate().is_err(), "{why}");
        let wire = serde_json::to_value(&r)?;
        check!(
            serde_json::from_value::<InspectionRecord>(wire).is_err(),
            "{why} is refused at decode"
        );
    }
    // The ROM, properly: no core, no governor, unknown, no seal.
    let mut rom = good.clone();
    rom.class = HullClass::Zero;
    rom.core = None;
    rom.governed = None;
    rom.provenance = Provenance::Unknown;
    rom.seal_digest = None;
    rom.seal_check = SealCheck::NoSeal;
    rom.licence_state = None;
    check!(rom.validate().is_ok());
    let wire = serde_json::to_value(&rom)?;
    check_eq!(wire["class"], "0");
    check_eq!(wire["provenance"], serde_json::json!({"kind": "unknown"}));
    check!(wire.get("seal_digest").is_none());
    check!(wire.get("licence_state").is_none());
    Ok(())
}
