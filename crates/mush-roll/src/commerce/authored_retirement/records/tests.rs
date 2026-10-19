//! Structural/capacity tests only. No fixture bytes confer paid rights or fork permission.
use super::*;
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn maximum() -> Result<RetirementOperation, Box<dyn std::error::Error>> {
    let actor = ObjId(u32::MAX);
    let mind = u64::MAX - 1;
    let reference = |number| RevisionRef {
        key: crate::RevisionKey {
            character: actor,
            mind,
            number,
        },
        digest: DefinitionDigest::from_bytes([255; 32]),
    };
    let total = crate::KEEP_UNNAMED + crate::MAX_NAMED_VERSIONS;
    let first = u64::MAX - u64::try_from(total)? + 1;
    let mut unnamed = Vec::new();
    let mut named = Vec::new();
    for i in 0..crate::KEEP_UNNAMED {
        unnamed.push(reference(first + u64::try_from(i)?));
    }
    for i in 0..crate::MAX_NAMED_VERSIONS {
        named.push(NamedRetention {
            name: format!("{i:03}{}", "\\".repeat(61)),
            revision: reference(first + u64::try_from(crate::KEEP_UNNAMED + i)?),
        });
    }
    Ok(RetirementOperation {
        format: 1,
        actor,
        mind,
        selection: DefinitionDigest::from_bytes([255; 32]),
        operation: CommitToken::from_bytes([255; 16]),
        base: EditBase {
            edit: u64::MAX,
            revision: Some(u64::MAX),
        },
        action: super::super::action_reference()?,
        name: "\\".repeat(64),
        parent: serde_json::from_value(
            json!({"format":1,"revision":reference(u64::MAX),"source":DefinitionDigest::from_bytes([255;32])}),
        )?,
        unnamed,
        named,
    })
}
fn envelope<R: Record>(row: &R) -> Result<serde_json::Value, serde_json::Error> {
    // Exact enclosing fields used by ActionTxn::put, not only row.value.
    Ok(
        json!({"family":{"owner":"roll","id":R::FAMILY},"key":row.key(),
        "schema":{"id":format!("{}.value.v1",R::FAMILY),"version":1},"value":serde_json::to_value(row)?}),
    )
}
#[test]
fn retirement_maximum_inventory_complete_envelopes_fit_declared_limits() -> TestResult {
    let op = maximum()?;
    op.validate()?;
    let bytes = mush_core::canonical_json_bytes_with_limit(
        &envelope(&op)?,
        RetirementOperation::MAX_BYTES,
    )?;
    assert!(
        bytes.len() > 40_000 && bytes.len() < 65_536,
        "complete maximum-width operation has bounded headroom"
    );
    let tombstone = op.tombstone()?;
    let bytes = mush_core::canonical_json_bytes_with_limit(
        &envelope(&tombstone)?,
        RetiredDraftParent::MAX_BYTES,
    )?;
    assert!(bytes.len() < 4096);
    let decoded: RetirementOperation = serde_json::from_slice(&serde_json::to_vec(&op)?)?;
    decoded.validate()?;
    assert!(decoded == op);
    Ok(())
}
#[test]
fn retirement_decode_bounds_inventory_before_constructing_excess_entries() -> TestResult {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DECODED: AtomicUsize = AtomicUsize::new(0);
    struct Entry;
    impl<'de> Deserialize<'de> for Entry {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let _ = u64::deserialize(d)?;
            DECODED.fetch_add(1, Ordering::SeqCst);
            Ok(Self)
        }
    }
    let mut decoder = serde_json::Deserializer::from_str("[1,2,3,4,5]");
    assert!(inventory::<_, Entry, 2>(&mut decoder).is_err());
    assert_eq!(
        DECODED.load(Ordering::SeqCst),
        2,
        "third retained item was not allocated/decoded"
    );
    let op = maximum()?;
    let mut value = serde_json::to_value(&op)?;
    let rows = value
        .get_mut("unnamed")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("unnamed")?;
    rows.push(serde_json::to_value(op.unnamed.first().ok_or("first")?)?);
    assert!(serde_json::from_value::<RetirementOperation>(value).is_err());
    let mut value = serde_json::to_value(&op)?;
    let rows = value
        .get_mut("named")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("named")?;
    rows.push(serde_json::to_value(op.named.first().ok_or("first")?)?);
    assert!(serde_json::from_value::<RetirementOperation>(value).is_err());
    Ok(())
}
#[test]
fn retirement_inventory_action_and_pair_are_exact_metadata() -> TestResult {
    let op = maximum()?;
    for mode in 0..8 {
        let mut altered = op.clone();
        match mode {
            0 => altered.format = 2,
            1 => {
                altered
                    .named
                    .first_mut()
                    .ok_or("named")?
                    .revision
                    .key
                    .character = ObjId(1)
            }
            2 => {
                altered.named.first_mut().ok_or("named")?.revision =
                    *altered.unnamed.first().ok_or("unnamed")?
            }
            3 => altered.named.reverse(),
            4 => {
                altered.named.pop();
            }
            5 => altered.base.edit = 0,
            6 => altered.name = "Wrong Case".into(),
            7 => {
                altered.action = super::super::retention_reference()?;
            }
            _ => return Err("mode".into()),
        }
        assert!(
            altered.validate().is_err(),
            "invalid bounded metadata case {mode}"
        );
    }
    let mut changed = op.clone();
    changed.operation = CommitToken::from_bytes([254; 16]);
    assert!(changed.digest()? != op.digest()? && changed.tombstone()? != op.tombstone()?);
    let mut value = serde_json::to_value(&op)?;
    value
        .as_object_mut()
        .ok_or("object")?
        .insert("source".into(), json!("forbidden"));
    assert!(serde_json::from_value::<RetirementOperation>(value).is_err());
    let raw = serde_json::to_string(&op)?;
    let duplicate = raw.replacen("{", "{\"format\":1,", 1);
    assert!(serde_json::from_str::<RetirementOperation>(&duplicate).is_err());
    Ok(())
}
#[test]
fn retirement_action_is_stable_without_manifest_or_paid_qualification() -> TestResult {
    let [action, _, reader] = super::super::definitions()?;
    assert_eq!(action.references(), [super::super::retention_reference()?]);
    assert_eq!(reader.references(), action.references());
    let op = maximum()?;
    let anchor = crate::commerce::roots::RootAnchor::DraftRetirement(op.operation, op.digest()?);
    assert_eq!(
        crate::commerce::roots::RootAnchor::parse(&anchor.record())?,
        anchor
    );
    assert!(
        crate::commerce::roots::RootAnchor::parse(&format!("{}/extra", anchor.record())).is_err()
    );
    assert!(
        action
            .references()
            .iter()
            .all(|reference| reference.key().kind.as_str() == "owner_participant")
    );
    for (edit, revision, valid) in [
        ("0", "none", true),
        ("9", "7", true),
        ("00", "none", false),
        ("2", "0", false),
        ("2", "3", false),
        ("+2", "1", false),
    ] {
        assert_eq!(
            super::super::Request {
                mind: None,
                name: "child".into(),
                edit: edit.into(),
                revision: revision.into()
            }
            .base()
            .is_ok(),
            valid
        );
    }
    Ok(())
}

#[test]
fn captured_retirement_keeps_old_envelope_and_reconstructs_only_known_action_versions() -> TestResult
{
    let old = maximum()?;
    let old_bytes = serde_json::to_vec(&old)?;
    let old_decoded: RetirementOperation = serde_json::from_slice(&old_bytes)?;
    assert_eq!(serde_json::to_vec(&old_decoded)?, old_bytes);
    old_decoded.validate()?;
    let mut captured = old.clone();
    captured.action = super::super::captured_action_reference()?;
    captured.validate()?;
    captured.tombstone()?.validate()?;
    let mut expected = serde_json::to_value(&old)?;
    *expected.get_mut("action").ok_or("action")? = serde_json::to_value(&captured.action)?;
    assert_eq!(
        serde_json::to_value(&captured)?,
        expected,
        "only exact action identity differs in retained format1"
    );
    assert!(
        mush_core::canonical_json_bytes_with_limit(
            &envelope(&captured)?,
            RetirementOperation::MAX_BYTES
        )
        .is_ok()
    );
    Ok(())
}
