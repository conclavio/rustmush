//! Private effect fixture only. It proves coordinator accounting/ack recovery,
//! not paid-contract authority, terminal outcome bounds, or all-missing audit.

use super::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<u64, Row>;
type Durable = BTreeMap<(SmolStr, SmolStr), Value>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Row {
    state: String,
    capacity: Option<Reservation>,
    text: String,
}

fn plain(text: &str) -> Row {
    Row {
        state: "plain".into(),
        capacity: None,
        text: text.into(),
    }
}

fn fixture(
    enabled: bool,
) -> Result<(ActionStateStore, Handle, DefinitionRef), Box<dyn std::error::Error>> {
    let owner = OwnerId::new("capacity_test")?;
    let owners = BTreeSet::from([owner.clone()]);
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new("rows")?,
        CanonicalId::new("decimal.v1")?,
        CanonicalId::new("row.v1")?,
        1,
        CanonicalId::new("decode.v1")?,
        CanonicalId::new("validate.v1")?,
        CanonicalId::new("encode.v1")?,
    )?
    .with_limits(20, 16 * 1024)?;
    let mut builder = StateFamilyRegistryBuilder::new();
    let handle = builder.register(
        owner.clone(),
        descriptor,
        |key: &u64| Ok(key.to_string()),
        |value: &Value| serde_json::from_value::<Row>(value.clone()).map_err(|e| e.to_string()),
        |_: &Row| Ok(()),
        |value: &Row| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let registry = builder
        .freeze_for_owners_with_readiness(&owners, MechanicalOwnerReadiness::for_owners(&owners))?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let effect = definitions.register(
        owner,
        DefinitionDescriptor::new(
            DefinitionKind::new("capacity_test")?,
            DefinitionId::new("fund")?,
            1,
            json!({"fixture_only": true}),
        )?,
    )?;
    let mut store = ActionStateStore::new(registry);
    if enabled {
        // No corresponding production constructor, registration method or
        // permission exists. This exact private fixture owns only Row above.
        store.capacity_verifiers.0.push(Verifier {
            effect: effect.clone(),
            verify: verify_fixture,
        });
    }
    Ok((store, handle, effect))
}

fn row_at<'a>(
    view: &'a View<'_>,
    key: &Key,
    prior: bool,
) -> Result<Option<&'a Row>, ActionStateError> {
    let state_key = StateKey {
        owner: key.owner.clone(),
        family: key.family.clone(),
        encoded: key.encoded.clone(),
    };
    let value = if prior {
        None
    } else {
        view.mutations.and_then(|m| m.get(&state_key))
    };
    match value {
        Some(TxnMutation::Put { value, .. }) => {
            downcast_value(value.as_ref(), &state_key).map(Some)
        }
        Some(TxnMutation::Delete) => Ok(None),
        None => view
            .records
            .get(&state_key)
            .map(|r| downcast_value(r.value.as_ref(), &state_key))
            .transpose(),
    }
}

fn verify_fixture(
    phase: Phase,
    record: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let current =
        row_at(view, &record.sponsor, false)?.ok_or_else(|| refused("fixture sponsor absent"))?;
    let expected = if matches!(phase, Phase::Terminal) {
        "closed"
    } else {
        "pending"
    };
    if current.state != expected || current.capacity.as_ref() != Some(record) {
        return Err(refused("fixture exact backlink"));
    }
    if matches!(phase, Phase::Funding) && row_at(view, &record.sponsor, true)?.is_some() {
        return Err(refused("fixture operation already exists"));
    }
    if matches!(phase, Phase::Terminal) {
        let prior =
            row_at(view, &record.sponsor, true)?.ok_or_else(|| refused("fixture prior absent"))?;
        if prior.state != "pending" || prior.capacity.as_ref() != Some(record) {
            return Err(refused("fixture prior backlink"));
        }
    }
    Ok(())
}

