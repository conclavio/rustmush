//! Coordinator protocol tests. These exercise actual typed transactions and
//! durable hydration; real paid worker/producer certification is tested by Roll.
use super::*;
use crate::action::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionRegistryBuilder};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
struct Fixture {
    store: ActionStateStore,
    handles: BTreeMap<String, Handle>,
    providers: ActionProviderRegistry,
    actions: [DefinitionRef; 5],
    invocation: ActionInvocation,
    manifest: DefinitionRef,
}
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_with_manifest(json!({}))
}
fn fixture_with_manifest(payload: Value) -> Result<Fixture, Box<dyn std::error::Error>> {
    let owner = OwnerId::new("roll")?;
    let owners = BTreeSet::from([owner.clone(), OwnerId::new("tacsim")?]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    let mut definitions = DefinitionRegistryBuilder::new();
    let manifest = definitions.register(
        OwnerId::new("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_manifest")?,
            DefinitionId::new("combat")?,
            1,
            payload,
        )?,
    )?;
    for (owner, descriptor) in closed_participants()? {
        definitions.register(owner, descriptor)?;
    }
    let mut actions = Vec::with_capacity(5);
    for descriptor in closed_actions(&manifest)? {
        actions.push(definitions.register(owner.clone(), descriptor)?);
    }
    let actions: [DefinitionRef; 5] = actions.try_into().map_err(|_| "action count")?;
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in &actions {
        providers.register(owner.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    let mut builder = StateFamilyRegistryBuilder::new();
    let mut handles = BTreeMap::new();
    for (name, bytes) in FAMILIES {
        let schema = |suffix: &str| CanonicalId::new(format!("roll.{name}.{suffix}.v1"));
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(name)?,
            schema("key")?,
            schema("value")?,
            1,
            schema("decode")?,
            schema("validate")?,
            schema("serialize")?,
        )?
        .with_limits(80, bytes)?;
        let handle = builder.register(
            owner.clone(),
            descriptor,
            |key: &String| Ok(key.clone()),
            |row: &Value| Ok(row.clone()),
            |_: &Value| Ok(()),
            |row: &Value| Ok(row.clone()),
        )?;
        handles.insert(name.to_string(), handle);
    }
    builder.registry_rating_capacity(&owner, manifest.clone(), actions.clone())?;
    let store =
        ActionStateStore::new(builder.freeze_for_owners_with_readiness(&owners, readiness)?);
    let request = RequestUid::from_bytes([1; 16]);
    let principal = PrincipalId::Object { id: ObjId(7) };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: ObjId(7),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner,
                namespace: CanonicalId::new("registry")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    Ok(Fixture {
        store,
        handles,
        providers,
        actions,
        invocation,
        manifest,
    })
}
fn rows(id: CommitToken, cases: u64, compatibility: DefinitionDigest) -> (Value, Value) {
    let subject = json!({"fixture":"coordinator_only"});
    let hash = DefinitionDigest::from_bytes([8; 32]);
    let job = json!({"format":1,"job":id,"subject":subject,"compatibility":{"digest":compatibility},"plan":hash,"state":{"kind":"queued"}});
    let execution = json!({"format":1,"job":id,"updated_by":id,
        "admission":{"job_account_binding":hash,"character":7,"run":1,"nonce":1,"subject":digest("tacsim.registry.subject.v1",&subject).unwrap(),
            "compatibility":compatibility,"plan":hash,"budget":hash,"start":{"sha256":hash,"bytes":1},"maximum_unit_ms":100000,"cases":cases,"maximum_pages":cases.div_ceil(15)},
        "observation":{"phase":"queued","next_sequence":1,"cursor":{"index":0,"simulated_ms":0},"charged_unit_ms":0,"flight":null,"result":null}});
    (job, execution)
}
fn admit(f: &mut Fixture, id: CommitToken, cases: u64) -> Result<PreparedTxn, ActionStateError> {
    let entry = f.providers.get_exact(&f.actions[0]).unwrap();
    let mut txn = f
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &f.invocation, id);
    let base = txn.read_base();
    let (job, execution) = rows(id, cases, f.manifest.digest());
    txn.insert(&f.handles["rating_jobs"], &id.to_hex(), job)?;
    txn.insert(&f.handles["rating_executions"], &id.to_hex(), execution)?;
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), id, None)
        .with_registered_transaction(&base, entry.registration_ordinal());
    context.reserve_registry_rating_completion(&mut txn)?;
    drop(context);
    txn.prepare()
}
fn install(
    store: &mut ActionStateStore,
    prepared: PreparedTxn,
) -> Result<Vec<ActionStateBootRecord>, ActionStateError> {
    let (ops, installation) = prepared.into_parts();
    let rows = ops
        .into_iter()
        .filter_map(|op| match op.op {
            ModuleWrite::JsonSet { key, value } => {
                Some(ActionStateBootRecord::new(op.ns, key, value))
            }
            _ => None,
        })
        .collect();
    store.install(installation)?;
    Ok(rows)
}

