//! Doc 17h cut M8a: the inspection policy's and record's own shape, off any world.
use super::*;
use mush_core::clock::{ClockRef, ClockStamp};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use records::{HullClass, InspectionRecord, Provenance, SealCheck, UngovernedNote};
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
        seq: 1,
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
        ungoverned_note: None,
        holder: None,
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
    // F9: keyed `hull/<hull>/slot/<seq mod 64>`, the ring slot (was the inspection commit's
    // hex).
    check_eq!(good.key(), "hull/0000004d/slot/01");
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
        ("seq zero", Box::new(|r| r.seq = 0)),
        ("operator", Box::new(|r| r.operator = ObjId(43))),
        // The M8a fix round's F4: no governor read is no hardware, so a core or a class
        // above 0 beside it is refused.
        (
            "governor missing beside a core",
            Box::new(|r| r.governed = None),
        ),
        (
            "no hardware at a class",
            Box::new(|r| {
                r.core = None;
                r.governed = None;
                r.provenance = Provenance::Unknown;
            }),
        ),
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
    // F4: a fitted core whose class cell reads 0 is still a core — a class-0 core with its
    // governor and its asset validates (and keeps its provenance).
    let mut zero = good.clone();
    zero.class = HullClass::Zero;
    check!(zero.validate().is_ok(), "a class-0 core is a core");
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

/// **The fix round's F9: 64 records a hull.** The key is `hull/<hull>/slot/<seq mod 64>`, so
/// the 65th inspection of a hull lands in its first's slot (the writer replaces it there,
/// evicting the oldest) and two hulls never share a slot; the ring's slots are the 64 keys of
/// one hull, each a key the family accepts.
#[test]
fn m8a_the_inspections_of_a_hull_ring_at_sixty_four() -> TestResult {
    check_eq!(INSPECTIONS_PER_HULL, 64);
    check_eq!(inspection_key(ObjId(77), 1), "hull/0000004d/slot/01");
    check_eq!(inspection_key(ObjId(77), 64), "hull/0000004d/slot/00");
    check_eq!(inspection_key(ObjId(77), 63), "hull/0000004d/slot/3f");
    check_eq!(
        inspection_key(ObjId(77), 65),
        inspection_key(ObjId(77), 1),
        "the 65th takes the first's slot"
    );
    check!(inspection_key(ObjId(7), 71) != inspection_key(ObjId(77), 1));
    let slots: Vec<String> = inspection_slots(ObjId(77)).collect();
    check_eq!(slots.len(), 64);
    check!(slots.windows(2).all(|pair| pair[0] < pair[1]), "{slots:?}");
    check!((1..=64).all(|seq| slots.contains(&inspection_key(ObjId(77), seq))));
    let good = record(&policy()?)?;
    check_eq!(good.key(), "hull/0000004d/slot/01");
    // Every slot key is one the family accepts; a slot past the ring, another family's key
    // shape, or hull 0 is not.
    check!(
        slots
            .iter()
            .all(|key| crate::commerce::families::valid_key(INSPECTION_FAMILY, key))
    );
    for bad in [
        "hull/0000004d/slot/40",
        "hull/00000000/slot/01",
        "03030303030303030303030303030303",
        "hull/4d/slot/01",
    ] {
        check!(
            !crate::commerce::families::valid_key(INSPECTION_FAMILY, bad),
            "{bad}"
        );
    }
    Ok(())
}

/// **Doc 17h cut M8b G11**: a record carrying the ungoverning refit's note round-trips; a
/// record with no core may not carry one.
#[test]
fn m8b_a_record_carries_the_ungoverned_note_only_with_a_core() -> TestResult {
    let policy = policy()?;
    let mut noted = record(&policy)?;
    noted.ungoverned_note = Some(UngovernedNote {
        facility: ObjId(9),
        at_ic: 12_345,
    });
    noted.validate()?;
    let value = serde_json::to_value(&noted)?;
    check_eq!(
        value.get("ungoverned_note").cloned().unwrap_or_default(),
        serde_json::json!({"facility": 9, "at_ic": 12_345})
    );
    check_eq!(serde_json::from_value::<InspectionRecord>(value)?, noted);
    check_eq!(noted.facts().ungoverned_note, noted.ungoverned_note);
    let mut coreless = noted.clone();
    coreless.core = None;
    coreless.governed = None;
    coreless.class = HullClass::Zero;
    check!(coreless.validate().is_err());
    Ok(())
}

/// One mutation of a valid claim that must make it invalid.
type ClaimChange = Box<dyn Fn(&mut ClaimRecord)>;

fn claim(policy: &RegistryInspectionPolicy) -> TestResult<ClaimRecord> {
    Ok(ClaimRecord {
        format: 1,
        claim: mush_core::CommitToken::from_bytes([5; 16]),
        action: policy.claim_action()?,
        authority: policy.clone(),
        author: ObjId(30),
        seq: 1,
        hulls: vec![ObjId(77), ObjId(91)],
        text: SmolStr::new("It pauses on a lost contact the way mine does."),
        at_ic: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 2_000,
        },
    })
}

