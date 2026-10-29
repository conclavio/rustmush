#![cfg(test)]

//! Data/registration bounds only. The Tacsim child uses actual purchased grants.
use super::records::{QualificationHead, QualificationOperation};
use super::*;
use crate::commerce::{families::CommerceFamilies, records::Record};
use mush_core::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, IdempotencyKey, IdempotencyScope,
    PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::{
    CanonicalHash, DefinitionDigest, Engine, GameModule, ModError, Registrar, Registry,
    StateFamilyHandle,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn manifest() -> DefinitionRef {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    reference("tacsim", manifest_descriptor())
}
fn row() -> TestResult<QualificationOperation> {
    let digest = DefinitionDigest::from_bytes([255; 32]);
    let revision = |actor: u32, mind: u64| json!({"format":1,"revision":{"key":{"character":actor,"mind":mind,"number":u64::MAX},"digest":digest},"source":digest});
    Ok(serde_json::from_value(json!({
        "format":1,"operation":CommitToken::from_bytes([255;16]),"action":protocol::action(&manifest())?,"manifest":manifest(),
        "owner":u32::MAX,"owner_licence":u64::MAX.to_string(),"owner_term":"1","charter":u32::MAX.to_string(),"line":u64::MAX.to_string(),"filed":u64::MAX.to_string(),
        "child_revision":revision(u32::MAX,u64::MAX),"birth_operation":CommitToken::from_bytes([254;16]),"title_revision":u64::MAX,
        "source_parent":{"filed":(u64::MAX-1).to_string(),"revision":revision(u32::MAX-1,u64::MAX-1)},
        "parent_charter":(u32::MAX-1).to_string(),"parent_line":(u64::MAX-1).to_string(),
        "selected_parent":{"licence":(u64::MAX-1).to_string(),"term":u64::MAX.to_string(),"filed":(u64::MAX-2).to_string(),"revision":revision(u32::MAX-1,u64::MAX-2),
            "window":{"format":1,"eligibility_start":i64::MAX-100,"issued_at":i64::MAX-100,"term_end":i64::MAX-1,"grace_end":i64::MAX},"form":{"kind":"open_resale"},"operation":CommitToken::from_bytes([253;16]),"terminal":CommitToken::from_bytes([252;16])},
        "ordinal":1,"previous":null,"observed_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX-1}
    }))?)
}
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
                namespace: CanonicalId::new("owner_qualification_test")?,
            },
            1,
            IdempotencyKey::Request(uid),
        ),
    })?)
}
#[test]
fn qualification_request_is_exact_numeric_self_selection_without_authority_fields() -> TestResult {
    let request =
        provider::Request::parse(&invocation(json!({"owner_licence":3,"parent_licence":2}))?)?;
    check_eq!(request.owner()?, LicenceId::new(3)?);
    check_eq!(request.parent()?, LicenceId::new(2)?);
    for value in [json!("2"), json!(0), json!(-1), json!(2.5), json!(null)] {
        check!(
            provider::Request::parse(&invocation(
                json!({"owner_licence":3,"parent_licence":value})
            )?)
            .is_err()
        );
    }
    check!(
        provider::Request::parse(&invocation(json!({"owner_licence":3,"parent_licence":3}))?)
            .is_err()
    );
    for key in [
        "actor",
        "form",
        "term_end",
        "source",
        "title_revision",
        "validated",
    ] {
        let mut value = json!({"owner_licence":3,"parent_licence":2});
        value
            .as_object_mut()
            .ok_or("params")?
            .insert(key.into(), json!(true));
        check!(provider::Request::parse(&invocation(value)?).is_err());
    }
    Ok(())
}
#[test]
fn qualification_records_reject_missing_links_unknown_authority_and_crossed_identity() -> TestResult
{
    let original = row()?;
    original.validate()?;
    for key in ["permission", "source_access", "child_window", "validated"] {
        let mut value = serde_json::to_value(&original)?;
        value
            .as_object_mut()
            .ok_or("row")?
            .insert(key.into(), json!(true));
        check!(serde_json::from_value::<QualificationOperation>(value).is_err());
    }
    let mut value = serde_json::to_value(&original)?;
    value.as_object_mut().ok_or("row")?.remove("previous");
    check!(serde_json::from_value::<QualificationOperation>(value).is_err());
    for (key, value) in [
        ("format", json!(2)),
        ("owner", json!(1)),
        ("owner_term", json!("2")),
        ("ordinal", json!(0)),
        ("title_revision", json!(0)),
    ] {
        let mut wire = serde_json::to_value(&original)?;
        *wire.get_mut(key).ok_or("field")? = value;
        check!(serde_json::from_value::<QualificationOperation>(wire).is_err());
    }
    let mut wire = serde_json::to_value(&original)?;
    wire.get_mut("selected_parent")
        .and_then(Value::as_object_mut)
        .ok_or("parent")?
        .insert("authority".into(), json!(true));
    check!(serde_json::from_value::<QualificationOperation>(wire).is_err());
    let mut bad = original.clone();
    bad.selected_parent.form = crate::commerce::paid::PaidForm::Sealed {};
    check!(bad.validate().is_err());
    let mut bad = original.clone();
    bad.selected_parent.terminal = bad.selected_parent.operation;
    check!(bad.validate().is_err());
    let mut head = serde_json::to_value(original.head())?;
    *head.get_mut("ordinal").ok_or("ordinal")? = json!(0);
    check!(serde_json::from_value::<QualificationHead>(head).is_err());
    Ok(())
}
#[test]
fn qualification_chain_is_ordered_by_cause_and_preserves_immutable_child_and_clock() -> TestResult {
    let mut old = row()?;
    old.observed_ic.clock = mush_core::ClockRef::WorldIc { generation: 1 };
    old.follows(None)?;
    let mut next = old.clone();
    next.operation = CommitToken::from_bytes([1; 16]);
    next.previous = Some(old.operation);
    next.ordinal = 2;
    next.selected_parent.form = crate::commerce::paid::PaidForm::Open {};
    next.follows(Some(&old))?;
    check!(next.follows(None).is_err());
    for mode in 0..7 {
        let mut bad = next.clone();
        match mode {
            0 => bad.previous = Some(CommitToken::from_bytes([7; 16])),
            1 => bad.ordinal = 3,
            2 => bad.owner_licence = LicenceId::new(3)?,
            3 => bad.child_revision = old.source_parent.revision,
            4 => bad.birth_operation = CommitToken::from_bytes([6; 16]),
            5 => bad.source_parent.filed = crate::commerce::FiledRevisionId::new(3)?,
            _ => bad.observed_ic.at -= 1,
        };
        check!(bad.follows(Some(&old)).is_err());
    }
    // Distinct clock generations are explicit; no numeric cross-clock order is invented.
    next.observed_ic.clock = mush_core::ClockRef::WorldIc { generation: 2 };
    next.follows(Some(&old))?;
    let mut crossed = next.head();
    crossed.operation = old.operation;
    check!(crossed.matches(&next).is_err());
    Ok(())
}
#[test]
fn qualification_cause_retains_selected_parent_without_rewriting_child_ancestry() -> TestResult {
    let old = row()?;
    let mut replacement = old.clone();
    replacement.selected_parent.form = crate::commerce::paid::PaidForm::Open {};
    check_ne!(replacement.digest()?, old.digest()?);
    check_eq!(replacement.child_revision, old.child_revision);
    check_eq!(replacement.source_parent, old.source_parent);
    let root = replacement.root()?;
    check_eq!(root.identity, replacement.selected_parent.revision);
    check_ne!(root.identity, replacement.child_revision);
    check_eq!(RootAnchor::parse(&root.anchor.record())?, root.anchor);
    check!(RootAnchor::parse(&(root.anchor.record() + "/extra")).is_err());
    check!(RootAnchor::parse(&root.anchor.record().to_uppercase()).is_err());
    Ok(())
}
type Handles = (
    StateFamilyHandle<String, QualificationOperation>,
    StateFamilyHandle<String, QualificationHead>,
);
struct Rows(Arc<Mutex<Option<Handles>>>);
impl GameModule for Rows {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("handle lock"))? = Some((
            families.owner_qualification_operations,
            families.owner_qualification_heads,
        ));
        Ok(())
    }
}
#[test]
fn qualification_complete_max_width_envelopes_fit_real_registered_family_limits() -> TestResult {
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (operations, heads) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let mut operation = row()?;
    operation.ordinal = u64::MAX;
    operation.previous = Some(CommitToken::from_bytes([251; 16]));
    operation.validate()?;
    let head = operation.head();
    let document = |handle_owner: &OwnerId,
                    handle_id: &mush_core::StateFamilyId,
                    schema: &CanonicalId,
                    key: String,
                    value: Value| json!({"family":{"owner":handle_owner,"id":handle_id},"key":key,"schema":{"id":schema,"version":1},"value":value});
    let op_doc = document(
        operations.owner(),
        operations.descriptor().id(),
        operations.descriptor().value_schema(),
        operation.key(),
        serde_json::to_value(&operation)?,
    );
    let head_doc = document(
        heads.owner(),
        heads.descriptor().id(),
        heads.descriptor().value_schema(),
        head.key(),
        serde_json::to_value(&head)?,
    );
    check!(
        mush_core::canonical_json_bytes_with_limit(&op_doc, QualificationOperation::MAX_BYTES)?
            .len()
            < QualificationOperation::MAX_BYTES
    );
    check!(
        mush_core::canonical_json_bytes_with_limit(&head_doc, QualificationHead::MAX_BYTES)?.len()
            < QualificationHead::MAX_BYTES
    );
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &operation.key(), operation.clone())?;
    txn.insert(&heads, &head.key(), head)?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    let mut txn = engine.action_transaction(&operations)?;
    check!(
        txn.insert(&operations, &"not-a-commit".into(), operation)
            .is_err()
    );
    Ok(())
}
#[test]
fn qualification_requires_explicit_active_dependent_composition() -> TestResult {
    use crate::commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::RatingCompatibility,
    };
    let make = |mode| {
        CommerceConfig::new(
            mode,
            InitialFilingTerms::new(90 * 86400, 3 * 86400)
                .map_err(|e| ModError::msg(e.to_string()))?,
            manifest(),
            RatingCompatibility::combat_v1(manifest().digest()),
        )
    };
    check!(
        make(CommerceMode::Initialize)?
            .with_owner_parent_qualification()
            .is_err()
    );
    check!(
        make(CommerceMode::Active)?
            .with_owner_parent_qualification()
            .is_err()
    );
    let [a, _] = protocol::definitions(&manifest())?;
    check_eq!(a.version(), 1);
    Ok(())
}

