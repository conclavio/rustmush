#![cfg(test)]

//! Codec/root coherence only. These deliberately structural rows are not a
//! certificate, grant or healthy deployment; the Tacsim child uses real owners.
use super::*;
use crate::commerce::dependent_owner::{DependentParent, OwnerParentBinding};
use crate::commerce::filed::ParentContent;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn dependent() -> TestResult<(
    FiledRecord,
    OwnerTerm,
    InitialFilingOperation,
    OwnerParentBinding,
)> {
    let (_, _, _, parent_revision) = saved();
    let parent = DependentParent {
        owner: ACTOR,
        parent: ParentContent {
            filed: FiledRevisionId::new(9)?,
            revision: RevisionIdentity::capture(&parent_revision)?,
        },
        charter: CharterId::new(90)?,
        line: LineId::new(9)?,
        licence: LicenceId::new(9)?,
        term: TermGeneration::new(1)?,
        window: TermWindow::new(now(10), now(10), now(180), now(195))?,
        term_seconds: 100,
        grace_seconds: 20,
        holder_capture: None,
    };
    let mut revision = parent_revision;
    revision.format = 2;
    revision.reference.key.mind += 1;
    revision.provenance = Some(DerivativeProvenance {
        parent: parent.parent,
        charter: parent.charter,
        line: parent.line,
    });
    revision.validate()?;
    let (mut filed, mut term, mut operation) = rows(&revision);
    filed.format = 2;
    filed.content = FiledRevisionContent::capture(
        filed.content.id(),
        filed.content.position(),
        &revision,
        filed.content.filed_at(),
        Some(parent.parent),
    )?;
    term.format = 2;
    term.window = parent.child_window(filed.content.filed_at())?;
    term.parent = Some(parent.clone());
    operation.format = 3;
    operation.parent = Some(parent.clone());
    let binding = OwnerParentBinding {
        format: 1,
        licence: operation.licence,
        term: operation.term,
        charter: operation.charter,
        filed: operation.filed,
        parent,
        operation: operation.commit,
    };
    binding.check_rows(&filed, &term, &operation)?;
    Ok((filed, term, operation, binding))
}

#[test]
fn dependent_formats_require_parent_and_original_bytes_stay_omitted() -> TestResult {
    let (_, _, _, original) = saved();
    let (filed, term, operation) = rows(&original);
    for value in [
        serde_json::to_value(&term)?,
        serde_json::to_value(&operation)?,
    ] {
        assert!(value.get("parent").is_none());
    }
    assert_eq!(
        serde_json::to_vec(&term)?,
        serde_json::to_vec(&serde_json::from_slice::<OwnerTerm>(&serde_json::to_vec(
            &term
        )?)?)?
    );
    assert_eq!(
        serde_json::to_vec(&operation)?,
        serde_json::to_vec(&serde_json::from_slice::<InitialFilingOperation>(
            &serde_json::to_vec(&operation)?
        )?)?
    );
    let (child, child_term, child_op, _) = dependent()?;
    for (original, dependent, required_format) in [
        (
            serde_json::to_value(&filed)?,
            serde_json::to_value(&child)?,
            2,
        ),
        (
            serde_json::to_value(&term)?,
            serde_json::to_value(&child_term)?,
            2,
        ),
        (
            serde_json::to_value(&operation)?,
            serde_json::to_value(&child_op)?,
            3,
        ),
    ] {
        let decode = |v: Value| -> bool {
            if v.get("content").is_some() {
                serde_json::from_value::<FiledRecord>(v).is_ok()
            } else if v.get("window").is_some() {
                serde_json::from_value::<OwnerTerm>(v).is_ok()
            } else {
                serde_json::from_value::<InitialFilingOperation>(v).is_ok()
            }
        };
        assert!(decode(original.clone()) && decode(dependent.clone()));
        let mut missing = original.clone();
        *missing.get_mut("format").ok_or("format")? = json!(required_format);
        assert!(!decode(missing), "new format cannot default away ancestry");
        let mut old_format = dependent.clone();
        *old_format.get_mut("format").ok_or("format")? = json!(1);
        assert!(
            !decode(old_format),
            "old format cannot carry dependent authority"
        );
        if original.get("content").is_none() {
            let mut null = original;
            null.as_object_mut()
                .ok_or("row")?
                .insert("parent".into(), Value::Null);
            assert!(!decode(null), "present null is not omitted legacy bytes");
            let mut null = dependent.clone();
            *null.get_mut("parent").ok_or("parent")? = Value::Null;
            assert!(!decode(null));
            let mut unknown = dependent;
            unknown
                .get_mut("parent")
                .and_then(Value::as_object_mut)
                .ok_or("parent")?
                .insert("authority".into(), json!(true));
            assert!(!decode(unknown));
        }
    }
    Ok(())
}

