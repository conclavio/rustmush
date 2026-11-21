#![cfg(test)]

//! Codec/registration evidence only. Genuine generation transitions are covered
//! by the additive Tacsim child using the existing measured certificate/grants.
use super::records::*;
use super::*;
use crate::commerce::{
    WorldIcSeconds,
    families::CommerceFamilies,
    filing::InitialFilingTerms,
    module::{CommerceConfig, CommerceMode},
    rating::RatingCompatibility,
    records::{OwnerLicence, OwnerTerm, Record},
};
use mush_core::{
    DefinitionDigest, GameModule, ModError, Registrar, Registry, StateFamilyHandle, World,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn manifest() -> DefinitionRef {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    reference("tacsim", manifest_descriptor())
}
fn evidence(parent: Option<ParentQualification>) -> AttestationEvidence {
    AttestationEvidence {
        term_seconds: 100,
        grace_seconds: 20,
        parent,
    }
}
fn operation() -> TestResult<AttestationOperation> {
    let d = DefinitionDigest::from_bytes([255; 32]);
    let token = CommitToken::from_bytes([255; 16]);
    let rev = |actor: u32| json!({"format":1,"revision":{"key":{"character":actor,"mind":u64::MAX,"number":u64::MAX},"digest":d},"source":d});
    let selected = json!({"licence":(u64::MAX-1).to_string(),"term":u64::MAX.to_string(),"filed":(u64::MAX-2).to_string(),"revision":rev(u32::MAX-2),
        "window":{"format":1,"eligibility_start":i64::MAX-100,"issued_at":i64::MAX-100,"term_end":i64::MAX-20,"grace_end":i64::MAX},"form":{"kind":"open_resale"},"operation":CommitToken::from_bytes([251;16]),"terminal":CommitToken::from_bytes([250;16])});
    let parent = json!({"source":{"filed":(u64::MAX-1).to_string(),"revision":rev(u32::MAX-1)},
        "charter":(u32::MAX-1).to_string(),"line":(u64::MAX-1).to_string(),"selected":selected});
    let next = json!({"format":4,"licence":u64::MAX.to_string(),"generation":u64::MAX.to_string(),"window":{"format":1,"eligibility_start":i64::MAX-100,"issued_at":i64::MAX-20,"term_end":i64::MAX-20,"grace_end":i64::MAX},
        "revision":{"kind":"follow","line":u64::MAX.to_string()},"operation":token,
        "attestation":{"term_seconds":100,"grace_seconds":20,"parent":parent}});
    Ok(serde_json::from_value(
        json!({"format":1,"operation":token,"action":protocol::action(&manifest(),100,20)?,"manifest":manifest(),
            "owner":u32::MAX,"licence":u64::MAX.to_string(),"charter":u32::MAX.to_string(),"line":u64::MAX.to_string(),
            "birth_operation":CommitToken::from_bytes([254;16]),"predecessor":(u64::MAX-1).to_string(),"predecessor_operation":CommitToken::from_bytes([253;16]),"predecessor_digest":d,
            "filed":u64::MAX.to_string(),"revision":rev(u32::MAX),"rating":{"schema":1,"job":CommitToken::from_bytes([252;16]),"id":d,"receipt_digest":d},
            "title_revision":u64::MAX,"observed_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX-20},"observed_utc":i64::MIN,"next":next}),
    )?)
}
#[test]
fn old_owner_term_wire_remains_exact_and_new_evidence_is_closed() -> TestResult {
    let old = json!({"format":1,"licence":"1","generation":"1","window":{"format":1,"eligibility_start":100,"issued_at":100,"term_end":200,"grace_end":220},
        "revision":{"kind":"follow","line":"1"},"operation":CommitToken::from_bytes([1;16])});
    let row: OwnerTerm = serde_json::from_value(old.clone())?;
    row.validate()?;
    check_eq!(serde_json::to_value(&row)?, old);
    let old_licence = json!({"format":1,"id":"1","charter":"1","line":"1","term":"1","operation":CommitToken::from_bytes([1;16])});
    let licence: OwnerLicence = serde_json::from_value(old_licence.clone())?;
    licence.validate()?;
    check_eq!(serde_json::to_value(&licence)?, old_licence);
    for (format, term) in [(1, 2), (2, 1), (3, 2)] {
        let mut bad = licence.clone();
        bad.format = format;
        bad.term = crate::commerce::TermGeneration::new(term)?;
        check!(bad.validate().is_err());
    }
    let mut null = old.clone();
    null.as_object_mut()
        .ok_or("old")?
        .insert("attestation".into(), Value::Null);
    check!(serde_json::from_value::<OwnerTerm>(null).is_err());
    let row = operation()?;
    for field in ["attestation", "generation", "operation"] {
        let mut v = serde_json::to_value(&row.next)?;
        v.as_object_mut().ok_or("term")?.remove(field);
        check!(serde_json::from_value::<OwnerTerm>(v).is_err());
    }
    let mut v = serde_json::to_value(&row.next)?;
    v.get_mut("attestation")
        .and_then(Value::as_object_mut)
        .ok_or("evidence")?
        .remove("parent");
    check!(serde_json::from_value::<OwnerTerm>(v).is_err());
    for field in ["permission", "source", "validated", "title"] {
        let mut v = serde_json::to_value(&row)?;
        v.as_object_mut()
            .ok_or("row")?
            .insert(field.into(), json!(true));
        check!(serde_json::from_value::<AttestationOperation>(v).is_err());
    }
    Ok(())
}
#[test]
fn attestation_window_preserves_parent_grace_and_never_extends_on_restoration() -> TestResult {
    let mut e = operation()?.next.attestation.ok_or("evidence")?;
    let parent = e.parent.as_mut().ok_or("parent")?;
    parent.selected.window = TermWindow::new(
        WorldIcSeconds(0),
        WorldIcSeconds(0),
        WorldIcSeconds(100),
        WorldIcSeconds(115),
    )?;
    let active = e.window(WorldIcSeconds(50))?;
    check_eq!(
        (active.term_end().get(), active.grace_end().get()),
        (100, 115)
    );
    let grace = e.window(WorldIcSeconds(105))?;
    check_eq!(grace.phase_at(WorldIcSeconds(105)), TermPhase::Grace);
    check_eq!(grace.issued_at(), WorldIcSeconds(105));
    check_eq!(grace.eligibility_start(), WorldIcSeconds(0));
    check!(e.window(WorldIcSeconds(115)).is_err());
    let original = evidence(None).window(WorldIcSeconds(105))?;
    check_eq!(
        (original.term_end().get(), original.grace_end().get()),
        (205, 225)
    );
    check_eq!(grace.grace_end(), WorldIcSeconds(115));
    check!(evidence(None).window(WorldIcSeconds(i64::MAX)).is_err());
    Ok(())
}
#[test]
fn cause_identity_predecessor_and_config_are_not_caller_substitutable() -> TestResult {
    let row = operation()?;
    for mode in 0..7 {
        let mut bad = row.clone();
        match mode {
            0 => bad.operation = bad.birth_operation,
            1 => bad.predecessor = bad.next.generation,
            2 => bad.next.operation = bad.birth_operation,
            3 => bad.owner = ObjId(1),
            4 => bad.title_revision = 0,
            5 => bad.action = protocol::action(&manifest(), 99, 20)?,
            _ => bad.observed_ic.at -= 1,
        }
        check!(bad.validate().is_err());
    }
    check_ne!(term_digest(&row.next)?, row.predecessor_digest);
    let roots = row.roots()?;
    check_eq!(roots.len(), 2);
    for root in &roots {
        check_eq!(RootAnchor::parse(&root.anchor.record())?, root.anchor);
    }
    check_ne!(
        roots.first().ok_or("root")?.anchor,
        roots.get(1).ok_or("parent root")?.anchor
    );
    Ok(())
}
type Handles = (
    StateFamilyHandle<String, AttestationOperation>,
    StateFamilyHandle<String, OwnerTerm>,
);
struct Rows(Arc<Mutex<Option<Handles>>>);
impl GameModule for Rows {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let f = CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("handles"))? =
            Some((f.owner_attestations, f.terms));
        Ok(())
    }
}
#[test]
fn maximum_complete_operation_and_term_fit_actual_registered_family_envelopes() -> TestResult {
    let shared = Arc::new(Mutex::new(None));
    let mut engine = boxed_test_engine(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (operations, terms) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let op = operation()?;
    for (owner, id, schema, key, value, max) in [
        (
            operations.owner(),
            operations.descriptor().id(),
            operations.descriptor().value_schema(),
            op.key(),
            serde_json::to_value(&op)?,
            AttestationOperation::MAX_BYTES,
        ),
        (
            terms.owner(),
            terms.descriptor().id(),
            terms.descriptor().value_schema(),
            op.next.key(),
            serde_json::to_value(&op.next)?,
            OwnerTerm::MAX_BYTES,
        ),
    ] {
        let document = json!({"family":{"owner":owner,"id":id},"key":key,"schema":{"id":schema,"version":1},"value":value});
        check!(mush_core::canonical_json_bytes_with_limit(&document, max)?.len() < max);
    }
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &op.key(), op.clone())?;
    txn.insert(&terms, &op.next.key(), op.next.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    let mut txn = engine.action_transaction(&operations)?;
    check!(txn.insert(&operations, &"not-a-commit".into(), op).is_err());
    Ok(())
}
#[test]
fn optional_attestation_requires_active_composition_and_binds_frozen_durations() -> TestResult {
    let make = |mode| {
        CommerceConfig::new(
            mode,
            InitialFilingTerms::new(100, 20).unwrap(),
            manifest(),
            RatingCompatibility::combat_v1(manifest().digest()),
        )
    };
    check!(
        make(CommerceMode::Initialize)?
            .with_owner_attestations()
            .is_err()
    );
    check!(
        make(CommerceMode::Active)?
            .with_owner_attestations()
            .is_ok()
    );
    check!(
        make(CommerceMode::Active)?
            .with_owner_attestations()?
            .with_owner_attestations()
            .is_err()
    );
    check_ne!(
        protocol::action(&manifest(), 100, 20)?,
        protocol::action(&manifest(), 101, 20)?
    );
    check!(protocol::definitions(&manifest(), 0, 20).is_err());
    Ok(())
}

use mush_core::CanonicalHash;
use mush_core::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, IdempotencyKey, IdempotencyScope,
    PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
fn invocation(parameters: Value) -> TestResult<ActionInvocation> {
    let uid = RequestUid::from_bytes([1; 16]);
    let actor = ObjId(2);
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request: uid,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(crate::NS)?,
                namespace: CanonicalId::new("owner_attestation_test")?,
            },
            1,
            IdempotencyKey::Request(uid),
        ),
    })?)
}

