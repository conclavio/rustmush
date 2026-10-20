//! Structural rows and real registration/capsule boundaries; no certificate is minted here.
use super::{
    records::{UpdateOperation, UpdatePosition},
    *,
};
use crate::commerce::{
    families::CommerceFamilies,
    records::{LineRecord, Record},
    roots::RootAnchor,
    *,
};
use mush_core::mechanics::{DefinitionRegistryBuilder, OwnerId};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, Engine, GameModule, ModError, Registrar,
    Registry, StateFamilyHandle, World,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn action() -> TestResult<DefinitionRef> {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    let manifest = reference("tacsim", manifest_descriptor());
    let [descriptor, _] = registration::definitions(&manifest)?;
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, descriptor)?)
}
fn operation() -> TestResult<UpdateOperation> {
    let digest = DefinitionDigest::from_bytes([255; 32]);
    let commit = CommitToken::from_bytes([255; 16]);
    Ok(serde_json::from_value(
        json!({"format":1,"commit":commit,"action":action()?,
        "actor":u32::MAX,"initial":CommitToken::from_bytes([254;16]),"charter":u32::MAX.to_string(),
        "line":u64::MAX.to_string(),"licence":u64::MAX.to_string(),"term":"1",
        "previous":(u64::MAX-1).to_string(),"filed":u64::MAX.to_string(),"sequence":crate::MAX_RECORDS as u64+1,
        "revision":{"format":1,"revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":digest},"source":digest},
        "rating":{"schema":1,"job":CommitToken::from_bytes([253;16]),"id":digest,"receipt_digest":digest},
        "schema":digest,"duplicate":digest,"at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX},
        "economic":{"commit":commit,"transaction":u64::MAX,"journal_seq":u64::MAX,"posted_at_utc":i64::MIN,
            "plan_hash":[255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255,255],
            "actor":u32::MAX,"charter":u32::MAX,"title_revision":u64::MAX,"payer":u64::MAX,"sink":u64::MAX-1,
            "fee":i64::MAX,"instrument":"cr","issuer":"roll.registry"}}),
    )?)
}
#[test]
fn compatible_rows_reject_crossed_identity_and_unknown_authority() -> TestResult {
    let row = operation()?;
    for field in ["actor", "charter", "commit"] {
        let mut wire = serde_json::to_value(&row)?;
        let value = match field {
            "actor" | "charter" => json!(1),
            _ => json!(CommitToken::from_bytes([2; 16])),
        };
        *wire
            .get_mut("economic")
            .and_then(|v| v.get_mut(field))
            .ok_or("economic field")? = value;
        assert!(serde_json::from_value::<UpdateOperation>(wire).is_err());
    }
    for field in ["permission", "source", "parent", "validated"] {
        let mut wire = serde_json::to_value(&row)?;
        wire.as_object_mut()
            .ok_or("wire")?
            .insert(field.into(), json!(true));
        assert!(serde_json::from_value::<UpdateOperation>(wire).is_err());
    }
    for seq in [0, 1, crate::MAX_RECORDS as u64 + 2] {
        let mut wire = serde_json::to_value(&row)?;
        *wire.get_mut("sequence").ok_or("sequence")? = json!(seq);
        assert!(serde_json::from_value::<UpdateOperation>(wire).is_err());
    }
    // The additive native derived protocol cannot be relabeled as an old
    // original update merely by retaining the old action and outer format.
    let mut derived = row.clone();
    derived.economic.kind = Some(mush_economy::registry_filing::UpdateKind::DependentV1);
    assert!(derived.validate().is_err());
    assert!(serde_json::from_value::<UpdateOperation>(serde_json::to_value(derived)?).is_err());
    let mut same = row.clone();
    same.previous = same.filed;
    assert!(same.validate().is_err());
    let mut same = row.clone();
    same.initial = same.commit;
    assert!(same.validate().is_err());
    let root = RootAnchor::UpdateOperation(row.commit);
    assert_eq!(RootAnchor::parse(&root.record())?, root);
    assert_ne!(root.record(), RootAnchor::Operation(row.commit).record());
    Ok(())
}
#[test]
fn sequence_index_and_old_line_format_remain_exact() -> TestResult {
    let row = operation()?;
    let position = UpdatePosition {
        format: 1,
        line: row.line,
        sequence: row.sequence,
        filed: row.filed,
        operation: row.commit,
    };
    position.check(&row)?;
    assert_eq!(
        position.key(),
        UpdatePosition::key_for(row.line, row.sequence)
    );
    for mode in 0..3 {
        let mut crossed = position.clone();
        match mode {
            0 => crossed.sequence = 2,
            1 => crossed.filed = row.previous,
            _ => crossed.operation = row.initial,
        };
        assert!(crossed.check(&row).is_err());
    }
    let mut line = LineRecord {
        format: 1,
        id: row.line,
        charter: row.charter,
        schema_digest: row.schema,
        head: row.previous,
        next_sequence: 2,
        operation: row.initial,
    };
    line.validate()?;
    let old = serde_json::to_vec(&line)?;
    line.next_sequence = 3;
    assert!(line.validate().is_err());
    line.format = 2;
    line.validate()?;
    line.next_sequence = 2;
    assert!(line.validate().is_err());
    let restored: LineRecord = serde_json::from_slice(&old)?;
    restored.validate()?;
    assert_eq!(serde_json::to_vec(&restored)?, old);
    Ok(())
}
type Handle = StateFamilyHandle<String, UpdateOperation>;
struct Rows(Arc<Mutex<Option<Handle>>>);
impl GameModule for Rows {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        *self.0.lock().map_err(|_| ModError::msg("handle lock"))? =
            Some(CommerceFamilies::register(reg)?.line_updates);
        Ok(())
    }
}
#[test]
fn registered_update_family_bounds_complete_numeric_row_and_refuses_oversize() -> TestResult {
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
    let row = operation()?;
    let document = json!({"family":{"owner":handle.owner(),"id":handle.descriptor().id()},"key":row.key(),"schema":{"id":handle.descriptor().value_schema(),"version":1},"value":row});
    let bytes = mush_core::canonical_json_bytes_with_limit(&document, UpdateOperation::MAX_BYTES)?;
    assert!(bytes.len() < UpdateOperation::MAX_BYTES);
    let mut later = row.clone();
    later.format = 2;
    later.term = TermGeneration::new(u64::MAX)?;
    later.validate()?;
    let later_document = json!({"family":{"owner":handle.owner(),"id":handle.descriptor().id()},"key":later.key(),"schema":{"id":handle.descriptor().value_schema(),"version":1},"value":later});
    mush_core::canonical_json_bytes_with_limit(&later_document, UpdateOperation::MAX_BYTES)?;
    let mut txn = engine.action_transaction(&handle)?;
    txn.insert(&handle, &row.key(), row.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    let mut huge = row.clone();
    huge.commit = CommitToken::from_bytes([1; 16]);
    huge.economic.commit = huge.commit;
    huge.economic.instrument =
        mush_economy::InstrumentId::new("x".repeat(UpdateOperation::MAX_BYTES))
            .ok_or("instrument")?;
    let mut txn = engine.action_transaction(&handle)?;
    assert!(txn.insert(&handle, &huge.key(), huge).is_err());
    Ok(())
}
#[test]
fn update_registration_requires_explicit_active_configuration_and_actual_fee_owner() -> TestResult {
    use crate::commerce::bootstrap_tests::{Manifest, manifest_descriptor, reference};
    use crate::commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::RatingCompatibility,
    };
    let manifest = reference("tacsim", manifest_descriptor());
    let make = |mode| -> TestResult<CommerceConfig> {
        Ok(CommerceConfig::new(
            mode,
            InitialFilingTerms::new(90 * 86400, 3 * 86400)?,
            manifest.clone(),
            RatingCompatibility::combat_v1(manifest.digest()),
        )?)
    };
    assert!(
        make(CommerceMode::Initialize)?
            .with_compatible_updates()
            .is_err()
    );
    for (enabled, fee_owner) in [(false, false), (true, false), (true, true)] {
        let config = make(CommerceMode::Active)?;
        let config = if enabled {
            config.with_compatible_updates()?
        } else {
            config
        };
        let expected = action()?;
        let issuer = mush_org::SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
        let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
            issuer.clone(),
            mush_economy::AccountId(1),
            mush_economy::Amount::from_minor(100),
            mush_economy::CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )?;
        let economy = mush_economy::EconomyModule::new()
            .with_title_issuers(vec![issuer])
            .with_initial_filing(policy, config.initial_filing_action()?);
        let economy = if fee_owner {
            economy.with_compatible_mind_updates(expected.clone())
        } else {
            economy
        };
        let result = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(Manifest),
                Box::new(crate::RollModule::with_writers(["tacsim"]).with_commerce(config)),
                Box::new(economy),
            ],
        );
        if enabled && !fee_owner {
            assert!(result.is_err());
            continue;
        }
        let engine = result?;
        assert_eq!(
            engine
                .registry
                .definitions()
                .ok_or("definitions")?
                .iter()
                .filter(|e| e.reference() == &expected)
                .count(),
            usize::from(enabled)
        );
    }
    Ok(())
}