#[test]
fn admission_reserves_the_exact_maximum_and_survives_real_hydration() -> TestResult {
    let mut f = fixture()?;
    let id = CommitToken::from_bytes([5; 16]);
    let prepared = admit(&mut f, id, 1755)?;
    assert_eq!(prepared.operation_count(), 3);
    assert!(f.store.capacity.is_empty());
    let durable = install(&mut f.store, prepared)?;
    let reservation = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(reservation.record.targets.len(), 122);
    assert_eq!(
        reservation
            .record
            .targets
            .iter()
            .map(|target| target.ceiling)
            .sum::<usize>(),
        3_915_776
    );
    let mut restarted = fixture()?;
    restarted.store.hydrate(durable)?;
    assert_eq!(restarted.store.capacity_usage, f.store.capacity_usage);
    Ok(())
}

#[test]
fn zero_metadata_and_missing_paired_rows_refuse_boot() -> TestResult {
    let mut f = fixture()?;
    let prepared = admit(&mut f, CommitToken::from_bytes([5; 16]), 1)?;
    let durable = install(&mut f.store, prepared)?;
    for omit in ["action-state/capacity/", "rating_jobs", "rating_executions"] {
        let mut restarted = fixture()?;
        let records = durable
            .iter()
            .filter(|row| !row.key.contains(omit))
            .cloned()
            .collect::<Vec<_>>();
        assert!(restarted.store.hydrate(records).is_err(), "missing {omit}");
    }
    Ok(())
}

#[test]
fn ordinary_typed_insertion_cannot_bypass_the_closed_admission() -> TestResult {
    let mut f = fixture()?;
    let id = CommitToken::from_bytes([5; 16]);
    let (job, execution) = rows(id, 1, f.manifest.digest());
    let mut txn = f.store.transaction(&f.handles["rating_jobs"])?;
    txn.insert(&f.handles["rating_jobs"], &id.to_hex(), job)?;
    txn.insert(&f.handles["rating_executions"], &id.to_hex(), execution)?;
    assert!(txn.prepare().is_err());
    assert!(f.store.records.is_empty());
    Ok(())
}

#[test]
fn immutable_admission_and_non_sponsor_job_edits_are_both_fenced() -> TestResult {
    for family in ["rating_jobs", "rating_executions"] {
        let mut f = fixture()?;
        let id = CommitToken::from_bytes([5; 16]);
        let prepared = admit(&mut f, id, 1)?;
        install(&mut f.store, prepared)?;
        let mut txn = f.store.transaction(&f.handles[family])?;
        let (mut job, mut execution) = rows(id, 1, f.manifest.digest());
        job["plan"] = json!(DefinitionDigest::from_bytes([9; 32]));
        execution["admission"]["run"] = json!(2);
        txn.replace(
            &f.handles[family],
            &id.to_hex(),
            if family == "rating_jobs" {
                job
            } else {
                execution
            },
        )?;
        assert!(txn.prepare().is_err());
        assert_eq!(f.store.capacity.records.len(), 1);
    }
    Ok(())
}