/// M8c F11: the House arm of the operation record. A House row names the
/// House action (`expected_action`), never the personal one, and the
/// personal arm never names the House action; a House row's owner is the
/// acting character, who need not be the filing's author; a House row never
/// carries a charter ownership (a House charter is never transferred).
#[test]
fn the_house_arm_binds_its_own_action_and_no_charter_ownership() -> TestResult {
    let personal = operation()?;
    personal.validate()?;
    let mut house = personal.clone();
    house.house = Some(HouseAttestation {
        organization: mush_org::OrgId(0),
        scope: mush_org::OrgScopeId(0),
    });
    check!(
        house.validate().is_err(),
        "a House row naming the personal action"
    );
    house.action = protocol::house_action(&manifest(), 100, 20)?;
    house.validate()?;
    house.owner = mush_core::ObjId(7);
    house.validate()?;
    let mut crossed = personal.clone();
    crossed.action = house.action.clone();
    check!(
        crossed.validate().is_err(),
        "a personal row naming the House action"
    );
    let mut stranger = personal.clone();
    stranger.owner = mush_core::ObjId(7);
    check!(
        stranger.validate().is_err(),
        "a personal row's owner is the filing's author"
    );
    // The House arm with a charter ownership, through the wire.
    let proof = json!({"generation":65537,"operation":CommitToken::from_bytes([249;16]),"digest":DefinitionDigest::from_bytes([255;32])});
    let mut value = serde_json::to_value(&house)?;
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("charter_ownership".into(), proof);
    fields.insert("format".into(), json!(2));
    check!(
        serde_json::from_value::<AttestationOperation>(value.clone()).is_err(),
        "a House row with a charter ownership"
    );
    value.as_object_mut().ok_or("operation")?.remove("house");
    value
        .as_object_mut()
        .ok_or("operation")?
        .insert("action".into(), serde_json::to_value(&personal.action)?);
    serde_json::from_value::<AttestationOperation>(value)?.validate()?;
    // The round trip keeps the House arm.
    let back: AttestationOperation = serde_json::from_value(serde_json::to_value(&house)?)?;
    check_eq!(back, house);
    // F10: the attested owner term takes the owner-term bound (365 days).
    let year = AttestationEvidence {
        term_seconds: crate::commerce::MAX_OWNER_TERM_SECONDS,
        grace_seconds: 20,
        parent: None,
    };
    let window = year.window(WorldIcSeconds(100))?;
    check_eq!(
        window.term_end().get() - window.issued_at().get(),
        crate::commerce::MAX_OWNER_TERM_SECONDS
    );
    let over = AttestationEvidence {
        term_seconds: crate::commerce::MAX_OWNER_TERM_SECONDS + 1,
        ..year
    };
    check!(over.window(WorldIcSeconds(100)).is_err());
    // F8: organisation 0 is an organisation.
    check_eq!(
        provider::Request::parse(&invocation(json!({"owner_licence":3,"org":0}))?)?.org(),
        Some(mush_org::OrgId(0))
    );
    Ok(())
}
#[test]
fn request_is_numeric_self_only_without_terms_or_parent_authority() -> TestResult {
    check_eq!(
        provider::Request::parse(&invocation(json!({"owner_licence":3}))?)?.licence()?,
        LicenceId::new(3)?
    );
    for value in [json!("3"), json!(0), json!(-1), json!(3.5), json!(null)] {
        check!(provider::Request::parse(&invocation(json!({"owner_licence":value}))?).is_err());
    }
    for key in [
        "actor",
        "parent_licence",
        "generation",
        "term_end",
        "source",
        "validated",
    ] {
        let mut p = json!({"owner_licence":3});
        p.as_object_mut()
            .ok_or("params")?
            .insert(key.into(), json!(1));
        check!(provider::Request::parse(&invocation(p)?).is_err());
    }
    Ok(())
}