// Keep the real installation fixture in its own frame, separate from the
// versioned codec probes and their corruption cases.
fn install_captured_operation(operation: QualificationOperation) -> TestResult {
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
    let document = json!({"family":{"owner":operations.owner(),"id":operations.descriptor().id()},"key":operation.key(),"schema":{"id":operations.descriptor().value_schema(),"version":1},"value":operation});
    mush_core::canonical_json_bytes_with_limit(&document, QualificationOperation::MAX_BYTES)?;
    let mut txn = engine.action_transaction(&operations)?;
    txn.insert(&operations, &operation.key(), operation)?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    Ok(())
}

/// These are strict data/size probes only; historical transfer authority comes
/// from the real native receipt and complete paired owner audit.
#[test]
fn charter_ownership_capture_is_versioned_and_legacy_bytes_remain_exact() -> TestResult {
    let old = row()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: QualificationOperation = serde_json::from_slice(&bytes)?;
    check_eq!(serde_json::to_vec(&restored)?, bytes);
    let legacy = serde_json::to_value(&old)?;
    check!(legacy.get("charter_ownership").is_none());
    let proof = serde_json::json!({"generation":65537,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])});
    let mut value = legacy.clone();
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("charter_ownership".into(), proof.clone());
    check!(
        serde_json::from_value::<QualificationOperation>(value.clone()).is_err(),
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
    let new: QualificationOperation = serde_json::from_value(value.clone())?;
    new.validate()?;
    check_eq!(serde_json::to_value(&new)?, value);
    install_captured_operation(new)?;
    for corrupt in [
        serde_json::Value::Null,
        serde_json::json!({"generation":1,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])}),
        serde_json::json!({"generation":65538,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])}),
    ] {
        let mut bad = value.clone();
        bad.as_object_mut()
            .ok_or("operation")?
            .insert("charter_ownership".into(), corrupt);
        check!(serde_json::from_value::<QualificationOperation>(bad).is_err());
    }
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .ok_or("operation")?
        .remove("charter_ownership");
    check!(serde_json::from_value::<QualificationOperation>(missing).is_err());
    let mut extra = value.clone();
    extra
        .get_mut("charter_ownership")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("ownership")?
        .insert("validated".into(), serde_json::json!(true));
    check!(serde_json::from_value::<QualificationOperation>(extra).is_err());
    Ok(())
}