fn key(handle: &Handle, number: u64) -> Key {
    Key {
        owner: handle.owner().clone(),
        family: handle.descriptor().id.clone(),
        encoded: number.to_string().into(),
    }
}

fn reservation(
    handle: &Handle,
    effect: &DefinitionRef,
    token: u8,
    sponsor: u64,
    slots: &[(u64, SlotMode, usize)],
) -> Reservation {
    let mut targets = slots
        .iter()
        .map(|(number, mode, ceiling)| Target {
            key: key(handle, *number),
            schema: handle.descriptor().value_schema().clone(),
            schema_version: 1,
            ceiling: *ceiling,
            mode: *mode,
        })
        .collect::<Vec<_>>();
    targets.push(Target {
        key: key(handle, sponsor),
        schema: handle.descriptor().value_schema().clone(),
        schema_version: 1,
        ceiling: handle.descriptor().max_record_bytes(),
        mode: SlotMode::UpdateExisting,
    });
    targets.sort_by(|a, b| a.key.encoded.cmp(&b.key.encoded));
    Reservation {
        format: 1,
        funding: CommitToken::from_bytes([token; 16]),
        effect: effect.clone(),
        sponsor: key(handle, sponsor),
        protocol_fingerprint: None,
        targets,
    }
}

fn stage_funding(
    store: &mut ActionStateStore,
    handle: &Handle,
    sponsor: u64,
    record: Reservation,
) -> Result<PreparedTxn, ActionStateError> {
    let mut txn = store.transaction(handle)?;
    txn.insert(
        handle,
        &sponsor,
        Row {
            state: "pending".into(),
            capacity: Some(record.clone()),
            text: String::new(),
        },
    )?;
    txn.capacity_changes.funding = Some(record);
    txn.prepare()
}

fn persist(ops: Vec<ModuleOp>, durable: &mut Durable) -> TestResult {
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                durable.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                durable.remove(&(op.ns, key));
            }
            ModuleWrite::StreamAppend { .. } => return Err("unexpected stream".into()),
        }
    }
    Ok(())
}

fn acknowledge(
    store: &mut ActionStateStore,
    prepared: PreparedTxn,
    durable: &mut Durable,
) -> TestResult {
    let (ops, installation) = prepared.into_parts();
    persist(ops, durable)?;
    store.install(installation)?;
    Ok(())
}

fn boot(durable: &Durable) -> Vec<ActionStateBootRecord> {
    durable
        .iter()
        .map(|((ns, key), value)| {
            ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
        })
        .collect()
}

fn terminal(
    store: &mut ActionStateStore,
    handle: &Handle,
    sponsor: u64,
    record: &Reservation,
    extras: &[(u64, Row)],
) -> Result<PreparedTxn, ActionStateError> {
    let mut txn = store.transaction(handle)?;
    txn.replace(
        handle,
        &sponsor,
        Row {
            state: "closed".into(),
            capacity: Some(record.clone()),
            text: String::new(),
        },
    )?;
    for (key, row) in extras {
        txn.insert(handle, key, row.clone())?;
    }
    txn.capacity_changes.terminal = Some(ReservationId(
        record.sponsor.owner.clone(),
        record.funding.to_hex(),
    ));
    txn.prepare()
}

