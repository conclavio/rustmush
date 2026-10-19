//! Actual owner-image/drop tests. These structural subjects do not claim a
//! Registry worker pass; real producer-to-filing acceptance is a separate gate.
use super::super::model::{ArtifactBytes, BulkConfig, Opponent, Plan, Scenario, WorkKind};
use super::super::registry_budget::{RegistryBudget, RegistryCase};
use super::super::registry_tickets::{AdmissionData, Binding};
use super::*;
use mush_core::{CommitToken, DefinitionDigest};
use mush_roll::commerce::rating::{
    CaseKey, CaseKind, ProbeName, RatingClass, RatingCompatibility, RatingSubject,
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn prepared() -> TestResult<AdmissionData> {
    let hash = DefinitionDigest::from_bytes([1; 32]);
    let revision = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash},"source":hash}))?;
    let subject = RatingSubject::new(revision, hash, hash, hash, hash, RatingClass::I);
    let start = ArtifactBytes::new(br#"{"fixture":"owner-image"}"#.to_vec())?;
    let plan = Plan {
        kind: WorkKind::Registry,
        registry: Some(RegistryBudget::prepare(
            subject.digest()?,
            hash,
            hash,
            vec![RegistryCase {
                key: CaseKey::new(
                    RatingClass::I,
                    CaseKind::Probe {
                        pair: 0,
                        name: ProbeName::EmptyArmed,
                    },
                )?,
                units: 2,
                native_ms: 250,
                maximum_ms: 10_000,
            }],
        )?),
        scenario: Scenario::Duel,
        opponent: Opponent::Doctrine,
        per_side: 0,
        unit_count: 0,
        games: 0,
        seeds: vec![],
        limit_ms: 0,
        native_substep_ms: 0,
        start: start.descriptor().clone(),
        result_reservation_bytes: 8 * 1024 * 1024,
    };
    Ok(AdmissionData {
        account: AccountId::mint("registry-owner-image", 1)?,
        character: ObjId(7),
        nonce: 1,
        subject,
        compatibility: RatingCompatibility::combat_v1(hash),
        plan,
        start,
    })
}

fn policy() -> TestResult<Policy> {
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };
    let manifest = DefinitionRegistryBuilder::new().register(
        OwnerId::new("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_manifest")?,
            DefinitionId::new("combat")?,
            1,
            json!({"fixture":true}),
        )?,
    )?;
    let refs = mush_roll::commerce::rating::registration::RegistryDefinitions::new(manifest)?;
    Ok(Policy::new(refs.actions().clone())?)
}

#[test]
fn dropped_bound_artifacts_keep_ticket_and_bytes_and_exact_reentry_consumes_once() -> TestResult {
    let data = prepared()?;
    let account = data.account;
    let character = data.character;
    let job = CommitToken::from_bytes([9; 16]);
    let mut quota = BulkStore::new(BulkConfig::default())?;
    let (run_id, admitted) =
        quota.prepare_registry_admit(account, character, 1, data.plan.clone(), job, 0)?;
    let run = admitted.resulting_run(run_id).ok_or("admitted run")?;
    let admission = registry_projection::admission(account, job, run)?;
    let initial = registry_projection::observation(run)?;
    let subject = (data.subject.clone(), data.compatibility.clone());
    let mut bytes = data.start.clone();
    let key = Key {
        account,
        run: run_id,
        part: Part::Start,
    };
    let blob = PreparedBlob::prepare(key, &mut bytes, vec![])?;
    let expected = serde_json::to_value(&blob.manifest)?;
    let mut artifacts = Artifacts::default();
    let id = artifacts.registry.admit(data)?;
    let ticket = artifacts.registry.get(id)?;
    let output = QuotaOutput::observed(
        job,
        job,
        Transition::Admit,
        id,
        admission,
        None,
        initial,
        Some(subject),
    )?;
    let participant = Arc::new(ArtifactParticipant::new(policy()?));
    let patch = || -> TestResult<ArtifactPatch> {
        let TicketKind::Admit(data) = &ticket.kind else {
            return Err("admit ticket".into());
        };
        let mut bytes = data.start.clone();
        Ok(ArtifactPatch {
            quota: output.clone(),
            ticket: Arc::clone(&ticket),
            blob: Some(PreparedBlob::prepare(key, &mut bytes, vec![])?),
            existing: None,
        })
    };
    // A durable Unknown/drop before core's receipt-authorized apply is not an
    // owner acknowledgement. The unchanged exact ticket can be rebound later.
    drop(Arc::clone(&participant).bind(&mut artifacts, patch()?)?);
    assert!(artifacts.get(key).is_err());
    artifacts.registry.check(id, &ticket)?;
    let bound = Arc::clone(&participant).bind(&mut artifacts, patch()?)?;
    bound.apply();
    assert_eq!(serde_json::to_value(artifacts.get(key)?)?, expected);
    assert!(artifacts.registry.get(id).is_err());
    assert!(
        Arc::clone(&participant)
            .bind(&mut artifacts, patch()?)
            .is_err()
    );
    let ack = artifacts
        .registry
        .take_acknowledged()
        .ok_or("acknowledgement")?;
    assert_eq!(
        (ack.ticket, ack.operation, ack.job, ack.transition),
        (id, job, job, Transition::Admit)
    );
    assert!(artifacts.registry.take_acknowledged().is_none());
    quota.check_registry(&admitted, job)?;
    quota.apply_registry_checked(admitted, job);
    assert!(quota.take_registry_lease(job).is_none());
    Ok(())
}

