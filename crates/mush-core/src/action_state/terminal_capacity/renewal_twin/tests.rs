//! Actual coordinator transactions, reservation metadata and cold hydration, written once
//! and run for both issuers (doc 46 CT.3). Business JSON is inert test data; these tests do
//! not claim an E4 payment.
//!
//! A body here is generic over [`Rig`], the test-side twin of [`Issuer`]: what the two
//! issuers' fixtures differ by (the birth seed, the registry the birth leaves, the
//! issuer's own fields of a Draft, the two `PrepareCtx` calls). [`instantiate!`] names
//! each body as a `#[test]` in `renewal::tests` and in `house_renewal::tests`, so a test
//! keeps the path it has always had and runs once an issuer.
#![cfg(test)]
use super::*;
use crate::action::*;
use crate::resolution::PrepareCtx;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, ObjId};

pub(in crate::action_state::terminal_capacity) mod ct0_walk;
pub(in crate::action_state::terminal_capacity) mod service;

pub(in crate::action_state::terminal_capacity) type TestResult<T = ()> =
    Result<T, Box<dyn std::error::Error>>;
pub(in crate::action_state::terminal_capacity) type Handle = StateFamilyHandle<String, Value>;
/// A birth seed: the action store, its family handles by name, and the
/// disk image a reboot reads.
pub(in crate::action_state::terminal_capacity) type Seed = (
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
);
pub(in crate::action_state::terminal_capacity) type Disk = BTreeMap<(SmolStr, SmolStr), Value>;

/// What differs between the two issuers' test rigs.
pub(in crate::action_state::terminal_capacity) trait Rig:
    Issuer
{
    /// The family the licence's birth allocation is read from.
    const BIRTH: &'static str;
    /// The word a missing birth allocation is reported with.
    const BIRTH_MISSING: &'static str;
    /// One acknowledged first generation, installed by the birth's own coordinator rig.
    fn birth_seed() -> TestResult<Seed>;
    /// The same, born under a Follow revision policy.
    fn follow_birth_seed() -> TestResult<Seed>;
    /// The sentence a declaration without this issuer's closed birth audits is refused with.
    const BIRTH_AUDITS_REFUSED: &'static str;
    /// `registry` once for each closed purchase audit this issuer's declaration needs, with
    /// that one audit taken away.
    fn without_birth_audits(registry: &StateFamilyRegistry) -> Vec<StateFamilyRegistry>;
    /// Carry the birth seed's delegations and closed purchase audits into the renewal
    /// fixture's registry. `builder.entries` already holds the birth's families.
    fn inherit(builder: &mut StateFamilyRegistryBuilder, birth: &StateFamilyRegistry)
    -> TestResult;
    /// Declare this issuer's renewal protocol, with the service close when one is given.
    fn declare(
        builder: &mut StateFamilyRegistryBuilder,
        market: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service: Option<DefinitionRef>,
    ) -> Result<(), ActionStateError>;
    /// The issuer's own fields of a Draft: its part of the binding, of the offer's basis
    /// and of the offer's terms.
    fn disclose(
        birth: &Value,
        grant: &Value,
        binding: &mut Value,
        basis: &mut Value,
        terms: &mut Value,
    ) -> TestResult;
    /// The Draft's reservation, through the module-facing call of this issuer.
    fn reserve(ctx: &mut PrepareCtx<'_>, txn: &mut ActionTxn<'_>) -> Result<(), ActionStateError>;
    /// A later transition's advance, through the module-facing call of this issuer.
    fn advance(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), ActionStateError>;
}

struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
pub(in crate::action_state::terminal_capacity) struct Fixture<I> {
    pub(in crate::action_state::terminal_capacity) store: ActionStateStore,
    pub(in crate::action_state::terminal_capacity) handles: BTreeMap<String, Handle>,
    pub(in crate::action_state::terminal_capacity) disk: Disk,
    pub(in crate::action_state::terminal_capacity) providers: ActionProviderRegistry,
    pub(in crate::action_state::terminal_capacity) actions: [DefinitionRef; 5],
    pub(in crate::action_state::terminal_capacity) service: Option<DefinitionRef>,
    pub(in crate::action_state::terminal_capacity) manifest: DefinitionRef,
    pub(in crate::action_state::terminal_capacity) participants: [DefinitionRef; 2],
    pub(in crate::action_state::terminal_capacity) invocation: ActionInvocation,
    issuer: PhantomData<I>,
}
fn handle<'a, I>(f: &'a Fixture<I>, family: &str) -> TestResult<&'a Handle> {
    f.handles.get(family).ok_or_else(|| "fixture handle".into())
}
pub(in crate::action_state::terminal_capacity) fn view<I>(f: &Fixture<I>) -> View<'_> {
    View {
        registry: &f.store.registry,
        records: &f.store.records,
        mutations: None,
    }
}
pub(in crate::action_state::terminal_capacity) fn fixture<I: Rig>() -> TestResult<Fixture<I>> {
    fixture_with_manifest(1)
}
pub(in crate::action_state::terminal_capacity) fn fixture_with_manifest<I: Rig>(
    manifest_generation: u64,
) -> TestResult<Fixture<I>> {
    fixture_config(manifest_generation, false, false)
}
pub(in crate::action_state::terminal_capacity) fn fixture_config<I: Rig>(
    manifest_generation: u64,
    service_provider: bool,
    service_protocol: bool,
) -> TestResult<Fixture<I>> {
    fixture_from_seed(
        manifest_generation,
        service_provider,
        service_protocol,
        I::birth_seed()?,
    )
}
pub(in crate::action_state::terminal_capacity) fn fixture_from_seed<I: Rig>(
    manifest_generation: u64,
    service_provider: bool,
    service_protocol: bool,
    (birth, mut handles, disk): Seed,
) -> TestResult<Fixture<I>> {
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
    for (index, id) in I::ACTIONS.iter().enumerate() {
        refs.push(
            definitions.register(
                market.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new(*id)?,
                    1,
                    action_payload::<I>(index),
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
    let service = if service_provider {
        Some(
            definitions.register(
                market.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new(I::SERVICE_ACTION)?,
                    1,
                    action_payload::<I>(5),
                )?
                .with_references(vec![
                    manifest.clone(),
                    participants[0].clone(),
                    participants[1].clone(),
                ]),
            )?,
        )
    } else {
        None
    };
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in actions.iter().chain(service.iter()) {
        providers.register(market.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    // Reuse exactly the registered first-issuance families and their handles.
    // Then run the ordinary declaration/freeze/hydrate route for new families.
    let mut builder = StateFamilyRegistryBuilder::new();
    builder.entries = birth.registry.entries.clone();
    I::inherit(&mut builder, &birth.registry)?;
    for (owner, family, bytes) in families::<I>()
        .into_iter()
        .filter(|(_, family, _)| [I::MARKET, I::ROLL, I::HEAD, I::RECEIPT].contains(family))
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
        if let Some(existing) = builder
            .entries
            .get(&(owner.clone(), descriptor.id().clone()))
        {
            if existing.descriptor().canonical_value() != descriptor.canonical_value() {
                return Err("retained renewal family descriptor differs".into());
            }
            continue;
        }
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
    let service_close = if service_protocol {
        Some(service.clone().ok_or("service ref")?)
    } else {
        None
    };
    I::declare(
        &mut builder,
        &market,
        manifest.clone(),
        participants.clone(),
        actions.clone(),
        service_close,
    )?;
    let mut store =
        ActionStateStore::new(builder.freeze_for_owners_with_readiness(&owners, readiness)?);
    store.hydrate(disk.iter().map(|((ns, key), row)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), row.clone())
    }))?;
    let request = RequestUid::from_bytes([30; 16]);
    let pointer = store
        .get(
            handles.get(LICENCE).ok_or("licence handle")?,
            &"licence/0000000000000001".to_owned(),
        )?
        .ok_or("current licence")?;
    let actor = ObjId(u32::try_from(number(pointer, "holder")?)?);
    let principal = PrincipalId::Object { id: actor };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
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
        service,
        manifest,
        participants,
        invocation,
        issuer: PhantomData,
    })
}
pub(in crate::action_state::terminal_capacity) fn install<I>(
    f: &mut Fixture<I>,
    prepared: PreparedTxn,
) -> TestResult {
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
pub(in crate::action_state::terminal_capacity) fn hydrate<I>(
    f: &Fixture<I>,
    disk: &Disk,
) -> TestResult<ActionStateStore> {
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
pub(in crate::action_state::terminal_capacity) fn draft<I: Rig>(
    f: &mut Fixture<I>,
    byte: u8,
    previous: Option<CommitToken>,
) -> TestResult<PreparedTxn> {
    let operation = CommitToken::from_bytes([byte; 16]);
    let licence = required(&view(f), &licence_key(LICENCE, 1)?)?;
    let birth = optional_family_value(&view(f), I::BIRTH, token(field(&licence, "operation")?)?)?
        .ok_or(I::BIRTH_MISSING)?;
    let predecessor = positive_string(field(&licence, "term")?)?;
    let next = predecessor + 1;
    let term = required(&view(f), &term_key(TERM, 1, predecessor)?)?;
    let economics = required(&view(f), &term_key(I::ECONOMICS, 1, predecessor)?)?;
    let grant = field(&economics, "grant")?;
    let renewal = field(grant, "renewal")?;
    let holder = field(&licence, "holder")?.clone();
    let holder_basis = if number(&licence, "format")? == 2 {
        let ownership = field(&licence, "ownership")?;
        let generation = number(ownership, "generation")?;
        let h = required(
            &view(f),
            &key(
                "roll_mind_paid_holders",
                format!("licence/{:016x}/{generation:016x}", 1),
            )?,
        )?;
        Some(json!({"acceptance":field(&h,"acceptance")?,"ownership":ownership}))
    } else {
        None
    };
    let mut binding = json!({"format":1,"licence":"1","holder":holder,"charter":"1","line":"1",
        "birth_operation":field(&licence,"operation")?,"predecessor":predecessor.to_string(),"next":next.to_string(),
        "predecessor_operation":field(&term,"operation")?,"predecessor_term":digest(I::PREDECESSOR_DOMAIN,&term)?,
        "predecessor_economics":digest(I::economics_domain(&economics)?,&economics)?,"filed":"1",
        "revision":field(&birth,"revision")?,"terms":field(&term,"terms")?,
        "term_seconds":100,"grace_seconds":10,"grant":grant,"product_digest":field(&economics,"product_digest")?});
    let at = 1000 + i64::from(byte);
    let mut basis = json!({"format":1,"licence":1,"predecessor_generation":predecessor,"next_generation":next,
        "predecessor_operation":field(&term,"operation")?,"product_digest":field(&economics,"product_digest")?,
        "template_digest":digest(I::TEMPLATE_DOMAIN,renewal)?,"grant_economics_digest":digest(I::grant_domain(grant)?,grant)?});
    let mut terms = json!({"format":1,"buyer":holder,"charter":1,"title_revision":1,"payer":1,
        "instrument":field(renewal,"instrument")?,"price":field(renewal,"price")?,"registry":{"permille":0,"recipient":null},"venue":null,
        "not_before":{"clock":{"kind":"world_ic","generation":1},"at":at},
        "expires_at":{"clock":{"kind":"world_ic","generation":1},"at":at+100}});
    I::disclose(&birth, grant, &mut binding, &mut basis, &mut terms)?;
    if let Some(basis) = &holder_basis {
        binding["format"] = json!(2);
        binding["holder_basis"] = basis.clone();
    }
    if text(field(renewal, "revision")?, "kind")? == "follow" {
        binding["format"] = json!(if holder_basis.is_some() { 4 } else { 3 });
        binding["revision_policy"] = field(renewal, "revision")?.clone();
    }
    let offer = json!({"format":1,"basis":basis,"terms":terms});
    let mut attempt = previous.map_or_else(
        || json!({"kind":"automatic"}),
        |p| json!({"kind":"buyer_retry","previous_operation":p}),
    );
    if let Some(basis) = holder_basis {
        attempt["kind"] = json!(if previous.is_some() {
            "holder_retry"
        } else {
            "holder_automatic"
        });
        attempt["basis"] = basis;
    }
    let roll = json!({"format":1,"operation":operation,"action":f.actions[0],"attempt":attempt,"binding":binding,
        "offer_digest":digest(I::OFFER_DOMAIN,&offer)?,"contract":u64::from(byte),"consent_hash":vec![0_u8;32],
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
        operation_key(I::MARKET, operation)?,
        market,
    )?;
    put(
        &mut txn,
        &f.handles,
        operation_key(I::ROLL, operation)?,
        roll,
    )?;
    put(&mut txn, &f.handles, licence_key(I::HEAD, 1)?, head)?;
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), operation, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    I::reserve(&mut ctx, &mut txn)?;
    drop(ctx);
    Ok(txn.prepare()?)
}
pub(in crate::action_state::terminal_capacity) fn advance<I: Rig>(
    f: &mut Fixture<I>,
    operation: CommitToken,
    index: usize,
    byte: u8,
) -> TestResult<PreparedTxn> {
    advance_with_service(f, operation, index, byte, None)
}
pub(in crate::action_state::terminal_capacity) fn advance_with_service<I: Rig>(
    f: &mut Fixture<I>,
    operation: CommitToken,
    index: usize,
    byte: u8,
    service_state: Option<Value>,
) -> TestResult<PreparedTxn> {
    advance_full(f, operation, index, byte, service_state, None)
}
/// `advance_with_service` with `rows`' extra writes (17g C2f: a renewal
/// writes no owner-index row, so the default is none).
pub(in crate::action_state::terminal_capacity) fn advance_full<I: Rig>(
    f: &mut Fixture<I>,
    operation: CommitToken,
    index: usize,
    byte: u8,
    service_state: Option<Value>,
    rows: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let commit = CommitToken::from_bytes([byte; 16]);
    let mut pair = pair::<I>(&view(f), operation)?;
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
        // CT.0: a walk may pass its own closure (the `expired` reason).
        4 => service_state.unwrap_or_else(|| json!({"kind":"unfunded_closed","closure":{"commit":commit,"action":f.actions[index],
            "at_ic":{"clock":{"kind":"world_ic","generation":1},"at":at},"posted_at_utc":at,"reason":"declined"}})),
        5 => service_state.ok_or("service closure state")?,
        _ => return Err("advance action".into()),
    };
    pair.phase = text(&state, "kind")?.into();
    pair.market["state"] = state.clone();
    pair.roll["state"] = state.clone();
    let binding = pair.binding()?;
    let receipt = json!({"format":1,"operation":operation,"licence":"1","predecessor":pair.predecessor.to_string(),"next":pair.next.to_string(),
        "contract":field(&pair.market,"contract")?,"consent_hash":field(&pair.market,"consent_hash")?,"state":state});
    let term = json!({"format":1,"licence":"1","generation":pair.next.to_string(),"operation":operation,
        "terms":field(binding,"terms")?,"revision":entitlement::binding_revision(binding)?,
        "window":{"format":1,"eligibility_start":at,"issued_at":at,"term_end":at+100,"grace_end":at+110}});
    let economics = json!({"format":I::grant_version(field(binding,"grant")?)?,"licence":"1","generation":pair.next.to_string(),"operation":operation,
        "product_digest":field(binding,"product_digest")?,"grant":field(binding,"grant")?});
    let mut licence = current_licence(&view(f), &pair)?;
    licence["term"] = json!(pair.next.to_string());
    let action = if index == 5 {
        f.service.as_ref().ok_or("service provider")?
    } else {
        &f.actions[index]
    };
    let provider = f.providers.get_exact(action)?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, commit);
    let base = txn.read_base();
    put(
        &mut txn,
        &f.handles,
        operation_key(I::MARKET, operation)?,
        pair.market.clone(),
    )?;
    put(
        &mut txn,
        &f.handles,
        operation_key(I::ROLL, operation)?,
        pair.roll.clone(),
    )?;
    if index != 1 {
        put(
            &mut txn,
            &f.handles,
            licence_key(I::HEAD, 1)?,
            expected_head(&pair, operation)?,
        )?;
        put(
            &mut txn,
            &f.handles,
            operation_key(I::RECEIPT, operation)?,
            receipt,
        )?;
    }
    if index == 2 {
        put(&mut txn, &f.handles, licence_key(LICENCE, 1)?, licence)?;
        put(&mut txn, &f.handles, term_key(TERM, 1, pair.next)?, term)?;
        put(
            &mut txn,
            &f.handles,
            term_key(I::ECONOMICS, 1, pair.next)?,
            economics,
        )?;
    }
    if let Some(write) = rows {
        write(&mut txn, &f.handles)?;
    }
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    I::advance(&mut ctx, &mut txn, operation)?;
    drop(ctx);
    Ok(txn.prepare()?)
}
pub(in crate::action_state::terminal_capacity) fn draft_reserves_seven_targets_fund_retains_exact_metadata_and_grant_hydrates<
    I: Rig,