/// **Doc 17h cut M8g G3: the claim's definitions** — the action and outcome under tacsim, the
/// action referencing the inspection's authority; the capability and family words.
#[test]
fn m8g_the_claim_names_its_definitions_under_tacsim() -> TestResult {
    let p = policy()?;
    let action = p.claim_action()?;
    let outcome = p.claim_outcome()?;
    check_eq!(
        (action.key().owner.as_str(), action.key().id.as_str()),
        (ACTION_OWNER, "mind_quirk_claim")
    );
    check_eq!(
        (outcome.key().owner.as_str(), outcome.key().id.as_str()),
        (ACTION_OWNER, "mind_quirk_claimed")
    );
    let [descriptor, _] = p.claim_descriptors()?;
    check_eq!(descriptor.references(), &[p.authority()?]);
    check_eq!(REGISTRY_CLAIMS_CAPABILITY, "roll.registry.claims");
    check_eq!(CLAIM_FAMILY, "roll_mind_claims");
    Ok(())
}

/// **Doc 17h cut M8g G3: the claim record** — valid as built, keyed in the author's 32-slot
/// ring (the 33rd claim takes the first's slot), round-trips its wire, and refuses every
/// malformed shape: no sequence or author, a foreign action, no hull, unsorted or repeated or
/// zero hulls, more than eight, and words that are empty, untrimmed, multi-line or too long.
#[test]
fn m8g_a_claim_record_is_bounded_and_strict() -> TestResult {
    let p = policy()?;
    let c = claim(&p)?;
    check!(c.validate().is_ok());
    check_eq!(c.key(), "author/0000001e/slot/01");
    check!(super::claims::valid_key(&c.key()));
    check!(
        !super::claims::valid_key("author/0000001e/slot/20"),
        "slot 32 is outside the ring"
    );
    check_eq!(claim_key(ObjId(30), 33), claim_key(ObjId(30), 1));
    check_eq!(claim_slots(ObjId(30)).count(), 32);
    check_eq!(CLAIMS_PER_AUTHOR, 32);
    let wire = serde_json::to_value(&c)?;
    check_eq!(serde_json::from_value::<ClaimRecord>(wire)?, c);
    let changes: Vec<ClaimChange> = vec![
        Box::new(|c| c.seq = 0),
        Box::new(|c| c.author = ObjId(0)),
        Box::new(|c| c.action = c.authority.action().unwrap_or_else(|_| c.action.clone())),
        Box::new(|c| c.hulls.clear()),
        Box::new(|c| c.hulls = vec![ObjId(91), ObjId(77)]),
        Box::new(|c| c.hulls = vec![ObjId(77), ObjId(77)]),
        Box::new(|c| c.hulls = vec![ObjId(0), ObjId(77)]),
        Box::new(|c| c.hulls = (1..=9).map(ObjId).collect()),
        Box::new(|c| c.text = SmolStr::new("")),
        Box::new(|c| c.text = SmolStr::new(" padded")),
        Box::new(|c| c.text = SmolStr::new("two\nlines")),
        Box::new(|c| c.text = SmolStr::new("x".repeat(MAX_CLAIM_TEXT + 1))),
    ];
    for (n, change) in changes.iter().enumerate() {
        let mut bad = c.clone();
        change(&mut bad);
        check!(bad.validate().is_err(), "change {n} still validates");
    }
    check_eq!(MAX_CLAIM_HULLS, 8);
    Ok(())
}

/// **Doc 17h cut M8g G3: `minds.claimed`** — a claim produces one fact per hull named,
/// `hull/<hull>` at the claim's sequence; the provider parses it as scoped (M8d K6's
/// ruling), so no public contract condition resolves it.
#[test]
fn m8g_a_claim_produces_one_scoped_fact_per_hull() -> TestResult {
    use mush_economy::FactProvider;
    let c = claim(&policy()?)?;
    let facts = c.facts();
    check_eq!(facts.len(), 2);
    let provider = super::super::facts::MindsFactProvider::new();
    let world = mush_core::World::new_minimal();
    for (fact, hull) in facts.iter().zip([77_u64, 91]) {
        check_eq!(fact.provider.as_str(), "minds");
        check_eq!(fact.kind.as_str(), "claimed");
        check_eq!(fact.id.as_str(), format!("hull/{hull:016x}"));
        check_eq!(fact.revision, Some(1));
        check!(provider.requires_scope(fact), "scoped");
        check!(
            provider
                .resolve(&world, "claimed", fact.id.as_str(), fact.revision)
                .is_none(),
            "no public resolve"
        );
    }
    Ok(())
}