#[test]
fn historical_update_reference_uses_retained_compatibility_not_current_manifest() -> TestResult {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    use crate::commerce::rating::RatingCompatibility;
    let old = reference("tacsim", manifest_descriptor());
    let changed = DefinitionRef::new(
        old.key().clone(),
        old.version(),
        DefinitionDigest::from_bytes([11; 32]),
    )?;
    let old_action =
        registration::historical_action(&RatingCompatibility::combat_v1(old.digest()))?;
    let changed_action =
        registration::historical_action(&RatingCompatibility::combat_v1(changed.digest()))?;
    assert_eq!(old_action, action()?);
    assert_eq!(old_action.locator(), changed_action.locator());
    assert_ne!(old_action, changed_action);
    let [descriptor, _] = registration::definitions(&changed)?;
    assert_eq!(
        changed_action,
        DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, descriptor)?
    );
    Ok(())
}

#[test]
fn update_generation_formats_keep_old_bytes_and_require_exact_historical_owner_term() -> TestResult
{
    use crate::commerce::owner_attestation::records::AttestationEvidence;
    use crate::commerce::records::{OwnerTerm, RevisionPolicy};
    let mut row = operation()?;
    row.at_ic.at = 1000;
    let old_bytes = serde_json::to_vec(&row)?;
    let restored: UpdateOperation = serde_json::from_slice(&old_bytes)?;
    assert_eq!(serde_json::to_vec(&restored)?, old_bytes);
    let mut term = OwnerTerm {
        format: 1,
        licence: row.licence,
        generation: TermGeneration::new(1)?,
        window: TermWindow::new(
            WorldIcSeconds(900),
            WorldIcSeconds(900),
            WorldIcSeconds(1100),
            WorldIcSeconds(1200),
        )?,
        revision: RevisionPolicy::Follow { line: row.line },
        operation: row.initial,
        parent: None,
        attestation: None,
    };
    row.check_owner_term(&term)?;
    for (format, generation) in [(1, 2), (2, 1), (3, 2)] {
        let mut crossed = row.clone();
        crossed.format = format;
        crossed.term = TermGeneration::new(generation)?;
        assert!(crossed.validate().is_err());
        assert!(serde_json::from_value::<UpdateOperation>(serde_json::to_value(crossed)?).is_err());
    }
    row.format = 2;
    row.term = TermGeneration::new(2)?;
    assert!(row.check_owner_term(&term).is_err());
    term.format = 3;
    term.generation = row.term;
    term.operation = CommitToken::from_bytes([252; 16]);
    term.attestation = Some(AttestationEvidence {
        term_seconds: 200,
        grace_seconds: 100,
        parent: None,
    });
    row.check_owner_term(&term)?;
    let new_bytes = serde_json::to_vec(&row)?;
    assert_eq!(
        serde_json::to_vec(&serde_json::from_slice::<UpdateOperation>(&new_bytes)?)?,
        new_bytes
    );
    for at in [899, 1200] {
        let mut crossed = row.clone();
        crossed.at_ic.at = at;
        assert!(crossed.check_owner_term(&term).is_err());
    }
    for at in [900, 1100, 1199] {
        let mut current = row.clone();
        current.at_ic.at = at;
        current.check_owner_term(&term)?;
    }
    let mut crossed = term.clone();
    crossed.operation = row.initial;
    assert!(row.check_owner_term(&crossed).is_err());
    let mut crossed = term.clone();
    crossed.generation = TermGeneration::new(3)?;
    assert!(row.check_owner_term(&crossed).is_err());
    let mut crossed = term.clone();
    crossed.revision = RevisionPolicy::Follow {
        line: LineId::new(1)?,
    };
    assert!(row.check_owner_term(&crossed).is_err());
    Ok(())
}