#[test]
fn queued_cancel_retires_metadata_in_the_same_terminal_install() -> TestResult {
    let mut f = fixture()?;
    let id = CommitToken::from_bytes([5; 16]);
    let prepared = admit(&mut f, id, 1)?;
    install(&mut f.store, prepared)?;
    let commit = CommitToken::from_bytes([6; 16]);
    let entry = f.providers.get_exact(&f.actions[3])?;
    let mut txn = f
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &f.invocation, commit);
    let base = txn.read_base();
    let (mut job, mut execution) = rows(id, 1, f.manifest.digest());
    job["state"] = json!({"kind":"cancelled"});
    execution["observation"]["phase"] = json!("cancelled");
    execution["updated_by"] = json!(commit);
    txn.replace(&f.handles["rating_jobs"], &id.to_hex(), job)?;
    txn.replace(&f.handles["rating_executions"], &id.to_hex(), execution)?;
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, entry.registration_ordinal());
    context.finish_registry_rating_completion(&mut txn, id)?;
    drop(context);
    let prepared = txn.prepare()?;
    assert_eq!(f.store.capacity.records.len(), 1);
    let terminal = install(&mut f.store, prepared)?;
    assert!(f.store.capacity.is_empty());
    let mut restarted = fixture()?;
    restarted.store.hydrate(terminal)?;
    assert!(restarted.store.capacity.is_empty());
    Ok(())
}

#[test]
fn wrong_transaction_context_refuses_and_poisons_caught_failure() -> TestResult {
    let f = fixture()?;
    let mut other = fixture()?;
    let entry = f.providers.get_exact(&f.actions[0])?;
    let other_entry = other.providers.get_exact(&other.actions[0])?;
    let mut txn = other
        .store
        .transaction_for_provider(other_entry.registration_ordinal())?;
    let id = CommitToken::from_bytes([5; 16]);
    txn.bind_registered_invocation(other_entry, &other.invocation, id);
    let wrong_base = ActionReadBase {
        slot: Arc::clone(&f.store.slot),
        revision: f.store.revision,
    };
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), id, None)
        .with_registered_transaction(&wrong_base, entry.registration_ordinal());
    assert!(
        context
            .reserve_registry_rating_completion(&mut txn)
            .is_err()
    );
    assert!(context.owner_error().is_some());
    drop(context);
    assert!(txn.prepare().is_err());
    Ok(())
}

fn transition(
    f: &mut Fixture,
    id: CommitToken,
    action: usize,
    commit_byte: u8,
    terminal: bool,
    update: impl FnOnce(&mut Value, &mut Value),
) -> Result<PreparedTxn, ActionStateError> {
    let view = View {
        registry: &f.store.registry,
        records: &f.store.records,
        mutations: None,
    };
    let mut prior = pair(&view, id)?;
    update(&mut prior.job, &mut prior.execution);
    let commit = CommitToken::from_bytes([commit_byte; 16]);
    prior.execution["updated_by"] = json!(commit);
    let entry = f.providers.get_exact(&f.actions[action]).unwrap();
    let mut txn = f
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &f.invocation, commit);
    let base = txn.read_base();
    txn.replace(&f.handles["rating_jobs"], &id.to_hex(), prior.job)?;
    txn.replace(
        &f.handles["rating_executions"],
        &id.to_hex(),
        prior.execution,
    )?;
    if terminal {
        let mut context =
            crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
                .with_registered_transaction(&base, entry.registration_ordinal());
        context.finish_registry_rating_completion(&mut txn, id)?;
    }
    txn.prepare()
}