/// These are strict data/size probes only; historical transfer authority comes
/// from the real native receipt and complete paired owner audit.
#[test]
fn charter_ownership_capture_is_versioned_and_legacy_bytes_remain_exact() -> TestResult {
    let old = operation()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: AttestationOperation = serde_json::from_slice(&bytes)?;
    check_eq!(serde_json::to_vec(&restored)?, bytes);
    let legacy = serde_json::to_value(&old)?;
    check!(legacy.get("charter_ownership").is_none());
    let proof = serde_json::json!({"generation":65537,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])});
    let mut value = legacy.clone();
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("charter_ownership".into(), proof.clone());
    check!(
        serde_json::from_value::<AttestationOperation>(value.clone()).is_err(),
        "old format cannot carry new authority attribution"
    );
    value
        .as_object_mut()
        .ok_or("operation")?
        .insert("format".into(), serde_json::json!(2));
    value
        .as_object_mut()
        .ok_or("operation")?
        .insert("owner".into(), serde_json::json!(1));
    let new: AttestationOperation = serde_json::from_value(value.clone())?;
    new.validate()?;
    check_eq!(serde_json::to_value(&new)?, value);
    let shared = Arc::new(Mutex::new(None));
    let mut engine = boxed_test_engine(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (operations, _) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let document = json!({"family":{"owner":operations.owner(),"id":operations.descriptor().id()},"key":new.key(),"schema":{"id":operations.descriptor().value_schema(),"version":1},"value":new});
    mush_core::canonical_json_bytes_with_limit(&document, AttestationOperation::MAX_BYTES)?;
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &new.key(), new.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    for corrupt in [
        serde_json::Value::Null,
        serde_json::json!({"generation":1,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])}),
        serde_json::json!({"generation":65538,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])}),
    ] {
        let mut bad = value.clone();
        bad.as_object_mut()
            .ok_or("operation")?
            .insert("charter_ownership".into(), corrupt);
        check!(serde_json::from_value::<AttestationOperation>(bad).is_err());
    }
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .ok_or("operation")?
        .remove("charter_ownership");
    check!(serde_json::from_value::<AttestationOperation>(missing).is_err());
    let mut extra = value.clone();
    extra
        .get_mut("charter_ownership")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("ownership")?
        .insert("validated".into(), serde_json::json!(true));
    check!(serde_json::from_value::<AttestationOperation>(extra).is_err());
    Ok(())
}

