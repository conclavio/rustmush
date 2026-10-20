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
    DefinitionDigest, Engine, GameModule, ModError, Registrar, Registry, StateFamilyHandle, World,
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
    assert_eq!(serde_json::to_value(&row)?, old);
    let old_licence = json!({"format":1,"id":"1","charter":"1","line":"1","term":"1","operation":CommitToken::from_bytes([1;16])});
    let licence: OwnerLicence = serde_json::from_value(old_licence.clone())?;
    licence.validate()?;
    assert_eq!(serde_json::to_value(&licence)?, old_licence);
    for (format, term) in [(1, 2), (2, 1), (3, 2)] {
        let mut bad = licence.clone();
        bad.format = format;
        bad.term = crate::commerce::TermGeneration::new(term)?;
        assert!(bad.validate().is_err());
    }
    let mut null = old.clone();
    null.as_object_mut()
        .ok_or("old")?
        .insert("attestation".into(), Value::Null);
    assert!(serde_json::from_value::<OwnerTerm>(null).is_err());
    let row = operation()?;
    for field in ["attestation", "generation", "operation"] {
        let mut v = serde_json::to_value(&row.next)?;
        v.as_object_mut().ok_or("term")?.remove(field);
        assert!(serde_json::from_value::<OwnerTerm>(v).is_err());
    }
    let mut v = serde_json::to_value(&row.next)?;
    v.get_mut("attestation")
        .and_then(Value::as_object_mut)
        .ok_or("evidence")?
        .remove("parent");
    assert!(serde_json::from_value::<OwnerTerm>(v).is_err());
    for field in ["permission", "source", "validated", "title"] {
        let mut v = serde_json::to_value(&row)?;
        v.as_object_mut()
            .ok_or("row")?
            .insert(field.into(), json!(true));
        assert!(serde_json::from_value::<AttestationOperation>(v).is_err());
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
    assert_eq!(
        (active.term_end().get(), active.grace_end().get()),
        (100, 115)
    );
    let grace = e.window(WorldIcSeconds(105))?;
    assert_eq!(grace.phase_at(WorldIcSeconds(105)), TermPhase::Grace);
    assert_eq!(grace.issued_at(), WorldIcSeconds(105));
    assert_eq!(grace.eligibility_start(), WorldIcSeconds(0));
    assert!(e.window(WorldIcSeconds(115)).is_err());
    let original = evidence(None).window(WorldIcSeconds(105))?;
    assert_eq!(
        (original.term_end().get(), original.grace_end().get()),
        (205, 225)
    );
    assert_eq!(grace.grace_end(), WorldIcSeconds(115));
    assert!(evidence(None).window(WorldIcSeconds(i64::MAX)).is_err());
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
        assert!(bad.validate().is_err());
    }
    assert_ne!(term_digest(&row.next)?, row.predecessor_digest);
    let roots = row.roots()?;
    assert_eq!(roots.len(), 2);
    for root in &roots {
        assert_eq!(RootAnchor::parse(&root.anchor.record())?, root.anchor);
    }
    assert_ne!(
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
    let mut engine = Engine::with_modules(
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
        assert!(mush_core::canonical_json_bytes_with_limit(&document, max)?.len() < max);
    }
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &op.key(), op.clone())?;
    txn.insert(&terms, &op.next.key(), op.next.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    let mut txn = engine.action_transaction(&operations)?;
    assert!(txn.insert(&operations, &"not-a-commit".into(), op).is_err());
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
    assert!(
        make(CommerceMode::Initialize)?
            .with_owner_attestations()
            .is_err()
    );
    assert!(
        make(CommerceMode::Active)?
            .with_owner_attestations()
            .is_ok()
    );
    assert!(
        make(CommerceMode::Active)?
            .with_owner_attestations()?
            .with_owner_attestations()
            .is_err()
    );
    assert_ne!(
        protocol::action(&manifest(), 100, 20)?,
        protocol::action(&manifest(), 101, 20)?
    );
    assert!(protocol::definitions(&manifest(), 0, 20).is_err());
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

#[test]
fn request_is_numeric_self_only_without_terms_or_parent_authority() -> TestResult {
    assert_eq!(
        provider::Request::parse(&invocation(json!({"owner_licence":3}))?)?.licence()?,
        LicenceId::new(3)?
    );
    for value in [json!("3"), json!(0), json!(-1), json!(3.5), json!(null)] {
        assert!(provider::Request::parse(&invocation(json!({"owner_licence":value}))?).is_err());
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
        assert!(provider::Request::parse(&invocation(p)?).is_err());
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
    assert_eq!(serde_json::to_vec(&restored)?, bytes);
    let legacy = serde_json::to_value(&old)?;
    assert!(legacy.get("charter_ownership").is_none());
    let proof = serde_json::json!({"generation":65537,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])});
    let mut value = legacy.clone();
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("charter_ownership".into(), proof.clone());
    assert!(
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
    assert_eq!(serde_json::to_value(&new)?, value);
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
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
        assert!(serde_json::from_value::<AttestationOperation>(bad).is_err());
    }
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .ok_or("operation")?
        .remove("charter_ownership");
    assert!(serde_json::from_value::<AttestationOperation>(missing).is_err());
    let mut extra = value.clone();
    extra
        .get_mut("charter_ownership")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("ownership")?
        .insert("validated".into(), serde_json::json!(true));
    assert!(serde_json::from_value::<AttestationOperation>(extra).is_err());
    Ok(())
}

#[test]
fn attested_parent_holder_capture_is_in_the_term_and_complete_envelope() -> TestResult {
    let old = operation()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: AttestationOperation = serde_json::from_slice(&bytes)?;
    assert_eq!(serde_json::to_vec(&restored)?, bytes);
    let mut wire = serde_json::to_value(&old)?;
    let selected = "/next/attestation/parent/selected";
    assert!(wire.pointer(&format!("{selected}/holder_basis")).is_none());
    wire.pointer_mut(selected).and_then(Value::as_object_mut).ok_or("selected")?.insert(
        "holder_basis".into(), json!({"format":1,"acceptance":CommitToken::from_bytes([249;16]),
        "ownership":{"generation":65537,"operation":CommitToken::from_bytes([248;16]),"digest":DefinitionDigest::from_bytes([255;32])}}));
    let captured: AttestationOperation = serde_json::from_value(wire.clone())?;
    assert_ne!(term_digest(&old.next)?, term_digest(&captured.next)?);
    assert_eq!(serde_json::to_value(&captured)?, wire);
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
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
    assert!(serde_json::from_value::<AttestationOperation>(wire).is_err());
    Ok(())
}