#[test]
fn paid_settlement_cannot_resurrect_a_cancel_requested_job() -> TestResult {
    let mut f = fixture()?;
    let id = CommitToken::from_bytes([5; 16]);
    let prepared = admit(&mut f, id, 1)?;
    install(&mut f.store, prepared)?;
    let grant = transition(&mut f, id, 1, 6, false, |job, execution| {
        job["state"] = json!({"kind":"running","lease_sequence":1});
        execution["observation"]["phase"] = json!("running");
        execution["observation"]["next_sequence"] = json!(2);
        execution["observation"]["charged_unit_ms"] = json!(400);
        execution["observation"]["flight"] = json!({"sequence":1,"utc_day":1,"allowance_unit_ms":400,"units":1,"native_ms":50,"cursor":{"index":0,"simulated_ms":0}});
    })?;
    install(&mut f.store, grant)?;
    let cancel = transition(&mut f, id, 3, 7, false, |_, execution| {
        execution["observation"]["phase"] = json!("cancel_requested");
    })?;
    install(&mut f.store, cancel)?;
    assert!(
        transition(&mut f, id, 2, 8, false, |_, execution| {
            execution["observation"]["phase"] = json!("running");
            execution["observation"]["flight"] = Value::Null;
        })
        .is_err()
    );
    assert_eq!(f.store.capacity.records.len(), 1);
    let settled = transition(&mut f, id, 2, 8, true, |job, execution| {
        job["state"] = json!({"kind":"cancelled"});
        execution["observation"]["phase"] = json!("cancelled");
        execution["observation"]["flight"] = Value::Null;
    })?;
    install(&mut f.store, settled)?;
    assert!(f.store.capacity.is_empty());
    Ok(())
}