#[test]
fn dependent_operation_refuses_house_and_crossed_qualification() -> TestResult {
    let (_, _, operation, _) = dependent()?;
    for mode in 0..6 {
        let mut crossed = serde_json::to_value(&operation)?;
        match mode {
            0 => *crossed.get_mut("format").ok_or("format")? = json!(2),
            1 => {
                crossed
                    .get_mut("economic")
                    .and_then(Value::as_object_mut)
                    .ok_or("economic")?
                    .insert(
                        "house".into(),
                        serde_json::to_value(
                            mush_economy::registry_filing::HouseFilingAttribution::new(
                                ACTOR,
                                mush_org::OrgId(7),
                                mush_org::OrgScopeId(9),
                            )?,
                        )?,
                    );
            }
            2 => {
                *crossed
                    .get_mut("parent")
                    .and_then(|p| p.get_mut("owner"))
                    .ok_or("owner")? = json!(ObjId(ACTOR.0 + 1))
            }
            3 => {
                *crossed
                    .get_mut("parent")
                    .and_then(|p| p.get_mut("licence"))
                    .ok_or("licence")? = json!(operation.licence)
            }
            4 => {
                *crossed
                    .get_mut("parent")
                    .and_then(|p| p.get_mut("term_seconds"))
                    .ok_or("duration")? = json!(0)
            }
            _ => {
                *crossed
                    .get_mut("parent")
                    .and_then(|p| p.get_mut("grace_seconds"))
                    .ok_or("duration")? = json!(super::super::MAX_GRACE_SECONDS + 1)
            }
        }
        assert!(
            serde_json::from_value::<InitialFilingOperation>(crossed).is_err(),
            "mode {mode}"
        );
    }
    let encoded = serde_json::to_string(&operation)?;
    let repeated = encoded.replacen("\"parent\":", "\"parent\":null,\"parent\":", 1);
    assert!(serde_json::from_str::<InitialFilingOperation>(&repeated).is_err());
    Ok(())
}

#[test]
fn owner_binding_refuses_crossed_child_term_operation_and_owner() -> TestResult {
    let (filed, term, operation, binding) = dependent()?;
    for mode in 0..8 {
        let (mut f, mut t, mut o, mut b) = (
            filed.clone(),
            term.clone(),
            operation.clone(),
            binding.clone(),
        );
        match mode {
            0 => b.parent.owner = ObjId(ACTOR.0 + 1),
            1 => b.operation = token(3),
            2 => b.licence = LicenceId::new(2)?,
            3 => b.filed = FiledRevisionId::new(2)?,
            4 => t.window = TermWindow::new(now(100), now(100), now(180), now(196))?,
            5 => o.parent.as_mut().ok_or("parent")?.term = TermGeneration::new(2)?,
            6 => f.operation = token(3),
            _ => t.parent.as_mut().ok_or("parent")?.line = LineId::new(8)?,
        }
        assert!(b.check_rows(&f, &t, &o).is_err(), "mode {mode}");
    }
    Ok(())
}