#[test]
fn reserve_ack_restart_and_terminal_ack_use_exact_durable_images() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    let mut durable = Durable::new();
    let prepared = stage_funding(&mut store, &handle, 1, record.clone())?;
    assert_eq!(prepared.operation_count(), 2);
    assert_eq!(prepared.authoritative_replay_writes().len(), 2);
    assert!(store.capacity.is_empty());
    acknowledge(&mut store, prepared, &mut durable)?;
    let expected_usage = store.capacity_usage;
    assert_eq!(expected_usage.records, 3); // operation, reserved receipt, metadata
    assert_eq!(
        expected_usage.bytes,
        16 * 1024 + 4096 + store.capacity.metadata_bytes()
    );
    let (mut recovered, recovered_handle, _) = fixture(true)?;
    recovered.hydrate(boot(&durable))?;
    assert_eq!(recovered.capacity_usage, expected_usage);
    assert_eq!(recovered.capacity.targets.len(), 2);
    let prepared = terminal(
        &mut recovered,
        &recovered_handle,
        1,
        &record,
        &[(2, plain("receipt"))],
    )?;
    let (ops, installation) = prepared.into_parts();
    persist(ops, &mut durable)?;
    // Complete only after all bound owners apply. A cache is never cleared on
    // mere preparation, submission, or typed bind.
    assert!(!recovered.capacity.is_empty());
    let bound = recovered.bind_installation(installation)?;
    let unsettled = bound.apply_images();
    assert!(unsettled._store.has_prepared());
    assert!(unsettled._store.capacity.is_empty());
    unsettled.complete();
    assert!(!recovered.has_prepared());
    assert_eq!(recovered.capacity_usage.records, 2);
    assert_eq!(
        recovered.capacity_usage.bytes,
        recovered.live_canonical_bytes
    );
    let (mut final_boot, _, _) = fixture(true)?;
    final_boot.hydrate(boot(&durable))?;
    assert_eq!(final_boot.capacity_usage, recovered.capacity_usage);
    Ok(())
}

#[test]
fn reserved_creation_and_required_existing_are_protected_from_ordinary_writes() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let mut durable = Durable::new();
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &8, plain("shared"))?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let record = reservation(
        &handle,
        &effect,
        1,
        1,
        &[
            (2, SlotMode::Create, 4096),
            (8, SlotMode::UpdateExisting, 4096),
        ],
    );
    let prepared = stage_funding(&mut store, &handle, 1, record)?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let before = store.capacity_usage;
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &2, plain("stolen"))?;
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::TerminalCapacity("reserved creation key"))
    ));
    let mut txn = store.transaction(&handle)?;
    txn.remove(&handle, &8)?;
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::TerminalCapacity(
            "required existing key deletion"
        ))
    ));
    let mut txn = store.transaction(&handle)?;
    txn.remove(&handle, &1)?;
    assert!(
        txn.prepare().is_err(),
        "operation backlink cannot disappear"
    );
    assert_eq!(store.capacity_usage, before);
    let mut txn = store.transaction(&handle)?;
    txn.replace(&handle, &8, plain("ordinary shared update"))?;
    txn.insert(&handle, &9, plain("unrelated"))?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    assert_eq!(store.capacity_usage.records, before.records + 1);
    assert!(store.get(&handle, &9)?.is_some());
    Ok(())
}

#[test]
fn shared_ceiling_is_charged_once_and_last_claim_controls_deletion() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let mut durable = Durable::new();
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &8, plain("shared"))?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let a = reservation(
        &handle,
        &effect,
        1,
        1,
        &[(8, SlotMode::UpdateExisting, 4096)],
    );
    let b = reservation(
        &handle,
        &effect,
        2,
        3,
        &[(8, SlotMode::UpdateExisting, 8192)],
    );
    for (id, r) in [(1, a.clone()), (3, b.clone())] {
        let prepared = stage_funding(&mut store, &handle, id, r)?;
        acknowledge(&mut store, prepared, &mut durable)?;
    }
    assert_eq!(
        store.capacity_usage.bytes,
        2 * 16384 + 8192 + store.capacity.metadata_bytes()
    );
    let prepared = terminal(&mut store, &handle, 3, &b, &[])?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let mut txn = store.transaction(&handle)?;
    txn.remove(&handle, &8)?;
    assert!(txn.prepare().is_err());
    let prepared = terminal(&mut store, &handle, 1, &a, &[])?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let mut txn = store.transaction(&handle)?;
    txn.remove(&handle, &8)?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    assert_eq!(store.capacity_usage.records, 2);
    Ok(())
}

