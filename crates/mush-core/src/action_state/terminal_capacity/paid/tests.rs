#![cfg(test)]

//! Coordinator-only evidence using real typed installation and hydration.
//! Quote/E4 values here are fixture data, not proof of a real funded purchase.
use super::*;
use crate::action::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId};
type TestResult = Result<(), Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
struct Fixture {
    form: &'static str,
    store: ActionStateStore,
    handles: BTreeMap<String, Handle>,
    providers: ActionProviderRegistry,
    actions: [DefinitionRef; 3],
    resale_actions: Option<[DefinitionRef; 5]>,
    resale_registry: Option<DefinitionRef>,
    invocation: ActionInvocation,
    disk: BTreeMap<(SmolStr, SmolStr), Value>,
}
const QUOTE: &str = "market_mind_original_quote";
const DEPLOYMENT: &str = "roll_mind_commerce";
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_versions(0, 0)
}
fn fixture_versions(
    manifest_generation: u64,
    economy_generation: u64,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_product(manifest_generation, economy_generation, 1, "sealed")
}
fn fixture_product(
    manifest_generation: u64,
    economy_generation: u64,
    version: u32,
    form: &'static str,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_product_resale(
        manifest_generation,
        economy_generation,
        version,
        form,
        false,
    )
}
fn fixture_product_resale(
    manifest_generation: u64,
    economy_generation: u64,
    version: u32,
    form: &'static str,
    resale: bool,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_product_resale_registry(
        manifest_generation,
        economy_generation,
        version,
        form,
        resale,
        false,
    )
}
fn fixture_product_resale_registry(
    manifest_generation: u64,
    economy_generation: u64,
    version: u32,
    form: &'static str,
    resale: bool,
    compound: bool,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    let market = OwnerId::new("market")?;
    let roll = OwnerId::new("roll")?;
    let owners = BTreeSet::from([
        market.clone(),
        roll.clone(),
        OwnerId::new("tacsim")?,
        OwnerId::new("economy")?,
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
    let mut participant_refs = Vec::new();
    for (owner, id, payload) in [
        (
            "economy",
            "initial_filing",
            json!({"generation":economy_generation}),
        ),
        (
            "roll",
            "commercial_roots",
            json!({"format":1,"protocol":"commercial_retention_v1"}),
        ),
    ] {
        participant_refs.push(definitions.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")?,
                DefinitionId::new(id)?,
                1,
                payload,
            )?,
        )?);
    }
    let participant_refs: [DefinitionRef; 2] = participant_refs
        .try_into()
        .map_err(|_| "participant count")?;
    let mut actions = Vec::new();
    for (id, transition) in ACTIONS.into_iter().zip(TRANSITIONS) {
        actions.push(
            definitions.register(
                market.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new(id)?,
                    version,
                    action_payload(version, transition)?,
                )?
                .with_references(vec![
                    manifest.clone(),
                    participant_refs[0].clone(),
                    participant_refs[1].clone(),
                ]),
            )?,
        );
    }
    let actions: [DefinitionRef; 3] = actions.try_into().map_err(|_| "actions")?;
    let resale_actions = if resale {
        Some(resale::register_actions(
            &mut definitions,
            &participant_refs[1],
        )?)
    } else {
        None
    };
    let resale_registry = if compound {
        Some(resale::register_registry(
            &mut definitions,
            &participant_refs[1],
        )?)
    } else {
        None
    };
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in actions
        .iter()
        .chain(resale_actions.iter().flat_map(|a| a.iter()))
    {
        providers.register(market.clone(), action.clone(), Arc::new(Provider))?;
    }
    if let Some(action) = &resale_registry {
        providers.register(roll.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    let mut builder = StateFamilyRegistryBuilder::new();
    let mut handles = BTreeMap::new();
    for (owner, family, bytes) in FAMILIES
        .into_iter()
        .chain([("market", QUOTE, 16384), ("roll", DEPLOYMENT, 4096)])
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
        let handle = builder.register(
            owner.clone(),
            descriptor,
            |key: &String| Ok(key.clone()),
            |row: &Value| Ok(row.clone()),
            |_: &Value| Ok(()),
            |row: &Value| Ok(row.clone()),
        )?;
        if owner == roll {
            builder.delegate_family(roll.clone(), &handle, market.clone())?;
        }
        handles.insert(family.to_owned(), handle);
    }
    builder.original_mind_purchase_capacity(
        &market,
        manifest,
        participant_refs.clone(),
        actions.clone(),
    )?;
    if let Some(resale_actions) = &resale_actions {
        resale::register_families(&mut builder, &mut handles, compound)?;
        builder.paid_resale_capacity(
            &market,
            participant_refs,
            resale_actions.clone(),
            resale_registry.clone(),
        )?;
    }
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
                owner: market,
                namespace: CanonicalId::new("purchase")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    let mut f = Fixture {
        form,
        store,
        handles,
        providers,
        actions,
        resale_actions,
        resale_registry,
        invocation,
        disk: BTreeMap::new(),
    };
    let q = quote_for_form(f.store.registry.original_purchase.as_ref().unwrap(), form);
    let mut txn = f.store.transaction(&f.handles[QUOTE])?;
    txn.insert(&f.handles[QUOTE], &quote_id().to_hex(), q)?;
    txn.insert(&f.handles[DEPLOYMENT],&"registry".into(),json!({"format":1,"initialized_by":CommitToken::from_bytes([9;16]),"next_line":1,"next_filed":1,"next_licence":1}))?;
    let prepared = txn.prepare()?;
    install(&mut f, prepared)?;
    Ok(f)
}
fn quote_id() -> CommitToken {
    CommitToken::from_bytes([2; 16])
}
fn op() -> CommitToken {
    CommitToken::from_bytes([5; 16])
}
fn quote() -> Value {
    let hash = DefinitionDigest::from_bytes([3; 32]);
    let revision = json!({"fixture":"exact-revision"});
    let product = json!({"kind":{"kind":"original_personal"},"seats":1,"form":{"kind":"sealed"},"class":"II","revision":{"kind":"pin","filed":"1","revision":revision},"transferable":true,"term_seconds":100,"grace_seconds":10});
    json!({"format":1,"publication":quote_id(),"buyer":7,"subject":{"charter":"1","line":"1","filed":"1","revision":revision,"rating":hash},"product":product,"product_digest":digest("market.original_product.v1",&product).unwrap()})
}
fn quote_for_form(protocol: &Protocol, form: &str) -> Value {
    let mut q = quote();
    let version = protocol.actions[0].version();
    q["format"] = json!(version);
    if version == 2 {
        q["product"]["kind"] = json!({"kind":"original_personal_source_forms"});
    }
    q["product"]["form"] = json!({"kind":form});
    q["product_digest"] = json!(
        digest(
            version_domain(
                version,
                "market.original_product.v1",
                "market.original_product.v2"
            ),
            &q["product"]
        )
        .unwrap()
    );
    q["subject"]["compatibility"] = json!(protocol.manifest.digest());
    q
}
fn rows(action: &DefinitionRef, protocol: &Protocol) -> (Value, Value) {
    rows_for_form(action, protocol, "sealed")
}
fn rows_for_form(action: &DefinitionRef, protocol: &Protocol, form: &str) -> (Value, Value) {
    let q = quote_for_form(protocol, form);
    let refs = json!({"economy":protocol.participants[0],"retention":protocol.participants[1]});
    let funding = json!({"contract":1,"hold":2,"transaction":3,"journal_seq":0,"consent_hash":[1,2,3],"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":100}});
    let purchase = json!({"format":1,"operation":op(),"effect":action,"participants":refs,"quote":q,"quote_digest":digest(version_domain(action.version(),"market.original_quote.v1","market.original_quote.v2"),&q).unwrap(),"licence":"1","term":"1","funding":funding,"phase":{"kind":"funded"},"updated_by":op()});
    let allocation = json!({"format":action.version(),"operation":op(),"participants":refs,"licence":"1","term":"1","quote":quote_id(),"quote_digest":purchase["quote_digest"],"product_digest":q["product_digest"],"holder":7,"charter":"1","line":"1","filed":"1","revision":q["subject"]["revision"],"rating":q["subject"]["rating"],"terms":{"seats":1,"class":"II","form":{"kind":form},"transferable":true},"funding":funding,"phase":{"kind":"funded"},"updated_by":op()});
    (purchase, allocation)
}
fn fund(
    f: &mut Fixture,
    wrong_allocator: bool,
    consume_quote: bool,
) -> Result<PreparedTxn, ActionStateError> {
    fund_variant(f, wrong_allocator, consume_quote, false)
}
fn fund_variant(
    f: &mut Fixture,
    wrong_allocator: bool,
    consume_quote: bool,
    extra_grant: bool,
) -> Result<PreparedTxn, ActionStateError> {
    let provider = f.providers.get_exact(&f.actions[0]).unwrap();
    let (purchase, allocation) = rows_for_form(
        &f.actions[0],
        f.store.registry.original_purchase.as_ref().unwrap(),
        f.form,
    );
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, op());
    let base = txn.read_base();
    txn.insert(&f.handles[PURCHASE], &op().to_hex(), purchase)?;
    txn.insert(&f.handles[ALLOCATION], &op().to_hex(), allocation)?;
    if extra_grant {
        txn.insert(&f.handles[LICENCE], &"licence/0000000000000002".into(),
            json!({"format":1,"id":"2","charter":"1","line":"1","holder":7,"term":"1","operation":op()}))?;
    }
    let mut deployment = txn
        .get(&f.handles[DEPLOYMENT], &"registry".into())?
        .unwrap()
        .clone();
    deployment["next_licence"] = json!(if wrong_allocator { 3 } else { 2 });
    txn.replace(&f.handles[DEPLOYMENT], &"registry".into(), deployment)?;
    if consume_quote {
        txn.remove(&f.handles[QUOTE], &quote_id().to_hex())?;
    }
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), op(), None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    context.reserve_original_mind_purchase_completion(&mut txn)?;
    drop(context);
    txn.prepare()
}
fn finish(f: &mut Fixture, grant: bool, corrupt: bool) -> Result<PreparedTxn, ActionStateError> {
    let commit = CommitToken::from_bytes([6; 16]);
    let provider = f
        .providers
        .get_exact(&f.actions[if grant { 1 } else { 2 }])
        .unwrap();
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, commit);
    let base = txn.read_base();
    let mut purchase = txn
        .get(&f.handles[PURCHASE], &op().to_hex())?
        .unwrap()
        .clone();
    let mut allocation = txn
        .get(&f.handles[ALLOCATION], &op().to_hex())?
        .unwrap()
        .clone();
    let kind = if grant { "granted" } else { "refunded" };
    for row in [&mut purchase, &mut allocation] {
        row["phase"] = json!({"kind":kind,"commit":commit});
        row["updated_by"] = json!(commit);
    }
    let receipt = json!({"format":1,"operation":op(),"commit":commit,"outcome":{"kind":kind},"contract":1,"hold":2,"transaction":4,"journal_seq":1,"licence":"1","term":"1","quote_digest":purchase["quote_digest"],"product_digest":allocation["product_digest"],"issued_at_ic":if grant {json!({"clock":{"kind":"world_ic","generation":1},"at":120})}else{Value::Null}});
    if corrupt {
        purchase["funding"]["transaction"] = json!(999);
        allocation["funding"]["transaction"] = json!(999);
    }
    txn.replace(&f.handles[PURCHASE], &op().to_hex(), purchase)?;
    txn.replace(&f.handles[ALLOCATION], &op().to_hex(), allocation.clone())?;
    txn.insert(&f.handles[RECEIPT], &op().to_hex(), receipt)?;
    if grant {
        txn.insert(&f.handles[LICENCE],&"licence/0000000000000001".into(),json!({"format":1,"id":"1","charter":"1","line":"1","holder":7,"term":"1","operation":op()}))?;
        txn.insert(&f.handles[TERM],&"licence/0000000000000001/term/0000000000000001".into(),json!({"format":1,"licence":"1","generation":"1","window":{"format":1,"eligibility_start":120,"issued_at":120,"term_end":220,"grace_end":230},"terms":allocation["terms"],"revision":quote()["product"]["revision"],"operation":op()}))?;
    }
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    context.finish_original_mind_purchase_completion(&mut txn, op())?;
    drop(context);
    txn.prepare()
}
fn install(f: &mut Fixture, prepared: PreparedTxn) -> Result<(), ActionStateError> {
    let (ops, installation) = prepared.into_parts();
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                f.disk.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                f.disk.remove(&(op.ns, key));
            }
            _ => panic!("unexpected operation"),
        }
    }
    f.store.install(installation)
}
fn reboot(f: &Fixture, omit: Option<&str>) -> Result<ActionStateStore, Box<dyn std::error::Error>> {
    let mut next = ActionStateStore::new(f.store.registry.clone());
    next.hydrate(
        f.disk
            .iter()
            .filter(|((_, key), _)| omit.is_none_or(|omit| !key.contains(omit)))
            .map(|((ns, key), value)| {
                ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
            }),
    )?;
    Ok(next)
}
#[test]
fn exact_five_targets_funded_grant_and_real_hydration() -> TestResult {
    let mut f = fixture()?;
    let prepared = fund(&mut f, false, true)?;
    install(&mut f, prepared)?;
    let row = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation missing")?;
    assert_eq!(row.record.targets.len(), 5);
    assert_eq!(
        row.record
            .targets
            .iter()
            .map(|target| target.ceiling)
            .sum::<usize>(),
        61440
    );
    reboot(&f, None)?;
    let prepared = finish(&mut f, true, false)?;
    install(&mut f, prepared)?;
    assert!(f.store.capacity.is_empty());
    reboot(&f, None)?;
    for family in [PURCHASE, ALLOCATION, LICENCE, TERM, RECEIPT] {
        assert!(reboot(&f, Some(family)).is_err(), "missing {family}");
    }
    Ok(())
}
#[test]
fn refund_keeps_allocation_without_licence_and_releases_capacity() -> TestResult {
    let mut f = fixture()?;
    let prepared = fund(&mut f, false, true)?;
    install(&mut f, prepared)?;
    let prepared = finish(&mut f, false, false)?;
    install(&mut f, prepared)?;
    assert!(f.store.capacity.is_empty());
    assert!(
        f.store
            .get(&f.handles[ALLOCATION], &op().to_hex())?
            .is_some()
    );
    assert!(
        f.store
            .get(&f.handles[LICENCE], &"licence/0000000000000001".into())?
            .is_none()
    );
    reboot(&f, None)?;
    Ok(())
}
#[test]
fn funded_pair_cannot_boot_without_its_reservation_or_either_half() -> TestResult {
    let mut f = fixture()?;
    let prepared = fund(&mut f, false, true)?;
    install(&mut f, prepared)?;
    for omit in ["action-state/capacity/", PURCHASE, ALLOCATION] {
        assert!(reboot(&f, Some(omit)).is_err(), "missing {omit}");
    }
    Ok(())
}
#[test]
fn funding_requires_exact_allocator_and_consumed_quote() -> TestResult {
    for (wrong_allocator, consume_quote) in [(true, true), (false, false)] {
        let mut f = fixture()?;
        assert!(fund(&mut f, wrong_allocator, consume_quote).is_err());
        assert!(f.store.capacity.is_empty());
    }
    Ok(())
}
#[test]
fn ordinary_typed_writer_cannot_create_paid_rights() -> TestResult {
    let mut f = fixture()?;
    let (purchase, allocation) = rows(
        &f.actions[0],
        f.store.registry.original_purchase.as_ref().unwrap(),
    );
    let mut txn = f.store.transaction(&f.handles[PURCHASE])?;
    txn.insert(&f.handles[PURCHASE], &op().to_hex(), purchase)?;
    txn.insert(&f.handles[ALLOCATION], &op().to_hex(), allocation)?;
    assert!(txn.prepare().is_err());
    Ok(())
}
#[test]
fn terminal_cannot_change_funding_even_with_both_matching_postimages() -> TestResult {
    let mut f = fixture()?;
    let prepared = fund(&mut f, false, true)?;
    install(&mut f, prepared)?;
    assert!(finish(&mut f, true, true).is_err());
    assert_eq!(f.store.capacity.records.len(), 1);
    Ok(())
}