fn merge_rows(
    before: Vec<ActionStateBootRecord>,
    after: Vec<ActionStateBootRecord>,
) -> Vec<ActionStateBootRecord> {
    before
        .into_iter()
        .chain(after)
        .map(|row| ((row.namespace.clone(), row.key.clone()), row))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

#[test]
fn changed_manifest_retains_exact_old_obligation_but_only_current_recover_can_finish_it()
-> TestResult {
    for running in [false, true] {
        let mut old = fixture()?;
        let id = CommitToken::from_bytes([21; 16]);
        let prepared = admit(&mut old, id, 1)?;
        let mut durable = install(&mut old.store, prepared)?;
        if running {
            let grant = transition(&mut old, id, 1, 22, false, |job, execution| {
                job["state"] = json!({"kind":"running","lease_sequence":1});
                execution["observation"]["phase"] = json!("running");
                execution["observation"]["next_sequence"] = json!(2);
                execution["observation"]["charged_unit_ms"] = json!(400);
                execution["observation"]["flight"] = json!({"sequence":1,"utc_day":1,"allowance_unit_ms":400,"units":1,"native_ms":50,"cursor":{"index":0,"simulated_ms":0}});
            })?;
            durable = merge_rows(durable, install(&mut old.store, grant)?);
        }
        let mut current = fixture_with_manifest(json!({"updated":true}))?;
        assert_ne!(old.manifest, current.manifest);
        assert_ne!(old.actions[0], current.actions[0]);
        current.store.hydrate(durable)?;
        assert_eq!(current.store.capacity_usage, old.store.capacity_usage);
        let retained = current
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("retained old obligation")?;
        assert_eq!(retained.record.effect, old.actions[0]);
        // Even exact old data cannot request public cancellation or another
        // paid execution under the newly registered manifest.
        assert!(
            transition(&mut current, id, 3, 23, true, |job, execution| {
                job["state"] = json!({"kind":"cancelled"});
                execution["observation"]["phase"] = json!("cancelled");
                execution["observation"]["flight"] = Value::Null;
            })
            .is_err()
        );
        if running {
            assert!(
                transition(&mut current, id, 2, 23, true, |job, execution| {
                    job["state"] = json!({"kind":"failed"});
                    execution["observation"]["phase"] = json!("failed");
                    execution["observation"]["flight"] = Value::Null;
                })
                .is_err()
            );
        } else {
            assert!(transition(&mut current,id,1,23,false,|job,execution| {
                job["state"]=json!({"kind":"running","lease_sequence":1});
                execution["observation"]["phase"]=json!("running");
                execution["observation"]["next_sequence"]=json!(2);
                execution["observation"]["charged_unit_ms"]=json!(400);
                execution["observation"]["flight"]=json!({"sequence":1,"utc_day":1,"allowance_unit_ms":400,"units":1,"native_ms":50,"cursor":{"index":0,"simulated_ms":0}});
            }).is_err());
        }
        let interrupted = transition(&mut current, id, 4, 23, true, |job, execution| {
            job["state"] = json!({"kind":"interrupted"});
            execution["observation"]["phase"] = json!("interrupted");
            execution["observation"]["flight"] = Value::Null;
        })?;
        assert_eq!(current.store.capacity.records.len(), 1);
        let terminal = install(&mut current.store, interrupted)?;
        assert!(current.store.capacity.is_empty());
        let job = current
            .store
            .get(&current.handles["rating_jobs"], &id.to_hex())?
            .ok_or("job")?;
        assert_eq!(job["compatibility"]["digest"], json!(old.manifest.digest()));
        let execution = current
            .store
            .get(&current.handles["rating_executions"], &id.to_hex())?
            .ok_or("execution")?;
        assert_eq!(
            execution["observation"]["charged_unit_ms"],
            json!(if running { 400 } else { 0 })
        );
        assert!(execution["observation"]["result"].is_null());
        let mut restarted = fixture_with_manifest(json!({"updated":true}))?;
        restarted.store.hydrate(terminal)?;
        assert!(restarted.store.capacity.is_empty());
    }
    Ok(())
}

#[test]
fn historical_effect_must_match_the_full_closed_reconstruction_and_persisted_digest() -> TestResult
{
    let mut old = fixture()?;
    let id = CommitToken::from_bytes([25; 16]);
    let prepared = admit(&mut old, id, 1)?;
    let durable = install(&mut old.store, prepared)?;
    let current = fixture_with_manifest(json!({"updated":true}))?;
    for replacement in [
        current.actions[0].clone(),
        DefinitionRef::new(
            old.actions[0].key().clone(),
            1,
            DefinitionDigest::from_bytes([77; 32]),
        )?,
    ] {
        let mut bad = durable.clone();
        let metadata = bad
            .iter_mut()
            .find(|row| row.key.starts_with(PREFIX))
            .ok_or("metadata")?;
        metadata.value["effect"] = json!(replacement);
        let mut restarted = fixture_with_manifest(json!({"updated":true}))?;
        assert!(restarted.store.hydrate(bad).is_err());
    }
    // Change both copied compatibility fields and their fingerprint, leaving
    // the old action reference intact: pair consistency alone is insufficient.
    let mut crossed = durable;
    let hash = DefinitionDigest::from_bytes([88; 32]);
    let job = crossed
        .iter_mut()
        .find(|row| row.key.contains("/rating_jobs/"))
        .ok_or("job")?;
    job.value["value"]["compatibility"]["digest"] = json!(hash);
    let job_value = job.value["value"].clone();
    let execution = crossed
        .iter_mut()
        .find(|row| row.key.contains("/rating_executions/"))
        .ok_or("execution")?;
    execution.value["value"]["admission"]["compatibility"] = json!(hash);
    let execution_value = execution.value["value"].clone();
    let metadata = crossed
        .iter_mut()
        .find(|row| row.key.starts_with(PREFIX))
        .ok_or("metadata")?;
    metadata.value["protocol_fingerprint"] = json!(fingerprint(&job_value, &execution_value)?);
    let mut restarted = fixture_with_manifest(json!({"updated":true}))?;
    assert!(restarted.store.hydrate(crossed).is_err());
    Ok(())
}

#[test]
fn current_admission_cannot_reserve_a_new_job_with_old_manifest_inputs() -> TestResult {
    let old = fixture()?;
    let mut current = fixture_with_manifest(json!({"updated":true}))?;
    let id = CommitToken::from_bytes([28; 16]);
    let entry = current.providers.get_exact(&current.actions[0])?;
    let mut txn = current
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &current.invocation, id);
    let base = txn.read_base();
    let (job, execution) = rows(id, 1, old.manifest.digest());
    txn.insert(&current.handles["rating_jobs"], &id.to_hex(), job)?;
    txn.insert(
        &current.handles["rating_executions"],
        &id.to_hex(),
        execution,
    )?;
    let mut context =
        crate::resolution::PrepareCtx::new(current.invocation.idempotency(), id, None)
            .with_registered_transaction(&base, entry.registration_ordinal());
    assert!(
        context
            .reserve_registry_rating_completion(&mut txn)
            .is_err()
    );
    drop(context);
    assert!(txn.prepare().is_err());
    assert!(current.store.records.is_empty());
    assert!(current.store.capacity.is_empty());
    Ok(())
}