#[test]
fn progress_rechecks_the_exact_start_manifest_and_keeps_failed_ticket_ownership() -> TestResult {
    let data = prepared()?;
    let account = data.account;
    let character = data.character;
    let job = CommitToken::from_bytes([11; 16]);
    let operation = CommitToken::from_bytes([12; 16]);
    let mut quota = BulkStore::new(BulkConfig::default())?;
    let (run, admitted) =
        quota.prepare_registry_admit(account, character, 1, data.plan.clone(), job, 0)?;
    quota.check_registry(&admitted, job)?;
    quota.apply_registry_checked(admitted, job);
    let before = registry_projection::observation(quota.run(account, character, run)?)?;
    let granted = quota.prepare_grant(account, run, 0, 2000)?;
    let next = granted.resulting_run(run).ok_or("granted run")?;
    let admission = registry_projection::admission(account, job, next)?;
    let after = registry_projection::observation(next)?;
    let key = Key {
        account,
        run,
        part: Part::Start,
    };
    let mut bytes = data.start;
    let blob = PreparedBlob::prepare(key, &mut bytes, vec![])?;
    let original = blob.manifest.clone();
    let mut artifacts = Artifacts::default();
    artifacts.install(blob.manifest);
    let id = artifacts.registry.grant(
        Binding {
            job,
            account,
            character,
            run,
        },
        2000,
    )?;
    let ticket = artifacts.registry.get(id)?;
    let output = QuotaOutput::observed(
        operation,
        job,
        Transition::Grant,
        id,
        admission,
        Some(before),
        after,
        None,
    )?;
    let patch = ArtifactPatch {
        quota: output,
        ticket: Arc::clone(&ticket),
        blob: None,
        existing: Some((key, serde_json::to_value(&original)?)),
    };
    let mut altered = original.clone();
    altered.artifact.bytes += 1;
    artifacts.install(altered);
    assert!(check_artifacts(&artifacts, &patch).is_err());
    artifacts.registry.check(id, &ticket)?;
    artifacts.install(original);
    check_artifacts(&artifacts, &patch)?;
    let participant = Arc::new(ArtifactParticipant::new(policy()?));
    participant.bind(&mut artifacts, patch)?.apply();
    quota.check_registry(&granted, operation)?;
    quota.apply_registry_checked(granted, operation);
    let lease = quota
        .take_registry_lease(operation)
        .ok_or("acknowledged exact lease")?;
    assert_eq!(lease.run(), run);
    assert_eq!(lease.maximum_sim_ms(), 2000);
    assert!(quota.take_registry_lease(operation).is_none());
    Ok(())
}

#[test]
fn physical_write_admission_rejects_alias_keys_wrong_accounts_and_unknown_fields() -> TestResult {
    let mut data = prepared()?;
    let key = Key {
        account: data.account,
        run: RunId(1),
        part: Part::Start,
    };
    let blob = PreparedBlob::prepare(key, &mut data.start, vec![])?;
    for write in &blob.writes {
        let exact = replay_write(write)?;
        assert!(super::super::artifacts::accepts_registry_write(&exact));
        if let ReplayWrite::ModuleJsonSet {
            namespace,
            key: actual,
            value,
        } = exact
        {
            let aliased = actual.as_str().replace(":1:start", ":01:start");
            assert!(!super::super::artifacts::accepts_registry_write(
                &ReplayWrite::ModuleJsonSet {
                    namespace: namespace.clone(),
                    key: AuthoritativeKey::parse_canonical(aliased)?,
                    value: value.clone(),
                }
            ));
            if actual.as_str() == key.document().as_str() {
                let mut other = blob.manifest.clone();
                other.key.account = AccountId::mint("other-owner-image", 1)?;
                assert!(!super::super::artifacts::accepts_registry_write(
                    &ReplayWrite::ModuleJsonSet {
                        namespace: namespace.clone(),
                        key: actual.clone(),
                        value: serde_json::to_value(other)?,
                    }
                ));
                let mut extra = value;
                extra
                    .as_object_mut()
                    .ok_or("manifest object")?
                    .insert("authority".into(), json!(true));
                assert!(!super::super::artifacts::accepts_registry_write(
                    &ReplayWrite::ModuleJsonSet {
                        namespace,
                        key: actual,
                        value: extra
                    }
                ));
            }
        }
    }
    Ok(())
}