#[test]
fn extra_paid_rows_cannot_hide_behind_an_existing_operation_on_boot_or_funding() -> TestResult {
    let mut complete = fixture()?;
    let prepared = fund(&mut complete, false, true)?;
    install(&mut complete, prepared)?;
    let prepared = finish(&mut complete, true, false)?;
    install(&mut complete, prepared)?;
    for phase in ["funded", "granted", "refunded"] {
        let mut f = fixture()?;
        let prepared = fund(&mut f, false, true)?;
        install(&mut f, prepared)?;
        if phase != "funded" {
            let prepared = finish(&mut f, phase == "granted", false)?;
            install(&mut f, prepared)?;
        }
        for family in [LICENCE, TERM] {
            let mut document = complete
                .disk
                .iter()
                .find(|((_, key), _)| key.contains(family))
                .ok_or("grant template")?
                .1
                .clone();
            let encoded = if family == LICENCE {
                "licence/0000000000000002"
            } else {
                "licence/0000000000000002/term/0000000000000001"
            };
            document["key"] = json!(encoded);
            document["value"][if family == LICENCE { "id" } else { "licence" }] = json!("2");
            let path = key("roll", family, encoded.into())?
                .resolve(&f.store.registry)?
                .persistence_key();
            f.disk.insert(("roll".into(), path.clone()), document);
            assert!(
                reboot(&f, None).is_err(),
                "extra {family} escaped reverse {phase} audit"
            );
            f.disk.remove(&("roll".into(), path));
        }
    }
    let mut f = fixture()?;
    assert!(fund_variant(&mut f, false, true, true).is_err());
    assert!(f.store.capacity.is_empty());
    Ok(())
}