#[test]
fn shared_existing_growth_above_ceiling_and_shrink_match_rebuilt_charge() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let mut durable = Durable::new();
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &8, plain("initial"))?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let a = reservation(
        &handle,
        &effect,
        1,
        1,
        &[(8, SlotMode::UpdateExisting, 2048)],
    );
    let b = reservation(
        &handle,
        &effect,
        2,
        3,
        &[(8, SlotMode::UpdateExisting, 4096)],
    );
    for (id, record) in [(1, a), (3, b)] {
        let prepared = stage_funding(&mut store, &handle, id, record)?;
        acknowledge(&mut store, prepared, &mut durable)?;
    }
    let ceiling_charge = store.capacity_usage;
    for text in ["\"".repeat(3000), "small again".to_owned()] {
        let growing = text.len() > 100;
        let mut txn = store.transaction(&handle)?;
        txn.replace(&handle, &8, plain(&text))?;
        let prepared = txn.prepare()?;
        acknowledge(&mut store, prepared, &mut durable)?;
        let actual = store.canonical(&handle, &8)?.ok_or("shared row")?.len();
        assert_eq!(actual > 4096, growing);
        let expected = charged(
            &store.capacity,
            &View {
                registry: &store.registry,
                records: &store.records,
                mutations: None,
            },
            store.live_canonical_bytes,
            store.records.len(),
        )?;
        assert_eq!(
            store.capacity_usage, expected,
            "incremental accounting equals full derivation"
        );
        assert_eq!(
            store.capacity_usage.bytes,
            ceiling_charge.bytes + actual.saturating_sub(4096)
        );
        let (mut restarted, _, _) = fixture(true)?;
        restarted.hydrate(boot(&durable))?;
        assert_eq!(
            restarted.capacity_usage, expected,
            "current shared rows may exceed an older claimant's ceiling"
        );
    }
    assert_eq!(store.capacity_usage, ceiling_charge);
    Ok(())
}

#[test]
fn duplicate_creation_claim_and_below_current_ceiling_refuse_before_reservation() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let mut durable = Durable::new();
    let a = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    let prepared = stage_funding(&mut store, &handle, 1, a)?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let before = store.capacity_usage;
    let b = reservation(&handle, &effect, 2, 3, &[(2, SlotMode::Create, 4096)]);
    assert!(matches!(
        stage_funding(&mut store, &handle, 3, b),
        Err(ActionStateError::TerminalCapacity(
            "exclusive creation key occupied"
        ))
    ));
    assert!(!store.has_prepared());
    assert_eq!(store.capacity_usage, before);
    assert!(store.get(&handle, &3)?.is_none());
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &8, plain(&"x".repeat(5000)))?;
    let prepared = txn.prepare()?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let below = reservation(
        &handle,
        &effect,
        3,
        4,
        &[(8, SlotMode::UpdateExisting, 4096)],
    );
    assert!(matches!(
        stage_funding(&mut store, &handle, 4, below),
        Err(ActionStateError::TerminalCapacity(
            "funding ceiling below current image"
        ))
    ));
    assert!(!store.has_prepared());
    assert!(store.get(&handle, &4)?.is_none());
    Ok(())
}

#[test]
fn unknown_effect_corrupt_metadata_and_missing_backlinks_refuse_atomically() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    let mut durable = Durable::new();
    let prepared = stage_funding(&mut store, &handle, 1, record)?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let (mut unavailable, _, _) = fixture(false)?;
    assert!(matches!(
        unavailable.hydrate(boot(&durable)),
        Err(ActionStateError::TerminalCapacity(
            "closed effect verifier unavailable"
        ))
    ));
    assert!(unavailable.is_empty());
    assert!(unavailable.capacity.is_empty());
    for variant in 0..5 {
        let mut corrupted = boot(&durable);
        let metadata = corrupted
            .iter_mut()
            .find(|r| is_capacity_key(&r.key))
            .ok_or("metadata")?;
        match variant {
            0 => {
                metadata.key = "action-state/capacity/v2/unknown".into();
            }
            1 => {
                metadata.value["unexpected"] = json!(true);
            }
            2 => {
                metadata.namespace = "foreign".into();
            }
            3 => {
                metadata.value["padding"] = json!("x".repeat(MAX_ACTION_STATE_RECORD_BYTES));
            }
            _ => {
                corrupted.retain(|r| is_capacity_key(&r.key));
            }
        }
        let (mut restarted, _, _) = fixture(true)?;
        assert!(restarted.hydrate(corrupted).is_err());
        assert!(restarted.is_empty());
        assert!(restarted.capacity.is_empty());
        assert_eq!(restarted.capacity_usage, Usage::default());
    }
    Ok(())
}