#[test]
fn parent_holder_capture_is_closed_and_legacy_qualification_bytes_stay_exact() -> TestResult {
    let old = row()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: QualificationOperation = serde_json::from_slice(&bytes)?;
    check_eq!(serde_json::to_vec(&restored)?, bytes);
    let mut wire = serde_json::to_value(&old)?;
    check!(wire.pointer("/selected_parent/holder_basis").is_none());
    let proof = json!({"format":1,"acceptance":CommitToken::from_bytes([249;16]),
        "ownership":{"generation":65537,"operation":CommitToken::from_bytes([248;16]),"digest":DefinitionDigest::from_bytes([255;32])}});
    wire.get_mut("selected_parent")
        .and_then(Value::as_object_mut)
        .ok_or("parent")?
        .insert("holder_basis".into(), proof.clone());
    let captured: QualificationOperation = serde_json::from_value(wire.clone())?;
    check_eq!(serde_json::to_value(&captured)?, wire);
    check_ne!(
        old.digest()?,
        captured.digest()?,
        "the actual cause covers ownership attribution"
    );
    install_captured_operation(captured)?;
    let mut corruptions = vec![Value::Null];
    for (path, value) in [
        ("/format", json!(2)),
        ("/ownership/generation", json!(1)),
        ("/ownership/generation", json!(65538)),
    ] {
        let mut bad = proof.clone();
        *bad.pointer_mut(path).ok_or("proof field")? = value;
        corruptions.push(bad);
    }
    for key in ["format", "acceptance", "ownership"] {
        let mut bad = proof.clone();
        bad.as_object_mut().ok_or("proof")?.remove(key);
        corruptions.push(bad);
    }
    let mut extra = proof;
    extra
        .as_object_mut()
        .ok_or("proof")?
        .insert("validated".into(), json!(true));
    corruptions.push(extra);
    for proof in corruptions {
        let mut bad = wire.clone();
        *bad.pointer_mut("/selected_parent/holder_basis")
            .ok_or("basis")? = proof;
        check!(serde_json::from_value::<QualificationOperation>(bad).is_err());
    }
    Ok(())
}