#[test]
fn old_manifest_and_participant_policy_can_refund_but_cannot_grant() -> TestResult {
    let mut old = fixture()?;
    let prepared = fund(&mut old, false, true)?;
    install(&mut old, prepared)?;
    for (manifest, economy) in [(1, 0), (0, 1), (1, 1)] {
        let mut current = fixture_versions(manifest, economy)?;
        assert_ne!(old.actions[0], current.actions[0]);
        current.store = ActionStateStore::new(current.store.registry.clone());
        current.disk = old.disk.clone();
        current
            .store
            .hydrate(current.disk.iter().map(|((ns, key), value)| {
                ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
            }))?;
        let reserved = current.store.capacity_usage;
        assert!(
            finish(&mut current, true, false).is_err(),
            "old purchase certified current grant: manifest={manifest}, economy={economy}"
        );
        assert_eq!(current.store.capacity_usage, reserved);
        let prepared = finish(&mut current, false, false)?;
        install(&mut current, prepared)?;
        assert!(current.store.capacity.is_empty());
        reboot(&current, None)?;
    }
    Ok(())
}

#[test]
fn already_granted_licence_survives_changed_manifest_and_participant_policy() -> TestResult {
    let mut old = fixture()?;
    let prepared = fund(&mut old, false, true)?;
    install(&mut old, prepared)?;
    let prepared = finish(&mut old, true, false)?;
    install(&mut old, prepared)?;
    let licence_key = "licence/0000000000000001".to_owned();
    let term_key = "licence/0000000000000001/term/0000000000000001".to_owned();
    let original_licence = old.store.get(&old.handles[LICENCE], &licence_key)?.cloned();
    let original_term = old.store.get(&old.handles[TERM], &term_key)?.cloned();
    assert!(original_licence.is_some());
    assert!(original_term.is_some());
    for (manifest, economy) in [(1, 0), (0, 1), (1, 1)] {
        let mut current = fixture_versions(manifest, economy)?;
        current.store = ActionStateStore::new(current.store.registry.clone());
        current.disk = old.disk.clone();
        current
            .store
            .hydrate(current.disk.iter().map(|((ns, key), value)| {
                ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
            }))?;
        assert!(current.store.capacity.is_empty());
        assert_eq!(
            current
                .store
                .get(&current.handles[LICENCE], &licence_key)?
                .cloned(),
            original_licence
        );
        assert_eq!(
            current
                .store
                .get(&current.handles[TERM], &term_key)?
                .cloned(),
            original_term
        );
        reboot(&current, None)?;
    }
    Ok(())
}