#[test]
fn reservation_shape_canonical_order_and_exact_backlink_are_required() -> TestResult {
    for variant in 0..6 {
        let (mut store, handle, effect) = fixture(true)?;
        let mut record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
        match variant {
            0 => record.targets.reverse(),
            1 => record
                .targets
                .push(record.targets.first().ok_or("target")?.clone()),
            2 => record.targets.first_mut().ok_or("target")?.ceiling = usize::MAX,
            3 => record.targets.first_mut().ok_or("target")?.schema_version = 2,
            4 => {
                record.targets =
                    vec![record.targets.first().ok_or("target")?.clone(); MAX_TARGETS + 1]
            }
            _ => record.targets.retain(|t| t.key != record.sponsor),
        }
        assert!(stage_funding(&mut store, &handle, 1, record).is_err());
        assert!(!store.has_prepared());
        assert!(store.is_empty());
    }
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &1, plain("not a funding operation"))?;
    txn.capacity_changes.funding = Some(record);
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::TerminalCapacity("fixture exact backlink"))
    ));
    Ok(())
}

#[test]
fn terminal_cannot_add_unreserved_keys_or_exceed_full_envelope_ceiling() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 512)]);
    let mut durable = Durable::new();
    let prepared = stage_funding(&mut store, &handle, 1, record.clone())?;
    acknowledge(&mut store, prepared, &mut durable)?;
    assert!(matches!(
        terminal(&mut store, &handle, 1, &record, &[(3, plain("fresh id"))]),
        Err(ActionStateError::TerminalCapacity(
            "unreserved terminal key"
        ))
    ));
    assert!(matches!(
        terminal(
            &mut store,
            &handle,
            1,
            &record,
            &[(2, plain(&"\"".repeat(400)))]
        ),
        Err(ActionStateError::TerminalCapacity(
            "terminal ceiling exceeded"
        ))
    ));
    assert!(!store.has_prepared());
    assert!(!store.capacity.is_empty());
    let prepared = terminal(
        &mut store,
        &handle,
        1,
        &record,
        &[(2, plain("actual receipt"))],
    )?;
    acknowledge(&mut store, prepared, &mut durable)?;
    Ok(())
}

#[test]
fn refusal_drop_and_crash_do_not_release_capacity_early() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    let mut durable = Durable::new();
    let prepared = stage_funding(&mut store, &handle, 1, record.clone())?;
    acknowledge(&mut store, prepared, &mut durable)?;
    let before = store.capacity_usage;
    drop(terminal(&mut store, &handle, 1, &record, &[])?);
    assert!(!store.has_prepared());
    assert_eq!(store.capacity_usage, before);
    let (_, installation) = terminal(&mut store, &handle, 1, &record, &[])?.into_parts();
    store.abort(installation)?;
    assert_eq!(store.capacity_usage, before);
    let (ops, installation) = terminal(&mut store, &handle, 1, &record, &[])?.into_parts();
    persist(ops, &mut durable)?;
    drop(installation);
    assert!(store.reservation_lost());
    assert_eq!(store.capacity_usage, before);
    let (mut restarted, _, _) = fixture(true)?;
    restarted.hydrate(boot(&durable))?;
    assert!(restarted.capacity.is_empty());
    Ok(())
}

