//! Structural profile/cause tests only. These rows are not Registry evidence.
use super::*;
use crate::commerce::RevisionIdentity;
use crate::commerce::rating::{RatingClass, RatingCompatibility, RatingSubject, test_support};
use mush_core::{
    ClockRef, ClockStamp, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, GOD, ObjId, OwnerId,
};
use serde_json::{Value, json};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
pub(super) fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
pub(super) fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
pub(super) fn manifest(n: u8) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_manifest")?,
            DefinitionId::new("combat")?,
            1,
            json!({"fixture":n}),
        )?,
    )?)
}
pub(super) fn complete(n: u8) -> Result<test_support::CompleteRating> {
    let identity: RevisionIdentity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash(1)},"source":hash(2)}))?;
    Ok(test_support::complete_rating(
        RatingSubject::new(identity, hash(3), hash(4), hash(5), hash(6), RatingClass::I),
        RatingCompatibility::combat_v1(manifest(n)?.digest()),
        hash(n),
        token(n),
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?)
}
#[test]
fn admission_requires_explicit_previous_and_preserves_absent_historical_bytes() -> Result {
    let old = complete(9)?;
    let historical = serde_json::to_vec(old.execution.admission())?;
    let round: crate::commerce::rating::execution::ExecutionAdmission =
        serde_json::from_slice(&historical)?;
    check_eq!(serde_json::to_vec(&round)?, historical);
    check!(
        !serde_json::to_value(&round)?
            .as_object()
            .ok_or("object")?
            .contains_key("reconformance")
    );
    let binding = ReconformanceAdmission::new(
        FiledRevisionId::new(1)?,
        complete(8)?.receipt.reference()?,
        None,
    )?;
    let wire = serde_json::to_value(&binding)?;
    check_eq!(wire.get("previous_attempt"), Some(&Value::Null));
    let mut missing = wire.clone();
    missing
        .as_object_mut()
        .ok_or("object")?
        .remove("previous_attempt");
    check!(serde_json::from_value::<ReconformanceAdmission>(missing).is_err());
    let mut extra = wire;
    extra
        .as_object_mut()
        .ok_or("object")?
        .insert("authorized".into(), json!(true));
    check!(serde_json::from_value::<ReconformanceAdmission>(extra).is_err());
    check!(
        ReconformanceAdmission::new(
            binding.filed(),
            binding.birth_rating(),
            Some(binding.birth_rating().job())
        )
        .is_err()
    );
    let mut marked = round.clone();
    marked.reconformance = Some(binding);
    marked.measured_rejection = true;
    check!(
        marked.validate().is_err(),
        "the saved author is not the Registry service actor"
    );
    marked.character = GOD;
    marked.validate()?;
    marked.measured_rejection = false;
    check!(marked.validate().is_err());
    let mut null = serde_json::to_value(round)?;
    null.as_object_mut()
        .ok_or("object")?
        .insert("reconformance".into(), Value::Null);
    check!(
        serde_json::from_value::<crate::commerce::rating::execution::ExecutionAdmission>(null)
            .is_err()
    );
    Ok(())
}
#[test]
fn exact_profile_and_head_identity_bind_full_manifest_and_filed_revision() -> Result {
    use crate::commerce::rating::registration::{ReconformanceDefinitions, RegistryDefinitions};
    let first = manifest(1)?;
    let later = manifest(2)?;
    let ordinary = RegistryDefinitions::new(first.clone())?;
    let current = ReconformanceDefinitions::new(first.clone())?;
    let changed = ReconformanceDefinitions::new(later)?;
    for ((old, service), new) in ordinary
        .actions()
        .iter()
        .zip(current.actions())
        .zip(changed.actions())
    {
        check_ne!(old, service);
        check_ne!(service, new);
    }
    let row = ReconformanceHead {
        format: 1,
        filed: FiledRevisionId::new(1)?,
        compatibility: first.digest(),
        job: token(9),
        admitted_at_utc: 100,
    };
    row.validate()?;
    check_eq!(row.key().len(), 64);
    check_ne!(
        row.key(),
        ReconformanceHead::key_for(FiledRevisionId::new(2)?, first.digest())?
    );
    check_ne!(row.key(), ReconformanceHead::key_for(row.filed, hash(99))?);
    let mut wrong = row.clone();
    wrong.admitted_at_utc = -1;
    check!(wrong.validate().is_err());
    let mut wire = serde_json::to_value(row)?;
    wire.as_object_mut()
        .ok_or("object")?
        .insert("certified".into(), json!(true));
    check!(serde_json::from_value::<ReconformanceHead>(wire).is_err());
    Ok(())
}
#[test]
fn measured_cause_is_closed_separate_from_manual_recall_and_has_full_envelope_room() -> Result {
    use crate::commerce::rating::nonconformance::NonconformanceRef;
    use recall::records::MeasuredRecall;
    let old = complete(8)?;
    let m = manifest(9)?;
    let rejection: NonconformanceRef = serde_json::from_value(
        json!({"job":token(9),"id":hash(20),"receipt_digest":hash(21),"schema":1}),
    )?;
    let row = MeasuredRecall {
        format: 1,
        operation: token(10),
        action: recall::protocol::action(&m)?,
        manifest: m,
        actor: GOD,
        filed: FiledRevisionId::new(1)?,
        revision: old.receipt.subject().revision(),
        birth_rating: old.receipt.reference()?,
        rejection,
        plan: hash(9),
        withdrawn_by: Some(token(6)),
        at_ic: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 101,
        },
        at_utc: 1000,
    };
    row.validate()?;
    check_eq!(row.status().withdrawn_by, Some(token(6)));
    check_eq!(row.status().recalled_by, Some(token(10)));
    check_eq!(row.root()?.identity, row.revision);
    let wire = serde_json::to_value(&row)?;
    check_eq!(serde_json::from_value::<MeasuredRecall>(wire.clone())?, row);
    for field in ["withdrawn_by", "rejection", "manifest"] {
        let mut missing = wire.clone();
        missing.as_object_mut().ok_or("object")?.remove(field);
        check!(serde_json::from_value::<MeasuredRecall>(missing).is_err());
    }
    let mut actor = row.clone();
    actor.actor = ObjId(7);
    check!(actor.validate().is_err());
    let mut crossed = row.clone();
    crossed.manifest = manifest(4)?;
    check!(crossed.validate().is_err());
    let mut crossed = row.clone();
    crossed.operation = crossed.rejection.job();
    check!(crossed.validate().is_err());
    let mut wire = wire;
    wire.as_object_mut()
        .ok_or("object")?
        .insert("operator".into(), json!(GOD));
    check!(serde_json::from_value::<MeasuredRecall>(wire).is_err());
    Ok(())
}