#[test]
fn crossed_historical_refs_or_manifest_cannot_boot_as_a_cleanup_obligation() -> TestResult {
    let mut original = fixture()?;
    let prepared = fund(&mut original, false, true)?;
    install(&mut original, prepared)?;
    for corrupt in ["manifest", "participant", "effect"] {
        let mut disk = original.disk.clone();
        let record = disk
            .iter_mut()
            .find(|((_, key), _)| key.contains(PURCHASE))
            .ok_or("purchase")?
            .1;
        match corrupt {
            "manifest" => {
                record["value"]["quote"]["subject"]["compatibility"] =
                    json!(DefinitionDigest::from_bytes([93; 32]))
            }
            "participant" => {
                record["value"]["participants"]["economy"]["digest"] =
                    json!(DefinitionDigest::from_bytes([94; 32]))
            }
            _ => {
                record["value"]["effect"]["digest"] = json!(DefinitionDigest::from_bytes([95; 32]))
            }
        }
        let current = fixture_versions(1, 1)?;
        let mut store = ActionStateStore::new(current.store.registry.clone());
        assert!(
            store
                .hydrate(
                    disk.into_iter()
                        .map(|((ns, key), value)| ActionStateBootRecord::new(ns, key, value))
                )
                .is_err(),
            "crossed {corrupt}"
        );
    }
    Ok(())
}