/// These are strict data/size probes only; historical transfer authority comes
/// from the real native receipt and complete paired owner audit.
#[test]
fn charter_ownership_capture_is_versioned_and_legacy_bytes_remain_exact() -> TestResult {
    let old = operation()?;
    let bytes = serde_json::to_vec(&old)?;
    let restored: UpdateOperation = serde_json::from_slice(&bytes)?;
    assert_eq!(serde_json::to_vec(&restored)?, bytes);
    let legacy = serde_json::to_value(&old)?;
    assert!(legacy.get("charter_ownership").is_none());
    let proof = serde_json::json!({"generation":65537,"operation":mush_core::CommitToken::from_bytes([249;16]),"digest":mush_core::DefinitionDigest::from_bytes([255;32])});
    let mut value = legacy.clone();
    let fields = value.as_object_mut().ok_or("operation")?;
    fields.insert("charter_ownership".into(), proof.clone());
    assert!(
        serde_json::from_value::<UpdateOperation>(value.clone()).is_err(),
        "old format cannot carry new authority attribution"
    );
    value
        .as_object_mut()
        .ok_or("operation")?
        .insert("format".into(), serde_json::json!(3));

    let new: UpdateOperation = serde_json::from_value(value.clone())?;
    new.validate()?;
    assert_eq!(serde_json::to_value(&new)?, value);
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let handle = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let document = json!({"family":{"owner":handle.owner(),"id":handle.descriptor().id()},"key":new.key(),"schema":{"id":handle.descriptor().value_schema(),"version":1},"value":new});
    mush_core::canonical_json_bytes_with_limit(&document, UpdateOperation::MAX_BYTES)?;
    let mut txn = engine.action_transaction(&handle)?;
    txn.insert(&handle, &new.key(), new.clone())?;
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
        assert!(serde_json::from_value::<UpdateOperation>(bad).is_err());
    }
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .ok_or("operation")?
        .remove("charter_ownership");
    assert!(serde_json::from_value::<UpdateOperation>(missing).is_err());
    let mut extra = value.clone();
    extra
        .get_mut("charter_ownership")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("ownership")?
        .insert("validated".into(), serde_json::json!(true));
    assert!(serde_json::from_value::<UpdateOperation>(extra).is_err());
    Ok(())
}

#[path = "tests/derived.rs"]
mod derived;