#[test]
fn attested_parent_holder_capture_is_in_the_term_and_complete_envelope() -> TestResult {
    let old = operation()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: AttestationOperation = serde_json::from_slice(&bytes)?;
    check_eq!(serde_json::to_vec(&restored)?, bytes);
    let mut wire = serde_json::to_value(&old)?;
    let selected = "/next/attestation/parent/selected";
    check!(wire.pointer(&format!("{selected}/holder_basis")).is_none());
    wire.pointer_mut(selected).and_then(Value::as_object_mut).ok_or("selected")?.insert(
        "holder_basis".into(), json!({"format":1,"acceptance":CommitToken::from_bytes([249;16]),
        "ownership":{"generation":65537,"operation":CommitToken::from_bytes([248;16]),"digest":DefinitionDigest::from_bytes([255;32])}}));
    let captured: AttestationOperation = serde_json::from_value(wire.clone())?;
    check_ne!(term_digest(&old.next)?, term_digest(&captured.next)?);
    check_eq!(serde_json::to_value(&captured)?, wire);
    let shared = Arc::new(Mutex::new(None));
    let mut engine = boxed_test_engine(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (operations, terms) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    for (family, schema, key, value, max) in [
        (
            operations.descriptor().id(),
            operations.descriptor().value_schema(),
            captured.key(),
            serde_json::to_value(&captured)?,
            AttestationOperation::MAX_BYTES,
        ),
        (
            terms.descriptor().id(),
            terms.descriptor().value_schema(),
            captured.next.key(),
            serde_json::to_value(&captured.next)?,
            OwnerTerm::MAX_BYTES,
        ),
    ] {
        let document = json!({"family":{"owner":operations.owner(),"id":family},"key":key,"schema":{"id":schema,"version":1},"value":value});
        mush_core::canonical_json_bytes_with_limit(&document, max)?;
    }
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &captured.key(), captured.clone())?;
    txn.insert(&terms, &captured.next.key(), captured.next)?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    *wire
        .pointer_mut(&format!("{selected}/holder_basis"))
        .ok_or("basis")? = Value::Null;
    check!(serde_json::from_value::<AttestationOperation>(wire).is_err());
    Ok(())
}