#[test]
fn global_and_bundle_limits_count_reserved_keys_and_metadata() -> TestResult {
    let (mut store, handle, effect) = fixture(true)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    // Real accounting is proved above; inject only the aggregate charge for
    // unchanged unrelated families to test the exact 512 MiB boundary cheaply.
    let reserved_charge = 16 * 1024 + 4096 + encode(record.clone())?.len();
    store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - reserved_charge + 1;
    assert!(matches!(
        stage_funding(&mut store, &handle, 1, record.clone()),
        Err(ActionStateError::LiveBytesExceeded { attempted, .. })
            if attempted == MAX_ACTION_STATE_BOOT_BYTES + 1
    ));
    assert!(!store.has_prepared());
    assert!(store.capacity.is_empty());
    store.live_canonical_bytes = 0;
    let mut txn = store.transaction(&handle)?;
    txn.set_limits(ActionTxnLimits::tightened(1, MAX_ACTION_BUNDLE_BYTES))?;
    txn.insert(
        &handle,
        &1,
        Row {
            state: "pending".into(),
            capacity: Some(record.clone()),
            text: String::new(),
        },
    )?;
    txn.capacity_changes.funding = Some(record.clone());
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::OperationLimit {
            attempted: 2,
            max: 1
        })
    ));
    let mut txn = store.transaction(&handle)?;
    txn.insert(
        &handle,
        &1,
        Row {
            state: "pending".into(),
            capacity: Some(record.clone()),
            text: String::new(),
        },
    )?;
    let typed_bytes = txn.canonical_bytes();
    txn.set_limits(ActionTxnLimits::tightened(2, typed_bytes))?;
    txn.capacity_changes.funding = Some(record.clone());
    assert!(
        matches!(txn.prepare(), Err(ActionStateError::BundleBytesExceeded { attempted, max })
        if max == typed_bytes && attempted > max)
    );
    let mut durable = Durable::new();
    let prepared = stage_funding(&mut store, &handle, 1, record)?;
    acknowledge(&mut store, prepared, &mut durable)?;
    store.capacity_usage.bytes = MAX_ACTION_STATE_BOOT_BYTES;
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &9, plain("ordinary"))?;
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::LiveBytesExceeded { .. })
    ));
    store.capacity_usage.bytes = 0; // count boundary is independent of byte bound
    store.capacity_usage.records = MAX_ACTION_STATE_BOOT_RECORDS;
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &9, plain("ordinary"))?;
    assert!(matches!(
        txn.prepare(),
        Err(ActionStateError::LiveRecordLimit { .. })
    ));
    Ok(())
}

#[test]
fn production_has_no_reserve_authority_and_zero_metadata_preserves_legacy_writes() -> TestResult {
    let (mut store, handle, effect) = fixture(false)?;
    let record = reservation(&handle, &effect, 1, 1, &[(2, SlotMode::Create, 4096)]);
    assert!(matches!(
        stage_funding(&mut store, &handle, 1, record),
        Err(ActionStateError::TerminalCapacity(
            "closed effect verifier unavailable"
        ))
    ));
    let mut txn = store.transaction(&handle)?;
    txn.insert(&handle, &1, plain("legacy"))?;
    let prepared = txn.prepare()?;
    assert_eq!(
        prepared.authoritative_replay_writes(),
        typed_replay_writes(prepared.records())
    );
    assert_eq!(
        prepared.canonical_bytes(),
        prepared
            .records()
            .iter()
            .filter_map(|r| r.canonical_post_image())
            .map(CanonicalStateBytes::len)
            .sum::<usize>()
    );
    let mut durable = Durable::new();
    acknowledge(&mut store, prepared, &mut durable)?;
    let (mut restarted, _, _) = fixture(false)?;
    restarted.hydrate(boot(&durable))?;
    assert_eq!(restarted.capacity_usage, store.capacity_usage);
    Ok(())
}