/// **Doc 17h cut M8d2 S2: the seizure's proof.** An ungoverned newest inspection naming the
/// fitted core, within the age, proves the key `hull/<8 hex>/inspection/<seq>`, and so (M8b-2)
/// does a grey or heirloom one on a governed core; a governed certified one, a coreless one, a
/// stale one (or one on another clock) and one whose core has moved do not, each with its own
/// word.
#[test]
fn m8d2_the_newest_ungoverned_inspection_proves_a_seizure() -> TestResult {
    use seizure::{NotSeizable, hold_elapsed, prove};
    let p = policy()?;
    let mut r = record(&p)?;
    r.governed = Some(false);
    r.seq = 3;
    let at = |s: i64| ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: s,
    };
    let proof =
        prove(&r, Some(ObjId(78)), &at(1_000 + 3 * 86_400), 3).map_err(|e| format!("{e:?}"))?;
    check_eq!(proof.key.as_str(), "hull/0000004d/inspection/3");
    check_eq!(
        (proof.hull, proof.core, proof.seq),
        (ObjId(77), ObjId(78), 3)
    );
    check_eq!(
        prove(&r, Some(ObjId(78)), &at(1_001 + 3 * 86_400), 3),
        Err(NotSeizable::Stale)
    );
    check_eq!(
        prove(&r, Some(ObjId(78)), &at(999), 3),
        Err(NotSeizable::Stale)
    );
    let other_clock = ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 1_000,
    };
    check_eq!(
        prove(&r, Some(ObjId(78)), &other_clock, 3),
        Err(NotSeizable::Stale)
    );
    check_eq!(
        prove(&r, Some(ObjId(79)), &at(1_000), 3),
        Err(NotSeizable::CoreMoved)
    );
    check_eq!(prove(&r, None, &at(1_000), 3), Err(NotSeizable::CoreMoved));
    let mut governed = r.clone();
    governed.governed = Some(true);
    check_eq!(
        prove(&governed, Some(ObjId(78)), &at(1_000), 3),
        Err(NotSeizable::NothingToSeize)
    );
    // M8b-2: grey and heirloom are grounds of their own, governed or not.
    for word in [Provenance::Grey, Provenance::Heirloom] {
        let mut grey = governed.clone();
        grey.provenance = word;
        let proof = prove(&grey, Some(ObjId(78)), &at(1_000), 3).map_err(|e| format!("{e:?}"))?;
        check_eq!(proof.key.as_str(), "hull/0000004d/inspection/3");
    }
    let mut coreless = r.clone();
    coreless.governed = None;
    coreless.core = None;
    coreless.class = HullClass::Zero;
    coreless.provenance = Provenance::Unknown;
    check_eq!(
        prove(&coreless, None, &at(1_000), 3),
        Err(NotSeizable::NothingToSeize)
    );
    // The hold: three IC days from the seizure, to the second, on the same clock only.
    check!(!hold_elapsed(&at(1_000), &at(999 + 3 * 86_400), 3));
    check!(hold_elapsed(&at(1_000), &at(1_000 + 3 * 86_400), 3));
    check!(!hold_elapsed(&at(1_000), &other_clock, 0));
    Ok(())
}

/// **Doc 17h cut M8d2 S4: the seizure record's census.** A seized record validates; its forfeit
/// needs all three fields, not before the hold, and a commit of its own; a record seizing the
/// hull itself, or with the hold ending before the seizure, does not.
#[test]
fn m8d2_the_seizure_records_census() -> TestResult {
    use super::super::records::Record;
    let token = |n: u8| mush_core::CommitToken::from_bytes([n; 16]);
    let seized = SeizureRecord {
        format: 1,
        inspection: token(1),
        hull: ObjId(77),
        seq: 3,
        core: ObjId(78),
        seizure: token(2),
        at_ic: 1_000,
        forfeit_after_ic: 1_000 + 3 * 86_400,
        seized_by: ObjId(42),
        forfeit: None,
        forfeited_at_ic: None,
        forfeited_by: None,
    };
    seized.validate()?;
    check_eq!(seized.key(), "hull/0000004d/inspection/3");
    let forfeited = SeizureRecord {
        forfeit: Some(token(3)),
        forfeited_at_ic: Some(seized.forfeit_after_ic),
        forfeited_by: Some(ObjId(41)),
        ..seized.clone()
    };
    forfeited.validate()?;
    check!(forfeited.forfeited() && forfeited.seized() == seized);
    type SeizureChange = Box<dyn Fn(&mut SeizureRecord)>;
    let bad: [SeizureChange; 5] = [
        Box::new(|r| r.core = r.hull),
        Box::new(|r| r.forfeit_after_ic = r.at_ic - 1),
        Box::new(|r| r.forfeited_by = None),
        Box::new(|r| r.forfeited_at_ic = Some(r.forfeit_after_ic - 1)),
        Box::new(|r| r.forfeit = Some(r.seizure)),
    ];
    for change in bad {
        let mut record = forfeited.clone();
        change(&mut record);
        check!(record.validate().is_err());
    }
    Ok(())
}