#[test]
fn parent_root_belongs_to_exact_owner_licence_and_retains_parent_identity() -> TestResult {
    let (filed, term, operation, binding) = dependent()?;
    let roots = InitialRoots::from_records(&filed, &term, &operation)?;
    let expected = RootAnchor::OwnerParent(operation.licence, ACTOR);
    let actual = roots
        .roots()
        .iter()
        .find(|root| root.anchor == expected)
        .ok_or("parent root")?;
    assert_eq!(actual.identity, binding.parent.parent.revision);
    assert_ne!(actual.identity, operation.revision);
    assert_eq!(RootAnchor::parse(&expected.record())?, expected);
    assert_ne!(
        expected.record(),
        RootAnchor::OwnerParent(operation.licence, ObjId(ACTOR.0 + 1)).record()
    );
    assert_ne!(
        expected.record(),
        RootAnchor::OwnerParent(LicenceId::new(2)?, ACTOR).record()
    );
    for bad in [
        "v1/owner_parent/0000000000000001/00000000",
        "v1/owner_parent/0000000000000001/0000000A",
        "v1/owner_parent/0000000000000001/0000000a/extra",
    ] {
        assert!(RootAnchor::parse(bad).is_err());
    }
    assert!(super::super::families::valid_key(
        OwnerParentBinding::FAMILY,
        &binding.key()
    ));
    assert!(!super::super::families::valid_key(
        OwnerParentBinding::FAMILY,
        &format!("{}/extra", binding.key())
    ));
    let mut crossed = term;
    crossed.parent.as_mut().ok_or("parent")?.licence = LicenceId::new(8)?;
    assert!(InitialRoots::from_records(&filed, &crossed, &operation).is_err());
    Ok(())
}

#[test]
fn captured_parent_window_caps_child_and_never_restarts_parent_grace() -> TestResult {
    let (_, _, _, binding) = dependent()?;
    let parent = binding.parent;
    let active = parent.child_window(now(100))?;
    assert_eq!(
        (active.term_end(), active.grace_end()),
        (now(180), now(195))
    );
    let grace = parent.child_window(now(185))?;
    assert_eq!(
        (grace.issued_at(), grace.term_end(), grace.grace_end()),
        (now(185), now(180), now(195))
    );
    assert_eq!(grace.phase_at(now(185)), super::super::TermPhase::Grace);
    assert!(parent.child_window(now(195)).is_err());
    // A later current grant can reopen a separate eligibility fence, but it
    // cannot change this immutable child's encoded historical issue/window.
    let old = serde_json::to_vec(&parent)?;
    let mut current = parent.clone();
    current.window = TermWindow::new(now(200), now(200), now(400), now(420))?;
    assert_ne!(current.child_window(now(201))?, grace);
    assert_eq!(serde_json::to_vec(&parent)?, old);
    assert_eq!(grace.phase_at(now(201)), super::super::TermPhase::Lapsed);
    Ok(())
}

fn source_holder_capture(at: i64) -> Value {
    json!({"format":1,"basis":{"format":1,"acceptance":token(249),
        "ownership":{"generation":65537,"operation":token(248),"digest":digest(255)}},
        "observed_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":at}})
}