#[test]
fn coherent_crossed_history_with_repaired_checksums_still_refuses_the_old_effect() -> TestResult {
    let mut original = fixture()?;
    let prepared = fund(&mut original, false, true)?;
    install(&mut original, prepared)?;
    let purchase_key = (
        SmolStr::new("market"),
        operation_key(PURCHASE, op())?
            .resolve(&original.store.registry)?
            .persistence_key(),
    );
    let allocation_key = (
        SmolStr::new("roll"),
        operation_key(ALLOCATION, op())?
            .resolve(&original.store.registry)?
            .persistence_key(),
    );
    let capacity_key = original
        .disk
        .keys()
        .find(|(_, key)| key.starts_with(PREFIX))
        .cloned()
        .ok_or("capacity metadata")?;
    for corrupt in ["manifest", "participant"] {
        let current = fixture_versions(1, 1)?;
        let protocol = current
            .store
            .registry
            .original_purchase
            .as_ref()
            .ok_or("protocol")?;
        let mut disk = original.disk.clone();
        let mut purchase = disk.get(&purchase_key).ok_or("purchase")?["value"].clone();
        let mut allocation = disk.get(&allocation_key).ok_or("allocation")?["value"].clone();
        let old_effect = purchase["effect"].clone();
        if corrupt == "manifest" {
            purchase["quote"]["subject"]["compatibility"] = json!(protocol.manifest.digest());
        } else {
            purchase["participants"]["economy"] = json!(protocol.participants[0]);
        }
        // Repair every affected paired field and both canonical checksums.
        // The refusal must therefore come from the exact old funding effect,
        // not a stale quote hash, inconsistent pair or capacity fingerprint.
        purchase["quote_digest"] = json!(digest("market.original_quote.v1", &purchase["quote"])?);
        allocation["quote_digest"] = purchase["quote_digest"].clone();
        allocation["participants"] = purchase["participants"].clone();
        assert_eq!(purchase["effect"], old_effect);
        assert_eq!(purchase["participants"], allocation["participants"]);
        assert_eq!(purchase["quote_digest"], allocation["quote_digest"]);
        assert_ne!(
            json!(historical_admission(protocol, &purchase)?),
            old_effect
        );
        let paired = Pair {
            purchase: purchase.clone(),
            allocation: allocation.clone(),
            phase: "funded".into(),
            licence: 1,
        };
        disk.get_mut(&purchase_key).ok_or("purchase")?["value"] = purchase;
        disk.get_mut(&allocation_key).ok_or("allocation")?["value"] = allocation;
        let metadata = disk.get_mut(&capacity_key).ok_or("capacity metadata")?;
        assert_eq!(metadata["effect"], old_effect);
        metadata["protocol_fingerprint"] = json!(fingerprint(&paired)?);
        let mut store = ActionStateStore::new(current.store.registry.clone());
        assert!(
            store
                .hydrate(
                    disk.into_iter()
                        .map(|((ns, key), value)| { ActionStateBootRecord::new(ns, key, value) })
                )
                .is_err(),
            "coherent crossed {corrupt} accepted the old effect"
        );
    }
    Ok(())
}

