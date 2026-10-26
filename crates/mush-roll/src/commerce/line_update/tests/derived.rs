//! Strict retained data probes; only the genuine child supplies parent authority.
use super::*;
use serde_json::Value;
fn derived_action() -> TestResult<DefinitionRef> {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    let [descriptor, _] =
        registration::derived_definitions(&reference("tacsim", manifest_descriptor()))?;
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, descriptor)?)
}
fn derived() -> TestResult<UpdateOperation> {
    let mut value = serde_json::to_value(operation()?)?;
    let digest = DefinitionDigest::from_bytes([255; 32]);
    let revision = json!({"format":1,"revision":{"key":{"character":u32::MAX-1,"mind":u64::MAX,"number":u64::MAX},"digest":digest},"source":digest});
    let ownership =
        json!({"generation":65537,"operation":CommitToken::from_bytes([243;16]),"digest":digest});
    let holder =
        json!({"format":1,"acceptance":CommitToken::from_bytes([244;16]),"ownership":ownership});
    let selected = json!({"licence":(u64::MAX-1).to_string(),"term":u64::MAX.to_string(),"filed":(u64::MAX-2).to_string(),"revision":revision,
        "window":{"format":1,"eligibility_start":i64::MAX-100,"issued_at":i64::MAX-2,"term_end":i64::MAX-1,"grace_end":i64::MAX},
        "form":{"kind":"open_resale"},"operation":CommitToken::from_bytes([247;16]),"terminal":CommitToken::from_bytes([246;16]),"holder_basis":holder});
    let parent = json!({"format":1,"source":{"filed":(u64::MAX-1).to_string(),"revision":revision},"charter":(u32::MAX-1).to_string(),"line":(u64::MAX-1).to_string(),"qualification":CommitToken::from_bytes([245;16]),"selected":selected});
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("format".into(), json!(4));
    fields.insert("action".into(), json!(derived_action()?));
    fields.insert("parent".into(), parent);
    fields.insert(
        "charter_ownership".into(),
        json!({"generation":65537,"operation":CommitToken::from_bytes([242;16]),"digest":digest}),
    );
    value
        .get_mut("economic")
        .and_then(Value::as_object_mut)
        .ok_or("economic")?
        .insert("kind".into(), json!("dependent_v1"));
    Ok(serde_json::from_value(value)?)
}
#[test]
fn dependent_update_requires_versioned_native_and_parent_evidence() -> TestResult {
    let original = operation()?;
    let old = serde_json::to_vec(&original)?;
    check!(serde_json::to_value(&original)?.get("parent").is_none());
    check_eq!(
        serde_json::to_vec(&serde_json::from_slice::<UpdateOperation>(&old)?)?,
        old
    );
    let row = derived()?;
    for mode in 0..9 {
        let mut value = serde_json::to_value(&row)?;
        match mode {
            0 => {
                value.as_object_mut().ok_or("operation")?.remove("parent");
            }
            1 => {
                value
                    .as_object_mut()
                    .ok_or("operation")?
                    .insert("parent".into(), Value::Null);
            }
            2 => {
                *value.get_mut("format").ok_or("format")? = json!(3);
            }
            3 => {
                *value.get_mut("action").ok_or("action")? = json!(action()?);
            }
            4 => {
                value
                    .get_mut("economic")
                    .and_then(Value::as_object_mut)
                    .ok_or("economic")?
                    .remove("kind");
            }
            5 => {
                value
                    .pointer_mut("/parent/selected")
                    .and_then(Value::as_object_mut)
                    .ok_or("selected")?
                    .insert("authority".into(), json!(true));
            }
            6 => {
                *value.pointer_mut("/parent/qualification").ok_or("cause")? = Value::Null;
            }
            7 => {
                *value
                    .pointer_mut("/parent/selected/holder_basis")
                    .ok_or("holder")? = Value::Null;
            }
            _ => {
                *value.pointer_mut("/parent/selected/form").ok_or("form")? =
                    json!({"kind":"sealed"});
            }
        }
        check!(serde_json::from_value::<UpdateOperation>(value).is_err());
    }
    let mut crossed = original.clone();
    crossed.parent = row.parent.clone();
    check!(crossed.validate().is_err());
    Ok(())
}
#[test]
fn derived_update_complete_envelope_keeps_existing_family_limit() -> TestResult {
    let row = derived()?;
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let handle = shared
        .lock()
        .map_err(|_| "handle lock")?
        .clone()
        .ok_or("handle")?;
    let document = json!({"family":{"owner":handle.owner(),"id":handle.descriptor().id()},"key":row.key(),"schema":{"id":handle.descriptor().value_schema(),"version":1},"value":row});
    let bytes = mush_core::canonical_json_bytes_with_limit(&document, UpdateOperation::MAX_BYTES)?;
    check!(bytes.len() < UpdateOperation::MAX_BYTES);
    let mut txn = engine.action_transaction(&handle)?;
    txn.insert(&handle, &row.key(), row.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    Ok(())
}
#[test]
fn derived_action_history_reconstructs_full_retained_manifest() -> TestResult {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    let manifest = reference("tacsim", manifest_descriptor());
    let old = registration::historical_derived_action(&rating::RatingCompatibility::combat_v1(
        manifest.digest(),
    ))?;
    check_eq!(old, derived_action()?);
    let changed = registration::historical_derived_action(
        &rating::RatingCompatibility::combat_v1(DefinitionDigest::from_bytes([1; 32])),
    )?;
    check_eq!(old.locator(), changed.locator());
    check_ne!(old, changed);
    check_ne!(old.locator(), action()?.locator());
    Ok(())
}