#[test]
fn accepted_source_holder_is_versioned_and_old_draft_and_parent_bytes_stay_exact() -> TestResult {
    use crate::commerce::parent_selector::DraftParentSelection;
    let (_, _, _, binding) = dependent()?;
    let old_parent = serde_json::to_vec(&binding.parent)?;
    assert_eq!(
        serde_json::to_vec(&serde_json::from_slice::<DependentParent>(&old_parent)?)?,
        old_parent
    );
    assert!(
        serde_json::to_value(&binding.parent)?
            .get("holder_capture")
            .is_none()
    );
    let old = json!({"format":1,"actor":ACTOR,"mind":u64::MAX,"parent":binding.parent.parent,
        "charter":binding.parent.charter,"line":binding.parent.line,"licence":binding.parent.licence,
        "created_parent_term":binding.parent.term,"generation":1,"created":token(250),"operation":token(250)});
    let legacy: DraftParentSelection = serde_json::from_value(old.clone())?;
    legacy.validate()?;
    assert_eq!(serde_json::to_value(&legacy)?, old);
    let mut current = old.clone();
    current["format"] = json!(2);
    current["holder_capture"] = source_holder_capture(i64::MAX);
    let captured: DraftParentSelection = serde_json::from_value(current.clone())?;
    captured.validate()?;
    assert_eq!(serde_json::to_value(&captured)?, current);
    let (mut engine, f) = engine();
    let document = json!({"family":{"owner":f.draft_parents.owner(),"id":f.draft_parents.descriptor().id()},
        "key":captured.key(),"schema":{"id":f.draft_parents.descriptor().value_schema(),"version":1},"value":captured});
    mush_core::canonical_json_bytes_with_limit(&document, DraftParentSelection::MAX_BYTES)?;
    let mut txn = engine.action_transaction(&f.draft_parents)?;
    txn.insert(&f.draft_parents, &captured.key(), captured)?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    for (format, present) in [(1, true), (2, false), (3, true)] {
        let mut bad = current.clone();
        bad["format"] = json!(format);
        if !present {
            bad.as_object_mut()
                .ok_or("selector")?
                .remove("holder_capture");
        }
        let bad: DraftParentSelection = serde_json::from_value(bad)?;
        assert!(bad.validate().is_err());
    }
    for (path, value) in [
        ("/holder_capture", Value::Null),
        ("/holder_capture/format", json!(2)),
        ("/holder_capture/basis/ownership/generation", json!(1)),
        ("/holder_capture/observed_ic/clock/kind", json!("world_utc")),
    ] {
        let mut bad = current.clone();
        *bad.pointer_mut(path).ok_or("capture field")? = value;
        assert!(serde_json::from_value::<DraftParentSelection>(bad).is_err());
    }
    let mut extra = current;
    extra
        .pointer_mut("/holder_capture")
        .and_then(Value::as_object_mut)
        .ok_or("capture")?
        .insert("authority".into(), json!(true));
    assert!(serde_json::from_value::<DraftParentSelection>(extra).is_err());
    Ok(())
}

#[test]
fn accepted_initial_parent_capture_is_paired_with_actual_issue_boundary_and_envelopes() -> TestResult
{
    let (filed, mut term, mut operation, mut binding) = dependent()?;
    let mut wire = serde_json::to_value(&binding.parent)?;
    wire["holder_capture"] = source_holder_capture(filed.content.filed_at().get());
    binding.parent = serde_json::from_value(wire.clone())?;
    term.parent = Some(binding.parent.clone());
    operation.parent = Some(binding.parent.clone());
    binding.check_rows(&filed, &term, &operation)?;
    let (mut engine, f) = engine();
    for (family, key, schema, limit, value) in [
        (
            f.owner_parents.descriptor().id(),
            binding.key(),
            f.owner_parents.descriptor().value_schema(),
            OwnerParentBinding::MAX_BYTES,
            serde_json::to_value(&binding)?,
        ),
        (
            f.terms.descriptor().id(),
            term.key(),
            f.terms.descriptor().value_schema(),
            OwnerTerm::MAX_BYTES,
            serde_json::to_value(&term)?,
        ),
        (
            f.operations.descriptor().id(),
            operation.key(),
            f.operations.descriptor().value_schema(),
            InitialFilingOperation::MAX_BYTES,
            serde_json::to_value(&operation)?,
        ),
    ] {
        mush_core::canonical_json_bytes_with_limit(
            &json!({"family":{"owner":"roll","id":family},"key":key,"schema":{"id":schema,"version":1},"value":value}),
            limit,
        )?;
    }
    let mut txn = engine.action_transaction(&f.owner_parents)?;
    txn.insert(&f.owner_parents, &binding.key(), binding.clone())?;
    txn.insert(&f.terms, &term.key(), term.clone())?;
    txn.insert(&f.operations, &operation.key(), operation.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    binding
        .parent
        .holder_capture
        .as_mut()
        .ok_or("capture")?
        .observed_ic
        .at += 1;
    assert!(binding.check_rows(&filed, &term, &operation).is_err());
    wire["holder_capture"] = Value::Null;
    assert!(serde_json::from_value::<DependentParent>(wire).is_err());
    Ok(())
}
