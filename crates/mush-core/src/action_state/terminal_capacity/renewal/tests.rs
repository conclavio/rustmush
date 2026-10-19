//! Actual coordinator transactions, reservation metadata and cold hydration.
//! Business JSON is inert test data; these tests do not claim an E4 payment.
use super::*;
use crate::action::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, ObjId};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
type Disk = BTreeMap<(SmolStr, SmolStr), Value>;
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
struct Fixture {
    store: ActionStateStore,
    handles: BTreeMap<String, Handle>,
    disk: Disk,
    providers: ActionProviderRegistry,
    actions: [DefinitionRef; 5],
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    invocation: ActionInvocation,
}
fn handle<'a>(f: &'a Fixture, family: &str) -> TestResult<&'a Handle> {
    f.handles.get(family).ok_or_else(|| "fixture handle".into())
}
fn view(f: &Fixture) -> View<'_> {
    View {
        registry: &f.store.registry,
        records: &f.store.records,
        mutations: None,
    }
}
fn fixture() -> TestResult<Fixture> {
    fixture_with_manifest(1)
}
fn fixture_with_manifest(manifest_generation: u64) -> TestResult<Fixture> {
    let (birth, mut handles, disk) = entitlement::tests::renewal_birth_seed()?;
    let market = OwnerId::new("market")?;
    let roll = OwnerId::new("roll")?;
    let owners = BTreeSet::from([
        market.clone(),
        roll.clone(),
        OwnerId::new("economy")?,
        OwnerId::new("tacsim")?,
    ]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    let mut definitions = DefinitionRegistryBuilder::new();
    let manifest = definitions.register(
        OwnerId::new("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_manifest")?,
            DefinitionId::new("combat")?,
            1,
            json!({"generation":manifest_generation}),
        )?,
    )?;
    let mut refs = Vec::new();
    for (owner, id, payload) in [
        ("economy", "initial_filing", json!({"generation":1})),
        (
            "roll",
            "commercial_roots",
            json!({"format":1,"protocol":"commercial_retention_v1"}),
        ),
    ] {
        refs.push(definitions.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")?,
                DefinitionId::new(id)?,
                1,
                payload,
            )?,
        )?);
    }
    let participants: [DefinitionRef; 2] = refs.try_into().map_err(|_| "participant count")?;
    let mut refs = Vec::new();
    for (index, id) in ACTIONS.iter().enumerate() {
        refs.push(
            definitions.register(
                market.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new(*id)?,
                    1,
                    action_payload(index),
                )?
                .with_references(vec![
                    manifest.clone(),
                    participants[0].clone(),
                    participants[1].clone(),
                ]),
            )?,
        );
    }
    let actions: [DefinitionRef; 5] = refs.try_into().map_err(|_| "action count")?;
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in &actions {
        providers.register(market.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    // Reuse exactly the registered first-issuance families and their handles.
    // Then run the ordinary declaration/freeze/hydrate route for new families.
    let mut builder = StateFamilyRegistryBuilder::new();
    builder.entries = birth.registry.entries.clone();
    for (owner, _) in builder.entries.keys() {
        *builder.owner_counts.entry(owner.clone()).or_default() += 1;
    }
    builder.delegations = birth.registry.delegations.clone();
    builder.family_delegations = birth.registry.family_delegations.clone();
    builder.original_purchase = birth.registry.original_purchase.clone();
    builder.entitlement_purchase = birth.registry.entitlement_purchase.clone();
    for (owner, family, bytes) in FAMILIES
        .into_iter()
        .filter(|(_, family, _)| matches!(*family, MARKET | ROLL | HEAD | RECEIPT))
    {
        let schema = |suffix: &str| CanonicalId::new(format!("{family}.{suffix}.v1"));
        let (decode, validate, key_bytes) = if owner == "roll" {
            ("strict_decode", "invariants", 192)
        } else {
            ("decode", "validate", 32)
        };
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(family)?,
            schema("key")?,
            schema("value")?,
            1,
            schema(decode)?,
            schema(validate)?,
            schema("canonical")?,
        )?
        .with_limits(key_bytes, bytes)?;
        let owner = OwnerId::new(owner)?;
        let h = builder.register(
            owner.clone(),
            descriptor,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
        if owner == roll {
            builder.delegate_family(roll.clone(), &h, market.clone())?;
        }
        handles.insert(family.to_owned(), h);
    }
    builder.mind_renewal_capacity(
        &market,
        manifest.clone(),
        participants.clone(),
        actions.clone(),
    )?;
    let mut store =
        ActionStateStore::new(builder.freeze_for_owners_with_readiness(&owners, readiness)?);
    store.hydrate(disk.iter().map(|((ns, key), row)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), row.clone())
    }))?;
    let request = RequestUid::from_bytes([30; 16]);
    let principal = PrincipalId::Object { id: ObjId(7) };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: ObjId(7),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([30; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: market,
                namespace: CanonicalId::new("renewal_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    Ok(Fixture {
        store,
        handles,
        disk,
        providers,
        actions,
        manifest,
        participants,
        invocation,
    })
}
fn install(f: &mut Fixture, prepared: PreparedTxn) -> TestResult {
    let (ops, installation) = prepared.into_parts();
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                f.disk.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                f.disk.remove(&(op.ns, key));
            }
            _ => return Err("unexpected persistence operation".into()),
        }
    }
    f.store.install(installation)?;
    Ok(())
}
fn hydrate(f: &Fixture, disk: &Disk) -> TestResult<ActionStateStore> {
    let mut store = ActionStateStore::new(f.store.registry.clone());
    store.hydrate(disk.iter().map(|((ns, key), row)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), row.clone())
    }))?;
    Ok(store)
}
fn put(
    txn: &mut ActionTxn<'_>,
    handles: &BTreeMap<String, Handle>,
    raw: Key,
    row: Value,
) -> TestResult {
    let handle = handles.get(raw.family.as_str()).ok_or("handle")?;
    let encoded = raw.encoded.to_string();
    if txn.get(handle, &encoded)?.is_some() {
        txn.replace(handle, &encoded, row)?;
    } else {
        txn.insert(handle, &encoded, row)?;
    }
    Ok(())
}
fn draft(f: &mut Fixture, byte: u8, previous: Option<CommitToken>) -> TestResult<PreparedTxn> {
    let operation = CommitToken::from_bytes([byte; 16]);
    let licence = required(&view(f), &licence_key(LICENCE, 1)?)?;
    let predecessor = positive_string(field(&licence, "term")?)?;
    let next = predecessor + 1;
    let term = required(&view(f), &term_key(TERM, 1, predecessor)?)?;
    let economics = required(&view(f), &term_key(ECONOMICS, 1, predecessor)?)?;
    let grant = field(&economics, "grant")?;
    let renewal = field(grant, "renewal")?;
    let binding = json!({"format":1,"licence":"1","holder":7,"charter":"1","line":"1",
        "birth_operation":field(&licence,"operation")?,"predecessor":predecessor.to_string(),"next":next.to_string(),
        "predecessor_operation":field(&term,"operation")?,"predecessor_term":digest("roll.mind.renewal_predecessor.v1",&term)?,
        "predecessor_economics":digest("roll.mind.paid_term_economics.v1",&economics)?,"filed":"1",
        "revision":field(field(&term,"revision")?,"revision")?,"terms":field(&term,"terms")?,
        "term_seconds":100,"grace_seconds":10,"grant":grant,"product_digest":field(&economics,"product_digest")?});
    let at = 1000 + i64::from(byte);
    let offer = json!({"format":1,"basis":{"format":1,"licence":1,"predecessor_generation":predecessor,"next_generation":next,
        "predecessor_operation":field(&term,"operation")?,"prior_quote":CommitToken::from_bytes([3;16]),
        "prior_quote_digest":DefinitionDigest::from_bytes([5;32]),"product_digest":field(&economics,"product_digest")?,
        "template_digest":digest("roll.mind.renewal_template.v1",renewal)?,"grant_economics_digest":digest("roll.mind.grant_economics.v1",grant)?,
        "royalty_chain_digest":field(field(grant,"royalty_chain")?,"digest")?},
        "terms":{"format":1,"lineage":"original","buyer":7,"seller":8,"charter":1,"title_revision":1,"payer":1,
        "instrument":field(renewal,"instrument")?,"price":field(renewal,"price")?,"registry":{"permille":0,"recipient":null},"venue":null,
        "parents":[],"remainder":{"account":2,"owner":{"kind":"object","id":8}},
        "not_before":{"clock":{"kind":"world_ic","generation":1},"at":at},
        "expires_at":{"clock":{"kind":"world_ic","generation":1},"at":at+100}}});
    let attempt = previous.map_or_else(
        || json!({"kind":"automatic"}),
        |p| json!({"kind":"buyer_retry","previous_operation":p}),
    );
    let roll = json!({"format":1,"operation":operation,"action":f.actions[0],"attempt":attempt,"binding":binding,
        "offer_digest":digest("economy.mind_renewal_offer.v1",&offer)?,"contract":u64::from(byte),"consent_hash":vec![0_u8;32],
        "created_at_ic":{"clock":{"kind":"world_ic","generation":1},"at":at},"created_at_utc":at,"state":{"kind":"drafted"}});
    let mut market = roll.clone();
    market["offer"] = offer;
    market["manifest"] = json!(f.manifest);
    market["participants"] = json!(f.participants);
    let temporary = Pair {
        market: market.clone(),
        roll: roll.clone(),
        licence: 1,
        predecessor,
        next,
        phase: "drafted".into(),
    };
    let head = expected_head(&temporary, operation)?;
    let provider = f.providers.get_exact(&f.actions[0])?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, operation);
    let base = txn.read_base();
    put(
        &mut txn,
        &f.handles,
        operation_key(MARKET, operation)?,
        market,
    )?;
    put(&mut txn, &f.handles, operation_key(ROLL, operation)?, roll)?;
    put(&mut txn, &f.handles, licence_key(HEAD, 1)?, head)?;
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), operation, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    ctx.reserve_mind_renewal_completion(&mut txn)?;
    drop(ctx);
    Ok(txn.prepare()?)
}
fn advance(
    f: &mut Fixture,
    operation: CommitToken,
    index: usize,
    byte: u8,
) -> TestResult<PreparedTxn> {
    let commit = CommitToken::from_bytes([byte; 16]);
    let mut pair = pair(&view(f), operation)?;
    let at = 1000 + i64::from(byte);
    let state = match index {
        1 => {
            json!({"kind":"funded","funding":{"format":1,"commit":commit,"action":f.actions[index],
            "contract":field(&pair.market,"contract")?,"hold":u64::from(byte),"transaction":u64::from(byte),"journal_seq":u64::from(byte),
            "consent_hash":field(&pair.market,"consent_hash")?,"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":at},"posted_at_utc":at}})
        }
        2 | 3 => {
            json!({"kind":if index==2{"granted"}else{"refunded"},"funding":field(pair.state()?,"funding")?,
            "terminal":{"format":1,"commit":commit,"action":f.actions[index],"outcome":if index==2{"granted"}else{"refunded"},
            "transaction":u64::from(byte),"journal_seq":u64::from(byte),"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":at},
            "posted_at_utc":at,"issued_at_ic":if index==2{json!({"clock":{"kind":"world_ic","generation":1},"at":at})}else{Value::Null}}})
        }
        4 => json!({"kind":"unfunded_closed","closure":{"commit":commit,"action":f.actions[index],
            "at_ic":{"clock":{"kind":"world_ic","generation":1},"at":at},"posted_at_utc":at,"reason":"declined"}}),
        _ => return Err("advance action".into()),
    };
    pair.phase = text(&state, "kind")?.into();
    pair.market["state"] = state.clone();
    pair.roll["state"] = state.clone();
    let binding = pair.binding()?;
    let receipt = json!({"format":1,"operation":operation,"licence":"1","predecessor":pair.predecessor.to_string(),"next":pair.next.to_string(),
        "contract":field(&pair.market,"contract")?,"consent_hash":field(&pair.market,"consent_hash")?,"state":state});
    let term = json!({"format":1,"licence":"1","generation":pair.next.to_string(),"operation":operation,
        "terms":field(binding,"terms")?,"revision":{"kind":"pin","filed":field(binding,"filed")?,"revision":field(binding,"revision")?},
        "window":{"format":1,"eligibility_start":at,"issued_at":at,"term_end":at+100,"grace_end":at+110}});
    let economics = json!({"format":1,"licence":"1","generation":pair.next.to_string(),"operation":operation,
        "product_digest":field(binding,"product_digest")?,"grant":field(binding,"grant")?});
    let mut licence = current_licence(&view(f), &pair)?;
    licence["term"] = json!(pair.next.to_string());
    let provider = f.providers.get_exact(&f.actions[index])?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, commit);
    let base = txn.read_base();
    put(
        &mut txn,
        &f.handles,
        operation_key(MARKET, operation)?,
        pair.market.clone(),
    )?;
    put(
        &mut txn,
        &f.handles,
        operation_key(ROLL, operation)?,
        pair.roll.clone(),
    )?;
    if index != 1 {
        put(
            &mut txn,
            &f.handles,
            licence_key(HEAD, 1)?,
            expected_head(&pair, operation)?,
        )?;
        put(
            &mut txn,
            &f.handles,
            operation_key(RECEIPT, operation)?,
            receipt,
        )?;
    }
    if index == 2 {
        put(&mut txn, &f.handles, licence_key(LICENCE, 1)?, licence)?;
        put(&mut txn, &f.handles, term_key(TERM, 1, pair.next)?, term)?;
        put(
            &mut txn,
            &f.handles,
            term_key(ECONOMICS, 1, pair.next)?,
            economics,
        )?;
    }
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    ctx.advance_mind_renewal_completion(&mut txn, operation)?;
    drop(ctx);
    Ok(txn.prepare()?)
}
#[test]
fn draft_reserves_seven_targets_fund_retains_exact_metadata_and_grant_hydrates() -> TestResult {
    let mut f = fixture()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let reservation = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(reservation.record.targets.len(), 7);
    assert_eq!(
        reservation
            .record
            .targets
            .iter()
            .map(|r| r.ceiling)
            .sum::<usize>(),
        81920
    );
    let canonical = reservation.canonical.clone();
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    assert_eq!(
        f.store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?
            .canonical,
        canonical
    );
    hydrate(&f, &f.disk)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    assert!(f.store.capacity.is_empty());
    let licence = required(&view(&f), &licence_key(LICENCE, 1)?)?;
    assert_eq!(text(&licence, "term")?, "2");
    assert_eq!(
        token(field(&licence, "operation")?)?,
        CommitToken::from_bytes([6; 16])
    );
    hydrate(&f, &f.disk)?;
    assert!(advance(&mut f, operation, 2, 34).is_err());
    Ok(())
}
#[test]
fn explicit_retry_after_both_closures_keeps_old_history_valid_after_later_grant() -> TestResult {
    for funded in [false, true] {
        let mut f = fixture()?;
        let first = CommitToken::from_bytes([31; 16]);
        let p = draft(&mut f, 31, None)?;
        install(&mut f, p)?;
        if funded {
            let p = advance(&mut f, first, 1, 32)?;
            install(&mut f, p)?;
        }
        let p = advance(&mut f, first, if funded { 3 } else { 4 }, 33)?;
        install(&mut f, p)?;
        let old = required(&view(&f), &operation_key(MARKET, first)?)?;
        assert!(draft(&mut f, 34, None).is_err());
        // The prepare-time rule must match cold recovery: a fresh token is
        // insufficient when its actual preparation time predates closure.
        assert!(draft(&mut f, 20, Some(first)).is_err());
        let p = draft(&mut f, 34, Some(first))?;
        install(&mut f, p)?;
        let second = CommitToken::from_bytes([34; 16]);
        let p = advance(&mut f, second, 1, 35)?;
        install(&mut f, p)?;
        let p = advance(&mut f, second, 2, 36)?;
        install(&mut f, p)?;
        assert_eq!(required(&view(&f), &operation_key(MARKET, first)?)?, old);
        hydrate(&f, &f.disk)?;
        let p = draft(&mut f, 37, None)?;
        install(&mut f, p)?;
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn every_missing_paired_head_receipt_or_grant_row_refuses_cold_recovery() -> TestResult {
    let mut f = fixture()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    for (_, family, _) in FAMILIES {
        let raw = match family {
            HEAD | LICENCE => licence_key(family, 1)?,
            TERM | ECONOMICS => term_key(family, 1, 2)?,
            _ => operation_key(family, operation)?,
        };
        let resolved = raw.resolve(&f.store.registry)?;
        let mut disk = f.disk.clone();
        assert!(
            disk.remove(&(raw.owner.as_str().into(), resolved.persistence_key()))
                .is_some()
        );
        assert!(hydrate(&f, &disk).is_err(), "omitted {family}");
    }
    Ok(())
}
#[test]
fn ordinary_writes_cannot_mutate_a_renewal_generation_or_erase_its_custody() -> TestResult {
    let mut f = fixture()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    assert!(draft(&mut f, 34, Some(operation)).is_err());
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    let raw = term_key(TERM, 1, 2)?;
    let mut row = required(&view(&f), &raw)?;
    row["window"]["term_end"] = json!(9000);
    let term_handle = handle(&f, TERM)?.clone();
    let mut txn = f.store.transaction(&term_handle)?;
    txn.replace(&term_handle, &raw.encoded.to_string(), row)?;
    assert!(txn.prepare().is_err());
    Ok(())
}

#[test]
fn changed_manifest_keeps_pending_recovery_and_only_allows_current_cleanup() -> TestResult {
    for funded in [false, true] {
        let mut old = fixture()?;
        let operation = CommitToken::from_bytes([31; 16]);
        let p = draft(&mut old, 31, None)?;
        install(&mut old, p)?;
        if funded {
            let p = advance(&mut old, operation, 1, 32)?;
            install(&mut old, p)?;
        }
        let mut current = fixture_with_manifest(2)?;
        current.store = hydrate(&current, &old.disk)?;
        current.disk = old.disk.clone();
        assert!(advance(&mut current, operation, if funded { 2 } else { 1 }, 33).is_err());
        let p = advance(&mut current, operation, if funded { 3 } else { 4 }, 34)?;
        install(&mut current, p)?;
        assert!(current.store.capacity.is_empty());
        hydrate(&current, &current.disk)?;
    }
    Ok(())
}

#[test]
fn complete_seven_target_charge_accepts_exact_limit_and_refuses_one_byte_more() -> TestResult {
    let mut measured = fixture()?;
    let old = measured.store.live_canonical_bytes;
    let p = draft(&mut measured, 31, None)?;
    install(&mut measured, p)?;
    let added = measured
        .store
        .capacity_usage
        .bytes
        .checked_sub(old)
        .ok_or("charge")?;
    assert!(added >= 81920);
    // The real seven-target reservation and metadata determine the charge.
    // Only the unrelated aggregate is injected, as in existing capacity tests.
    let mut exact = fixture()?;
    exact.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added;
    let p = draft(&mut exact, 31, None)?;
    install(&mut exact, p)?;
    assert_eq!(
        exact.store.capacity_usage.bytes,
        MAX_ACTION_STATE_BOOT_BYTES
    );
    let mut over = fixture()?;
    over.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added + 1;
    let error = draft(&mut over, 31, None).err().ok_or("byte limit")?;
    assert!(matches!(error.downcast_ref::<ActionStateError>(),
        Some(ActionStateError::LiveBytesExceeded { attempted, .. })
        if *attempted == MAX_ACTION_STATE_BOOT_BYTES + 1));
    Ok(())
}

#[test]
fn retry_links_and_current_head_are_required_in_cold_history() -> TestResult {
    let mut f = fixture()?;
    let first = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let p = advance(&mut f, first, 4, 32)?;
    install(&mut f, p)?;
    let second = CommitToken::from_bytes([33; 16]);
    let p = draft(&mut f, 33, Some(first))?;
    install(&mut f, p)?;
    let p = advance(&mut f, second, 1, 34)?;
    install(&mut f, p)?;
    let p = advance(&mut f, second, 2, 35)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    for previous in [second, CommitToken::from_bytes([99; 16])] {
        let mut disk = f.disk.clone();
        for family in [MARKET, ROLL] {
            let raw = operation_key(family, second)?;
            let key = raw.resolve(&f.store.registry)?;
            let row = disk
                .get_mut(&(raw.owner.as_str().into(), key.persistence_key()))
                .ok_or("operation")?;
            row["value"]["attempt"]["previous_operation"] = json!(previous);
        }
        assert!(hydrate(&f, &disk).is_err());
    }
    let mut disk = f.disk.clone();
    let raw = licence_key(HEAD, 1)?;
    let key = raw.resolve(&f.store.registry)?;
    let head = disk
        .get_mut(&(raw.owner.as_str().into(), key.persistence_key()))
        .ok_or("head")?;
    head["value"]["last_operation"] = json!(first);
    assert!(hydrate(&f, &disk).is_err());
    Ok(())
}