/// **P2-MOD-19**: a House attestation committed by an authorized House officer
/// or `tacsim.house_attestation` agent (`actor != birth.requester`) writes
/// `house: Some(...)`, `charter_ownership: None`, `owner: actor` and MUST pass
/// all five `owner_attestation::history` verification helpers (`current`,
/// `current_in_txn`, `historical`, `historical_in_txn`, `verify`) and Roll boot
/// audit (`history::audit`), while non-House (personal) attestations MUST still
/// enforce `charter_transfer::history::require_captured(_in_txn)`.
#[test]
fn p2_mod_19_house_attestation_by_other_officer_passes_all_readers_and_boot_audit() -> TestResult {
    use crate::commerce::{
        CharterId, FiledRevisionId, LineId, RevisionIdentity, TermGeneration,
        charter_transfer::records::CharterOwnershipRef,
        filed::{FiledRevisionContent, FilingPosition, FilingSequence},
        publication_tests::{open_gate_for_owner_unit_test, saved},
        rating::{RatingClass, RatingFamilies, RatingSubject, test_support::complete_rating},
        records::{
            FiledRecord, FilingEconomicReceipt, InitialFilingOperation, PUBLICATION_FORMAT,
            RevisionPolicy, canonical_duplicate_digest, schema_line_digest,
        },
    };
    use crate::{Envelope, FORMAT, ReferenceRecord, RevisionAdapter, RollError, RollStore};
    use mush_core::{ClockRef, ClockStamp, ModuleBootData, ModuleCtx, Outbox};
    use mush_economy::{AccountId, Amount, TransactionId};
    use mush_org::{OrgId, OrgScopeId, PrincipalId as OrgPrincipal, SystemPrincipalId};

    struct TestLanguage;
    impl RevisionAdapter for TestLanguage {
        fn normalize_source(&self, source: &str) -> Result<String, RollError> {
            Ok(source.into())
        }
        fn validate(&self, _: &str, _: &Envelope) -> Result<(), RollError> {
            Ok(())
        }
    }

    let (_, sources, base_disk, revision) = saved();
    let curator = ObjId(10); // matches `saved()`'s `ACTOR`
    let officer = ObjId(99); // different authorised officer / agent (`actor != birth.requester`)
    let org = OrgId(4);
    let scope = OrgScopeId(6);
    let stamp = |at: i64| ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    };
    let wic = |at: i64| WorldIcSeconds::from_stamp(&stamp(at)).unwrap();

    let identity = RevisionIdentity::capture(&revision)?;
    let definition = DefinitionDigest::from_bytes([0x44; 32]);
    let compat = RatingCompatibility::combat_v1(manifest().digest());
    let subject = RatingSubject::new(
        identity,
        revision.envelope.digest("fn think() {}")?,
        schema_line_digest(&revision.envelope)?,
        definition,
        definition,
        RatingClass::I,
    );
    let rating_job = CommitToken::from_bytes([0x12; 16]);
    let complete = complete_rating(
        subject,
        compat,
        DefinitionDigest::from_bytes([0x55; 32]),
        rating_job,
        stamp(100),
    )?;
    let rating_ref = complete.receipt.reference()?;

    let charter = CharterId::new(20)?;
    let line = LineId::new(1)?;
    let filed_id = FiledRevisionId::new(1)?;
    let licence_id = LicenceId::new(1)?;
    let gen1 = TermGeneration::new(1)?;
    let gen2 = TermGeneration::new(2)?;
    let birth_commit = CommitToken::from_bytes([0x10; 16]);
    let attest_commit = CommitToken::from_bytes([0x20; 16]);

    let filed = FiledRecord {
        format: PUBLICATION_FORMAT,
        content: FiledRevisionContent::capture(
            filed_id,
            FilingPosition {
                charter,
                line,
                sequence: FilingSequence::new(1)?,
            },
            &revision,
            wic(100),
            None,
        )?,
        definition,
        duplicate: canonical_duplicate_digest(&revision)?,
        rating: rating_ref,
        operation: birth_commit,
    };
    let term1 = OwnerTerm {
        attestation: None,
        parent: None,
        format: PUBLICATION_FORMAT,
        licence: licence_id,
        generation: gen1,
        window: TermWindow::new(wic(100), wic(100), wic(200), wic(220))?,
        revision: RevisionPolicy::Follow { line },
        operation: birth_commit,
    };
    let ev = evidence(None);
    let term2 = OwnerTerm {
        attestation: Some(ev.clone()),
        parent: None,
        format: 3,
        licence: licence_id,
        generation: gen2,
        window: ev.window(wic(205))?,
        revision: RevisionPolicy::Follow { line },
        operation: attest_commit,
    };
    let licence = OwnerLicence {
        format: 2,
        id: licence_id,
        charter,
        line,
        term: gen2,
        operation: birth_commit,
    };

    let build_birth = |house: bool| -> TestResult<InitialFilingOperation> {
        let (format, issued_owner, house_attr) = if house {
            (
                2,
                OrgPrincipal::Organization(org),
                Some(mush_economy::registry_filing::HouseFilingAttribution::new(
                    curator, org, scope,
                )?),
            )
        } else {
            (PUBLICATION_FORMAT, OrgPrincipal::Object(curator), None)
        };
        Ok(InitialFilingOperation {
            parent: None,
            format,
            commit: birth_commit,
            requester: curator,
            shell: serde_json::from_value(json!({
                "requester": curator,
                "request": RequestUid::from_bytes([1; 16]),
            }))?,
            shell_receipt: CommitToken::from_bytes([0x01; 16]),
            charter,
            line,
            filed: filed_id,
            licence: licence_id,
            term: gen1,
            revision: identity,
            rating: rating_ref,
            economic: FilingEconomicReceipt {
                house: house_attr,
                transaction: TransactionId(1),
                journal_seq: 1,
                title: charter.object(),
                issued_owner,
                title_revision: 1,
                payer: AccountId(1),
                sink: AccountId(2),
                fee: Amount::ZERO,
                issuer: SystemPrincipalId::new("test.registry").ok_or("issuer")?,
                catalog: mush_economy::CatalogRef {
                    ruleset: "roll".into(),
                    code: "mind_charter".into(),
                },
            },
        })
    };

    #[derive(Default)]
    struct AuditProbe {
        current_ok: Option<bool>,
        historical_ok: Option<bool>,
        verify_ok: Option<bool>,
        audit_count: Option<Result<u64, String>>,
    }

    type FixtureHandles = (CommerceFamilies, RatingFamilies);
    struct FixtureMod {
        handles: Arc<Mutex<Option<FixtureHandles>>>,
        store: Option<RollStore>,
        birth: InitialFilingOperation,
        op: AttestationOperation,
        licence: OwnerLicence,
        gen2: TermGeneration,
        term2: OwnerTerm,
        probe: Arc<Mutex<AuditProbe>>,
    }
    impl GameModule for FixtureMod {
        fn name(&self) -> &'static str {
            "roll"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let f = CommerceFamilies::register(reg)?;
            let r = RatingFamilies::register(reg)?;
            *self.handles.lock().map_err(|_| ModError::msg("lock"))? = Some((f, r));
            if let Some(store) = self.store.take() {
                reg.state(store);
            }
            Ok(())
        }
        fn on_world_loaded(
            &mut self,
            ctx: &mut ModuleCtx<'_>,
            _: ModuleBootData,
        ) -> Result<(), ModError> {
            let guard = self.handles.lock().map_err(|_| ModError::msg("lock"))?;
            let Some((f, ratings)) = guard.as_ref() else {
                return Err(ModError::msg("handles"));
            };
            let roll = ctx
                .w
                .module_state::<RollStore>()
                .ok_or_else(|| ModError::msg("roll store"))?;
            let mut probe = self.probe.lock().map_err(|_| ModError::msg("probe"))?;
            probe.current_ok =
                Some(history::current(ctx, roll, f, &self.licence).is_ok_and(|t| t == self.term2));
            probe.historical_ok = Some(
                history::historical(ctx, roll, f, &self.birth, self.gen2)
                    .is_ok_and(|t| t == self.term2),
            );
            probe.verify_ok = Some(history::verify(ctx, roll, f, ratings, &self.op).is_ok());
            probe.audit_count =
                Some(history::audit(ctx, roll, f, ratings, true).map_err(|e| format!("{e:?}")));
            Ok(())
        }
    }

    let check_case = |birth: &InitialFilingOperation,
                      op: &AttestationOperation,
                      expect_ok: bool|
     -> TestResult {
        let mut disk = base_disk.clone();
        for required in op.roots()? {
            let rec = ReferenceRecord {
                format: FORMAT,
                key: required.anchor.key(),
                revisions: vec![required.target()],
            };
            disk.insert(rec.key.key(), serde_json::to_value(&rec)?);
        }
        let boot = ModuleBootData {
            kv: disk
                .iter()
                .map(|(k, v)| (k.as_str().into(), v.clone()))
                .collect(),
            ..Default::default()
        };
        let mut store = RollStore::load_commerce(&boot)?;
        store.audit_sources(&TestLanguage, &sources)?;
        open_gate_for_owner_unit_test(&mut store);

        let shared = Arc::new(Mutex::new(None));
        let probe = Arc::new(Mutex::new(AuditProbe::default()));
        let mut engine = boxed_test_engine(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(FixtureMod {
                handles: shared.clone(),
                store: Some(store),
                birth: birth.clone(),
                op: op.clone(),
                licence: licence.clone(),
                gen2,
                term2: term2.clone(),
                probe: probe.clone(),
            })],
        )?;
        let (f, ratings) = shared
            .lock()
            .map_err(|_| "lock")?
            .clone()
            .ok_or("handles")?;
        let mut txn = engine.action_transaction(&f.licences)?;
        txn.insert(&f.operations, &birth.key(), birth.clone())?;
        txn.insert(&f.filed, &filed.key(), filed.clone())?;
        txn.insert(&f.terms, &term1.key(), term1.clone())?;
        txn.insert(&f.terms, &term2.key(), term2.clone())?;
        txn.insert(&f.licences, &licence.key(), licence.clone())?;
        txn.insert(&f.owner_attestations, &op.key(), op.clone())?;
        txn.insert(&ratings.receipts, &rating_job, complete.receipt.clone())?;
        let (_, install) = txn.prepare()?.into_parts();
        engine.install_prepared_action(install)?;

        // Fire real `on_world_loaded` with a real `ModuleCtx` to test `current`,
        // `historical`, `verify`, and `audit`.
        engine.modules_loaded(
            std::collections::HashMap::new(),
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        )?;
        {
            let p = probe.lock().map_err(|_| "probe")?;
            check_eq!(p.current_ok, Some(expect_ok), "history::current");
            check_eq!(p.historical_ok, Some(expect_ok), "history::historical");
            check_eq!(p.verify_ok, Some(expect_ok), "history::verify");
            if expect_ok {
                check_eq!(p.audit_count, Some(Ok(1)), "history::audit");
            } else {
                check!(
                    matches!(p.audit_count, Some(Err(_))),
                    "history::audit must fail"
                );
            }
        }

        // Also test the two transactional helpers: `current_in_txn` and `historical_in_txn`.
        let world_snap = engine.world.clone();
        let mut txn = engine.action_transaction(&f.licences)?;
        let cur_txn = history::current_in_txn(&mut txn, &world_snap, &f, &licence);
        let hist_txn = history::historical_in_txn(&mut txn, &world_snap, &f, birth, gen2);
        if expect_ok {
            check_eq!(cur_txn?, term2);
            check_eq!(hist_txn?, term2);
        } else {
            check!(cur_txn.is_err(), "current_in_txn must reject");
            check!(hist_txn.is_err(), "historical_in_txn must reject");
        }
        Ok(())
    };

    // 1. House attestation by `officer != birth.requester` MUST PASS all 5
    //    history readers and full Roll boot audit!
    let house_birth = build_birth(true)?;
    let house_op = AttestationOperation {
        format: 1,
        charter_ownership: None,
        house: Some(HouseAttestation {
            organization: org,
            scope,
        }),
        operation: attest_commit,
        action: protocol::house_action(&manifest(), 100, 20)?,
        manifest: manifest(),
        owner: officer,
        licence: licence_id,
        charter,
        line,
        birth_operation: birth_commit,
        predecessor: gen1,
        predecessor_operation: birth_commit,
        predecessor_digest: term_digest(&term1)?,
        filed: filed_id,
        revision: identity,
        rating: rating_ref,
        title_revision: 1,
        observed_ic: stamp(205),
        observed_utc: 205,
        next: term2.clone(),
    };
    house_op.follows(&term1, &house_birth)?;
    check_case(&house_birth, &house_op, true)?;

    // 2. Personal (non-House) attestation by `owner == birth.requester` passes.
    let personal_birth = build_birth(false)?;
    let personal_ok = AttestationOperation {
        format: 1,
        charter_ownership: None,
        house: None,
        action: protocol::action(&manifest(), 100, 20)?,
        owner: curator,
        ..house_op.clone()
    };
    personal_ok.follows(&term1, &personal_birth)?;
    check_case(&personal_birth, &personal_ok, true)?;

    // 3. Personal (non-House) attestation by `officer != birth.requester` with
    //    an unbacked `charter_ownership` MUST STILL BE REJECTED by all 5 helpers
    //    (`require_captured` / `require_captured_in_txn`) and `history::audit`!
    let forged_ownership: CharterOwnershipRef = serde_json::from_value(json!({
        "generation": 65537,
        "operation": CommitToken::from_bytes([0x99; 16]),
        "digest": DefinitionDigest::from_bytes([0x88; 32]),
    }))?;
    let personal_forged = AttestationOperation {
        format: 2,
        charter_ownership: Some(forged_ownership),
        house: None,
        action: protocol::action(&manifest(), 100, 20)?,
        owner: officer,
        ..house_op
    };
    personal_forged.follows(&term1, &personal_birth)?;
    check_case(&personal_birth, &personal_forged, false)?;
    Ok(())
}

use mush_testkit::fixtures::boxed_engine as boxed_test_engine;