>() -> TestResult {
    let mut f = fixture::<I>()?;
    // The House protocol never stands alone: the personal one is declared beside it.
    assert!(Personal::protocol(&f.store.registry).is_some());
    assert!(I::protocol(&f.store.registry).is_some());
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
pub(in crate::action_state::terminal_capacity) fn explicit_retry_after_both_closures_keeps_old_history_valid_after_later_grant<
    I: Rig,
>() -> TestResult {
    for funded in [false, true] {
        let mut f = fixture::<I>()?;
        let first = CommitToken::from_bytes([31; 16]);
        let p = draft(&mut f, 31, None)?;
        install(&mut f, p)?;
        if funded {
            let p = advance(&mut f, first, 1, 32)?;
            install(&mut f, p)?;
        }
        let p = advance(&mut f, first, if funded { 3 } else { 4 }, 33)?;
        install(&mut f, p)?;
        let old = required(&view(&f), &operation_key(I::MARKET, first)?)?;
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
        assert_eq!(required(&view(&f), &operation_key(I::MARKET, first)?)?, old);
        hydrate(&f, &f.disk)?;
        let p = draft(&mut f, 37, None)?;
        install(&mut f, p)?;
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn every_missing_paired_head_receipt_or_grant_row_refuses_cold_recovery<
    I: Rig,
>() -> TestResult {
    let mut f = fixture::<I>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    for (_, family, _) in families::<I>() {
        let raw = if [I::HEAD, LICENCE].contains(&family) {
            licence_key(family, 1)?
        } else if [TERM, I::ECONOMICS].contains(&family) {
            term_key(family, 1, 2)?
        } else {
            operation_key(family, operation)?
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
pub(in crate::action_state::terminal_capacity) fn ordinary_writes_cannot_mutate_a_renewal_generation_or_erase_its_custody<
    I: Rig,
>() -> TestResult {
    let mut f = fixture::<I>()?;
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

pub(in crate::action_state::terminal_capacity) fn changed_manifest_keeps_pending_recovery_and_only_allows_current_cleanup<
    I: Rig,
>() -> TestResult {
    for funded in [false, true] {
        let mut old = fixture::<I>()?;
        let operation = CommitToken::from_bytes([31; 16]);
        let p = draft(&mut old, 31, None)?;
        install(&mut old, p)?;
        if funded {
            let p = advance(&mut old, operation, 1, 32)?;
            install(&mut old, p)?;
        }
        let mut current = fixture_with_manifest::<I>(2)?;
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

pub(in crate::action_state::terminal_capacity) fn complete_seven_target_charge_accepts_exact_limit_and_refuses_one_byte_more<
    I: Rig,
>() -> TestResult {
    let mut measured = fixture::<I>()?;
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
    let mut exact = fixture::<I>()?;
    exact.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added;
    let p = draft(&mut exact, 31, None)?;
    install(&mut exact, p)?;
    assert_eq!(
        exact.store.capacity_usage.bytes,
        MAX_ACTION_STATE_BOOT_BYTES
    );
    let mut over = fixture::<I>()?;
    over.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added + 1;
    let error = draft(&mut over, 31, None).err().ok_or("byte limit")?;
    assert!(matches!(error.downcast_ref::<ActionStateError>(),
        Some(ActionStateError::LiveBytesExceeded { attempted, .. })
        if *attempted == MAX_ACTION_STATE_BOOT_BYTES + 1));
    Ok(())
}

pub(in crate::action_state::terminal_capacity) fn retry_links_and_current_head_are_required_in_cold_history<
    I: Rig,
>() -> TestResult {
    let mut f = fixture::<I>()?;
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
        for family in [I::MARKET, I::ROLL] {
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
    let raw = licence_key(I::HEAD, 1)?;
    let key = raw.resolve(&f.store.registry)?;
    let head = disk
        .get_mut(&(raw.owner.as_str().into(), key.persistence_key()))
        .ok_or("head")?;
    head["value"]["last_operation"] = json!(first);
    assert!(hydrate(&f, &disk).is_err());
    Ok(())
}

// Reuse only acknowledged coordinator transitions; native rows remain inert
// fixture data and do not stand in for real Economy payment authority.
pub(in crate::action_state::terminal_capacity) fn resale_term_seed<I: Rig>(
    grant: bool,
) -> TestResult<Seed> {
    let mut f = fixture::<I>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    if grant {
        let p = advance(&mut f, operation, 1, 32)?;
        install(&mut f, p)?;
        let p = advance(&mut f, operation, 2, 33)?;
        install(&mut f, p)?;
    }
    Ok((f.store, f.handles, f.disk))
}

pub(in crate::action_state::terminal_capacity) fn closed_holder_seed<I: Rig>(
    funded: bool,
) -> TestResult<Seed> {
    let mut f = fixture::<I>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    if funded {
        let p = advance(&mut f, operation, 1, 32)?;
        install(&mut f, p)?;
    }
    let p = advance(&mut f, operation, if funded { 3 } else { 4 }, 33)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok((f.store, f.handles, f.disk))
}
pub(in crate::action_state::terminal_capacity) fn continue_holder_seed<I: Rig>(
    seed: Seed,
    byte: u8,
    previous: Option<CommitToken>,
    terminal: usize,
) -> TestResult<Seed> {
    let mut f = fixture_from_seed::<I>(1, false, false, seed)?;
    let before = required(&view(&f), &licence_key(LICENCE, 1)?)?;
    let op = CommitToken::from_bytes([byte; 16]);
    let p = draft(&mut f, byte, previous)?;
    install(&mut f, p)?;
    let reserved = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?
        .canonical
        .clone();
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
        7
    );
    assert_eq!(
        f.store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?
            .record
            .targets
            .iter()
            .map(|t| t.ceiling)
            .sum::<usize>(),
        81920
    );
    hydrate(&f, &f.disk)?;
    assert!(
        draft(&mut f, byte + 10, None).is_err(),
        "duplicate automatic while pending"
    );
    if terminal == 0 {
        return Ok((f.store, f.handles, f.disk));
    }
    if terminal != 4 {
        let p = advance(&mut f, op, 1, byte + 1)?;
        install(&mut f, p)?;
        assert_eq!(
            f.store
                .capacity
                .records
                .values()
                .next()
                .ok_or("reservation")?
                .canonical,
            reserved
        );
    }
    let p = advance(&mut f, op, terminal, byte + 2)?;
    install(&mut f, p)?;
    let after = required(&view(&f), &licence_key(LICENCE, 1)?)?;
    for name in [
        "format",
        "holder",
        "ownership",
        "operation",
        "charter",
        "line",
    ] {
        assert!(
            before.get(name) == after.get(name),
            "renewal changed immutable holder identity"
        );
    }
    assert!(f.store.capacity.is_empty());
    hydrate(&f, &f.disk)?;
    if terminal != 2 {
        assert!(
            draft(&mut f, byte + 10, None).is_err(),
            "duplicate automatic after refusal"
        );
    }
    // Both owner records must carry the same strict format and attempt basis;
    // corrupt both so this reaches the historical join rather than pair equality.
    for case in 0..7 {
        let mut disk = f.disk.clone();
        for family in [I::MARKET, I::ROLL] {
            let raw = operation_key(family, op)?;
            let resolved = raw.resolve(&f.store.registry)?;
            let row = disk
                .get_mut(&(raw.owner.as_str().into(), resolved.persistence_key()))
                .ok_or("operation")?;
            match case {
                0 => {
                    row["value"]["binding"]["holder_basis"] = Value::Null;
                }
                1 => {
                    row["value"]["binding"]["format"] = json!(1);
                }
                2 => {
                    row["value"]["attempt"]["basis"]["ownership"]["digest"] =
                        json!(DefinitionDigest::from_bytes([99; 32]));
                }
                3 => {
                    row["value"]["binding"]["holder_basis"]["acceptance"] =
                        json!(CommitToken::from_bytes([99; 16]));
                    row["value"]["attempt"]["basis"] =
                        row["value"]["binding"]["holder_basis"].clone();
                }
                4 => {
                    row["value"]["attempt"] = json!({"kind":"automatic"});
                }
                5 => {
                    row["value"]["binding"]["holder_basis"]["ownership"]["generation"] = json!(1);
                    row["value"]["attempt"]["basis"] =
                        row["value"]["binding"]["holder_basis"].clone();
                }
                _ => {
                    row["value"]["attempt"]["kind"] = json!("holder_retry");
                    row["value"]["attempt"]["previous_operation"] =
                        json!(CommitToken::from_bytes([31; 16]));
                }
            }
        }
        assert!(
            hydrate(&f, &disk).is_err(),
            "crossed transferred-holder evidence admitted"
        );
    }
    Ok((f.store, f.handles, f.disk))
}

pub(in crate::action_state::terminal_capacity) fn refuse_holder_seed<I: Rig>(
    seed: Seed,
) -> TestResult {
    let mut f = fixture_from_seed::<I>(1, false, false, seed)?;
    let before = f.disk.clone();
    assert!(draft(&mut f, 100, None).is_err());
    assert!(f.disk == before, "failed holder renewal changed custody");
    hydrate(&f, &f.disk)?;
    Ok(())
}

pub(in crate::action_state::terminal_capacity) fn paid_follow_renewal_reserves_seven_targets_and_preserves_acquisition_through_retry<
    I: Rig,
>() -> TestResult {
    let mut f = fixture_from_seed::<I>(1, false, false, I::follow_birth_seed()?)?;
    let first = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let r = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(r.record.targets.len(), 7);
    assert_eq!(
        r.record.targets.iter().map(|t| t.ceiling).sum::<usize>(),
        81920
    );
    let p = advance(&mut f, first, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, first, 3, 33)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    let second = CommitToken::from_bytes([34; 16]);
    let p = draft(&mut f, 34, Some(first))?;
    install(&mut f, p)?;
    let p = advance(&mut f, second, 1, 35)?;
    install(&mut f, p)?;
    let p = advance(&mut f, second, 2, 36)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    let term = required(&view(&f), &term_key(TERM, 1, 2)?)?;
    assert_eq!(term["revision"], json!({"kind":"follow","line":"1"}));
    let pair = pair::<I>(&view(&f), second)?;
    assert_eq!(pair.binding()?["format"], json!(3));
    assert_eq!(pair.binding()?["filed"], json!("1"));
    let mut broken = f.disk.clone();
    for ((_, _), row) in &mut broken {
        if (row["family"]["id"] == json!(I::MARKET) || row["family"]["id"] == json!(I::ROLL))
            && row["value"]["operation"] == json!(second)
        {
            row["value"]["binding"]["revision_policy"]["line"] = json!("2");
        }
    }
    assert!(hydrate(&f, &broken).is_err());
    Ok(())
}

/// CT.3 mutations B1 and B2: a renewal protocol cannot be declared in a registry that lacks
/// a closed purchase audit of its issuer's birth. Each audit is taken away alone.
pub(in crate::action_state::terminal_capacity) fn a_declaration_without_a_closed_birth_audit_is_refused<
    I: Rig,
>() -> TestResult {
    let f = fixture::<I>()?;
    let protocol = I::protocol(&f.store.registry).ok_or("protocol")?;
    protocol.validate_families(&f.store.registry)?;
    let stripped = I::without_birth_audits(&f.store.registry);
    assert!(!stripped.is_empty());
    for registry in stripped {
        let refusal = protocol.validate_families(&registry);
        assert!(
            matches!(
                refusal,
                Err(ActionStateError::TerminalCapacity(reason)) if reason == I::BIRTH_AUDITS_REFUSED
            ),
            "a declaration without a closed birth audit answered {refusal:?}"
        );
    }
    Ok(())
}

/// The retained documents with one operation's offer edited and its digest recomputed in
/// both paired images, so that a cold boot reads the edit and not a stale digest.
fn reoffered<I: Rig>(
    f: &Fixture<I>,
    operation: CommitToken,
    edit: impl FnOnce(&mut Value),
) -> TestResult<Disk> {
    let mut disk = f.disk.clone();
    let row = |disk: &mut Disk, family: &str| -> TestResult<(SmolStr, SmolStr)> {
        let raw = operation_key(family, operation)?;
        let key = (
            raw.owner.as_str().into(),
            raw.resolve(&f.store.registry)?.persistence_key(),
        );
        disk.get(&key).ok_or("operation row")?;
        Ok(key)
    };
    let market = row(&mut disk, I::MARKET)?;
    let roll = row(&mut disk, I::ROLL)?;
    let image = disk.get_mut(&market).ok_or("market image")?;
    edit(&mut image["value"]["offer"]);
    let offer_digest = json!(digest(I::OFFER_DOMAIN, &image["value"]["offer"])?);
    image["value"]["offer_digest"] = offer_digest.clone();
    disk.get_mut(&roll).ok_or("roll image")?["value"]["offer_digest"] = offer_digest;
    Ok(disk)
}

/// CT.3 mutations B5 and B6: after a Grant nothing but the issuer's own rule reads the
/// offer's royalty chain, so this is where that rule is held. A personal offer's chain must
/// be the grant's; a House offer may carry none. An offer given a foreign chain is refused
/// at a cold boot by both, each by its own rule.
pub(in crate::action_state::terminal_capacity) fn a_granted_offer_is_held_to_its_issuers_royalty_rule<
    I: Rig,
>() -> TestResult {
    let mut f = fixture::<I>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    // The control: an edit to a field the kernel does not read still boots, so what is
    // refused below is refused for the chain and not for the way the offer was rewritten.
    let control = reoffered(&f, operation, |offer| offer["terms"]["payer"] = json!(2))?;
    assert!(control != f.disk);
    hydrate(&f, &control)?;
    let foreign = json!(DefinitionDigest::from_bytes([9; 32]));
    let crossed = reoffered(&f, operation, |offer| {
        offer["basis"]["royalty_chain_digest"] = foreign;
    })?;
    assert!(
        hydrate(&f, &crossed).is_err(),
        "an offer with a foreign royalty chain was admitted"
    );
    Ok(())
}

/// CT.3 mutations B6 and B8: a Follow binding still names the filed revision its licence
/// was acquired at, and only the issuer's birth check holds it to that (the Follow policy
/// itself reads the line alone). A binding that names another acquisition is refused at a
/// cold boot.
pub(in crate::action_state::terminal_capacity) fn a_follow_binding_is_held_to_the_acquisition_of_its_birth<
    I: Rig,
>() -> TestResult {
    let mut f = fixture_from_seed::<I>(1, false, false, I::follow_birth_seed()?)?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    let p = advance(&mut f, operation, 2, 33)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    assert_eq!(
        pair::<I>(&view(&f), operation)?.binding()?["format"],
        json!(3)
    );
    let mut disk = f.disk.clone();
    for family in [I::MARKET, I::ROLL] {
        let raw = operation_key(family, operation)?;
        let key = raw.resolve(&f.store.registry)?;
        let row = disk
            .get_mut(&(raw.owner.as_str().into(), key.persistence_key()))
            .ok_or("operation")?;
        assert_eq!(row["value"]["binding"]["filed"], json!("1"));
        row["value"]["binding"]["filed"] = json!("2");
    }
    assert!(
        hydrate(&f, &disk).is_err(),
        "a Follow binding that names another acquisition was admitted"
    );
    Ok(())
}

/// Doc 46 Q-46-3, held at the source. The compiler is the proof that a third issuer does
/// not compile (the `compile_fail` example on [`Issuer`], and the cut note's by-hand
/// attempts); this is the tripwire for the day somebody widens the seal instead: the seal
/// is implemented twice, in `issuer.rs`, in a module nothing else can name; the trait is
/// implemented twice, once in each issuer's file; and no line asks for it at run time.
#[test]
fn the_issuer_seal_admits_exactly_two() -> TestResult {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> TestResult {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                sources(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let layer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("action_state")
        .join("terminal_capacity");
    let mut files = Vec::new();
    sources(&layer, &mut files)?;
    files.sort();
    let (mut seals, mut issuers, mut seal_modules) = (Vec::new(), Vec::new(), Vec::new());
    // Spelled apart so that this line is not what it looks for.
    let run_time = concat!("dy", "n ");
    for path in files {
        let name = path
            .strip_prefix(&layer)?
            .to_string_lossy()
            .replace('\\', "/");
        for line in std::fs::read_to_string(&path)?.lines() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            assert!(
                !(line.contains(run_time) && line.contains("Issuer")),
                "{name}: the issuer is resolved at compile time: {line}"
            );
            if line.starts_with("impl") && line.contains("Sealed for ") {
                seals.push((name.clone(), line.to_owned()));
            }
            if line.starts_with("impl") && line.contains(" Issuer for ") {
                issuers.push((name.clone(), line.to_owned()));
            }
            if line.ends_with("mod sealed {") {
                seal_modules.push((name.clone(), line.to_owned()));
            }
        }
    }
    let row = |file: &str, line: &str| (file.to_owned(), line.to_owned());
    assert_eq!(
        seals,
        [
            row(
                "renewal_twin/issuer.rs",
                "impl Sealed for super::Personal {}"
            ),
            row("renewal_twin/issuer.rs", "impl Sealed for super::House {}"),
        ]
    );
    assert_eq!(
        issuers,
        [
            row("house_renewal.rs", "impl Issuer for House {"),
            row("renewal.rs", "impl Issuer for Personal {"),
        ]
    );
    // One seal module, private: the line is the whole declaration, with no `pub` before it.
    assert_eq!(
        seal_modules,
        [row("renewal_twin/issuer.rs", "mod sealed {")]
    );
    Ok(())
}

/// One body run twice: every shared body above as a `#[test]` of the issuer's own test
/// module, under the name it has always had, and the seeds the paid-resale tests take from
/// each issuer. A body left out of this list is dead code, which the lints refuse.
macro_rules! instantiate {
    ($issuer:ty) => {
        $crate::action_state::terminal_capacity::renewal_twin::tests::instantiate! {
            @each $issuer;
            draft_reserves_seven_targets_fund_retains_exact_metadata_and_grant_hydrates
            explicit_retry_after_both_closures_keeps_old_history_valid_after_later_grant
            every_missing_paired_head_receipt_or_grant_row_refuses_cold_recovery
            ordinary_writes_cannot_mutate_a_renewal_generation_or_erase_its_custody
            changed_manifest_keeps_pending_recovery_and_only_allows_current_cleanup
            complete_seven_target_charge_accepts_exact_limit_and_refuses_one_byte_more
            retry_links_and_current_head_are_required_in_cold_history
            paid_follow_renewal_reserves_seven_targets_and_preserves_acquisition_through_retry
            a_declaration_without_a_closed_birth_audit_is_refused
            a_granted_offer_is_held_to_its_issuers_royalty_rule
            a_follow_binding_is_held_to_the_acquisition_of_its_birth
        }

        // Reuse only acknowledged coordinator transitions; native rows remain inert
        // fixture data and do not stand in for real Economy payment authority.
        pub(in crate::action_state::terminal_capacity) fn resale_term_seed(
            grant: bool,
        ) -> TestResult<Seed> {
            $crate::action_state::terminal_capacity::renewal_twin::tests::resale_term_seed::<
                $issuer,
            >(grant)
        }
        pub(in crate::action_state::terminal_capacity) fn closed_holder_seed(
            funded: bool,
        ) -> TestResult<Seed> {
            $crate::action_state::terminal_capacity::renewal_twin::tests::closed_holder_seed::<
                $issuer,
            >(funded)
        }
        pub(in crate::action_state::terminal_capacity) fn continue_holder_seed(
            seed: Seed,
            byte: u8,
            previous: Option<CommitToken>,
            terminal: usize,
        ) -> TestResult<Seed> {
            $crate::action_state::terminal_capacity::renewal_twin::tests::continue_holder_seed::<
                $issuer,
            >(seed, byte, previous, terminal)
        }
        pub(in crate::action_state::terminal_capacity) fn refuse_holder_seed(
            seed: Seed,
        ) -> TestResult {
            $crate::action_state::terminal_capacity::renewal_twin::tests::refuse_holder_seed::<
                $issuer,
            >(seed)
        }
    };
    (@each $issuer:ty; $($name:ident)+) => {
        $(
            #[test]
            fn $name() -> TestResult {
                $crate::action_state::terminal_capacity::renewal_twin::tests::$name::<$issuer>()
            }
        )+
    };
}
pub(in crate::action_state::terminal_capacity) use instantiate;