#[test]
fn current_protocol_requires_actual_closed_action_and_participant_digests() -> TestResult {
    let f = fixture()?;
    let owner = OwnerId::new("roll")?;
    let mut wrong = f.actions.clone();
    wrong[0] = DefinitionRef::new(
        wrong[0].key().clone(),
        1,
        DefinitionDigest::from_bytes([99; 32]),
    )?;
    assert!(Protocol::new(&owner, f.manifest.clone(), wrong).is_err());
    let mut descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action")?,
        DefinitionId::new("rating_admit")?,
        1,
        json!({"format":1,"protocol":"registry_rating_v1","transition":"admit"}),
    )?;
    // All named identity fields are lawful; omitting the actual participant
    // references must still change the digest and refuse the declaration.
    descriptor = descriptor.with_references(vec![f.manifest.clone()]);
    let mut wrong = f.actions.clone();
    wrong[0] = DefinitionRegistryBuilder::new().register(owner.clone(), descriptor)?;
    assert!(Protocol::new(&owner, f.manifest, wrong).is_err());
    Ok(())
}

#[test]
fn unsuccessful_terminal_pair_refuses_a_retained_result_descriptor() -> TestResult {
    let mut f = fixture()?;
    let id = CommitToken::from_bytes([26; 16]);
    let prepared = admit(&mut f, id, 1)?;
    install(&mut f.store, prepared)?;
    for phase in ["failed", "cancelled", "interrupted"] {
        let mut execution = f
            .store
            .get(&f.handles["rating_executions"], &id.to_hex())?
            .ok_or("execution")?
            .clone();
        let mut job = f
            .store
            .get(&f.handles["rating_jobs"], &id.to_hex())?
            .ok_or("job")?
            .clone();
        job["state"] = json!({"kind":phase});
        execution["observation"]["phase"] = json!(phase);
        execution["observation"]["result"] =
            json!({"sha256":DefinitionDigest::from_bytes([4;32]),"bytes":100});
        execution["updated_by"] = json!(CommitToken::from_bytes([27; 16]));
        let job_key = key("rating_jobs", id, None)?.resolve(&f.store.registry)?;
        let execution_key = key("rating_executions", id, None)?.resolve(&f.store.registry)?;
        // Use real typed preparation to create canonical post-images; the pair
        // verifier must reject them before ordinary invocation checks matter.
        let mut txn = f.store.transaction(&f.handles["rating_jobs"])?;
        txn.replace(&f.handles["rating_jobs"], &id.to_hex(), job)?;
        txn.replace(&f.handles["rating_executions"], &id.to_hex(), execution)?;
        assert!(txn.mutations.contains_key(&job_key));
        assert!(txn.mutations.contains_key(&execution_key));
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        assert!(pair(&view, id).is_err());
    }
    assert_eq!(f.store.capacity.records.len(), 1);
    Ok(())
}