#[test]
fn source_forms_require_v2_pair_and_survive_exact_five_target_hydration() -> TestResult {
    for form in ["sealed", "open", "open_resale"] {
        for granted in [false, true] {
            let mut f = fixture_product(0, 0, 2, form)?;
            let prepared = fund(&mut f, false, true)?;
            install(&mut f, prepared)?;
            assert_eq!(
                f.store
                    .capacity
                    .records
                    .values()
                    .next()
                    .ok_or("reservation")?
                    .record
                    .targets
                    .len(),
                5
            );
            reboot(&f, None)?;
            let prepared = finish(&mut f, granted, false)?;
            install(&mut f, prepared)?;
            reboot(&f, None)?;
            assert!(f.store.capacity.is_empty());
            if granted {
                let term = f
                    .store
                    .get(
                        &f.handles[TERM],
                        &"licence/0000000000000001/term/0000000000000001".into(),
                    )?
                    .ok_or("term")?;
                assert_eq!(term.pointer("/terms/form/kind"), Some(&json!(form)));
            }
        }
    }
    for form in ["open", "open_resale", "unrestricted"] {
        let mut f = fixture_product(0, 0, 1, form)?;
        assert!(fund(&mut f, false, true).is_err());
        assert!(f.store.capacity.is_empty());
    }
    Ok(())
}
#[test]
fn exact_old_v1_funding_under_v2_can_only_refund() -> TestResult {
    let mut old = fixture()?;
    let prepared = fund(&mut old, false, true)?;
    install(&mut old, prepared)?;
    let before = old.disk.clone();
    let mut current = fixture_product(0, 0, 2, "open")?;
    current.disk = old.disk.clone();
    current.store = ActionStateStore::new(current.store.registry.clone());
    current
        .store
        .hydrate(current.disk.iter().map(|((ns, key), value)| {
            ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
        }))?;
    assert!(finish(&mut current, true, false).is_err());
    assert_eq!(current.disk, before);
    let prepared = finish(&mut current, false, false)?;
    install(&mut current, prepared)?;
    reboot(&current, None)?;
    let allocation = current
        .store
        .get(&current.handles[ALLOCATION], &op().to_hex())?
        .ok_or("allocation")?;
    assert_eq!(allocation.get("format"), Some(&json!(1)));
    assert_eq!(
        allocation.pointer("/terms/form/kind"),
        Some(&json!("sealed"))
    );
    Ok(())
}

#[path = "../paid_resale/tests.rs"]
mod resale;
