//! Real registered ActionTxn coordination, installation and cold hydration.
//! Native-shaped rows are inert fixture data; E4 escrow/Registry authority are
//! independently owned and are not manufactured by these capacity tests.
use super::*;
use crate::ObjId;
use crate::action::*;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
pub(in crate::action_state::terminal_capacity) struct Fixture {
    store: ActionStateStore,
    handles: BTreeMap<String, Handle>,
    providers: ActionProviderRegistry,
    actions: [DefinitionRef; 4],
    sanction: DefinitionRef,
    disk: BTreeMap<(SmolStr, SmolStr), Value>,
}
fn commit(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn handle<'a>(f: &'a Fixture, family: &str) -> TestResult<&'a Handle> {
    f.handles.get(family).ok_or_else(|| "family".into())
}
fn fixture(configured: bool) -> TestResult<Fixture> {
    let roll = OwnerId::new("roll")?;
    let owners = BTreeSet::from([roll.clone(), OwnerId::new("economy")?]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    let mut definitions = DefinitionRegistryBuilder::new();
    let participant = |id: &str| -> TestResult<DefinitionDescriptor> {
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new("owner_participant")?,
            DefinitionId::new(id)?,
            1,
            json!({"format":1}),
        )?)
    };
    let economy = definitions.register(OwnerId::new("economy")?, participant("initial_filing")?)?;
    let retention = definitions.register(roll.clone(), participant("commercial_roots")?)?;
    let mut actions = Vec::new();
    for i in 0..4 {
        let a = definitions.register(
            roll.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action")?,
                DefinitionId::new(*ACTIONS.get(i).ok_or("id")?)?,
                if i == 0 { 2 } else { 1 },
                protocol::action_payload(i),
            )?
            .with_references(vec![retention.clone()]),
        )?;
        assert_eq!(a, protocol::action_reference(&retention, i)?);
        actions.push(a);
    }
    let actions: [DefinitionRef; 4] = actions.try_into().map_err(|_| "actions")?;
    let authority = definitions.register(
        roll.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new("registry_authority")?,
            DefinitionId::new("mind_revocation")?,
            1,
            authority(),
        )?,
    )?;
    let sanction = definitions.register(
        roll.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new("registry_term_revocation")?,
            1,
            protocol::revocation_payload(),
        )?
        .with_references(vec![retention.clone(), authority]),
    )?;
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for a in actions.iter().chain([&sanction]) {
        providers.register(roll.clone(), a.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    let mut builder = StateFamilyRegistryBuilder::new();
    let mut handles = BTreeMap::new();
    for (family, max) in FAMILIES
        .into_iter()
        .chain([(SANCTION, 32768), (STATUS, 4096)])
    {
        let h = builder.register(
            roll.clone(),
            protocol::descriptor(family, max)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
        handles.insert(family.to_owned(), h);
    }
    // 17g C2f: the named buyer's offer index, which the Offer and Fund write.
    owner_index::tests::register_index_families(
        &mut builder,
        &mut handles,
        &[owner_index::PENDING_OFFERS],
    )?;
    if configured {
        builder.charter_pending_capacity(
            &roll,
            [economy, retention],
            actions.clone(),
            Some(sanction.clone()),
        )?;
    }
    let store =
        ActionStateStore::new(builder.freeze_for_owners_with_readiness(&owners, readiness)?);
    Ok(Fixture {
        store,
        handles,
        providers,
        actions,
        sanction,
        disk: BTreeMap::new(),
    })
}
fn invocation(actor: u32, n: u8) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes([n; 16]);
    let principal = PrincipalId::Object { id: ObjId(actor) };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: ObjId(actor),
        offered_action: ActionOfferId(CanonicalHash::from_bytes([1; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll")?,
                namespace: CanonicalId::new("pending_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn install(f: &mut Fixture, p: PreparedTxn) -> TestResult {
    let (ops, installation) = p.into_parts();
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                f.disk.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                f.disk.remove(&(op.ns, key));
            }
            _ => return Err("unexpected write".into()),
        }
    }
    f.store.install(installation)?;
    Ok(())
}
fn hydrate(
    f: &Fixture,
    disk: &BTreeMap<(SmolStr, SmolStr), Value>,
) -> TestResult<ActionStateStore> {
    let mut store = ActionStateStore::new(f.store.registry.clone());
    store
        .hydrate(disk.iter().map(|((ns, key), v)| {
            ActionStateBootRecord::new(ns.clone(), key.clone(), v.clone())
        }))?;
    Ok(store)
}
fn row(f: &Fixture, family: &str, k: &str) -> TestResult<Value> {
    f.store
        .get(handle(f, family)?, &k.to_owned())?
        .cloned()
        .ok_or_else(|| "row".into())
}
fn at(g: u64, n: i64) -> Value {
    json!({"clock":{"kind":"world_ic","generation":g},"at":n})
}
fn owner_term() -> Value {
    json!({"format":1,"licence":"4","generation":"1","window":{"format":1,"eligibility_start":0,"issued_at":0,"term_end":500,"grace_end":600},"operation":commit(3)})
}
fn authority() -> Value {
    json!({"format":1,"issuer":"fixture","organization":0,"scope":0,"operators":[9]})
}
fn offer_row(f: &Fixture, id: CommitToken, generation: u64) -> TestResult<Value> {
    let actual = owner_term();
    Ok(
        json!({"format":1,"operation":id,"action":f.actions[0],"charter":"100","birth":commit(1),"ownership":{"generation":generation,"operation":commit(2),"digest":hash(2)},"seller":6,"buyer":7,"seller_account":1,"instrument":"cr","price":100,"title_revision":3,"line":"1","filed":"1","revision":{"fixture":"exact"},"owner_term":{"licence":"4","generation":"1","operation":commit(3),"digest":digest("roll.charter_pending.owner_term.v1",&actual)?,"window":field(&actual,"window")?},"offered_at_ic":at(1,100),"offered_at_utc":1000,"expires_at_utc":2000,"settle_seconds":100}),
    )
}
fn publish_offer(f: &mut Fixture, id: CommitToken, generation: u64) -> TestResult {
    let p = offer_prepared(f, id, generation, None)?;
    install(f, p)
}
/// The seller's (6) Offer naming buyer 7 for charter 100, with `index` in
/// place of the default index write (17g C2f), prepared and not installed.
fn offer_prepared(
    f: &mut Fixture,
    id: CommitToken,
    generation: u64,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let o = offer_row(f, id, generation)?;
    let provider = f.providers.get_exact(&f.actions[0])?;
    let inv = invocation(6, 4)?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, id);
    txn.insert(f.handles.get(OFFER).ok_or("offer handle")?, &id.to_hex(), o)?;
    match index {
        Some(write) => write(&mut txn, &f.handles)?,
        None => index_offer(&mut txn, &f.handles, id)?,
    }
    Ok(txn.prepare()?)
}
/// 17g C2f: the fixture buyer's (7) offer-index key for charter 100.
fn buyer_offers_key() -> String {
    format!("{:020}/{}", 7, charter_key(100))
}
/// 17g C2f: the Offer's index write, in the Offer's transaction.
fn index_offer(
    txn: &mut ActionTxn<'_>,
    handles: &BTreeMap<String, Handle>,
    offer: CommitToken,
) -> TestResult {
    owner_index::tests::index_offer(
        txn,
        handles,
        owner_index::PENDING_OFFERS,
        &buyer_offers_key(),
        json!({"format":1,"buyer":7,"charter":"100"}),
        offer,
    )?;
    Ok(())
}
fn pending_row(
    f: &Fixture,
    id: CommitToken,
    offer_id: CommitToken,
    sequence: u64,
    previous: Value,
) -> TestResult<Value> {
    let o = row(f, OFFER, &offer_id.to_hex())?;
    let consent = json!({"format":1,"operation":id,"effect":f.actions[1],"terms":wire::native_terms(&o)?,"payer":2,"accepted_at_ic":at(1,110),"accepted_at_utc":1010,"deadline":at(1,210)});
    let hold = json!({"id":4,"account":2,"amount":100,"status":"Held","placed_by":{"Object":7},"beneficiary":{"System":"economy.contract"},"purpose":{"Escrow":{"contract":0}},"holder":{"namespace":"roll.charter_pending.v1","key":format!("{}/fund",id.to_hex())},"expires":null,"revision":1,"last_journal_seq":10});
    Ok(
        json!({"format":1,"operation":id,"action":f.actions[1],"sequence":sequence,"previous":previous,"offer":offer_id,"offer_digest":digest("roll.charter_pending.offer.v1",&o)?,"charter":"100","ownership":field(&o,"ownership")?,"owner_term":field(&o,"owner_term")?,"accepted_at_ic":at(1,110),"deadline":at(1,210),"claim_observation":{"generation":1,"next_transition":1,"open_owned":1},"economic":{"intent":{"consent":consent,"issuer":"fixture","catalog":"charter","contract":0,"hold":4},"funding":{"transaction":5,"journal_seq":10,"plan_hash":vec![7u8;32],"hold":hold},"terminal":null},"state":{"phase":"funded"}}),
    )
}
fn pending_head(p: &Value) -> TestResult<Value> {
    Ok(
        json!({"format":1,"charter":field(p,"charter")?,"current":{"sequence":field(p,"sequence")?,"operation":field(p,"operation")?,"digest":digest("roll.charter_pending.record.v1",p)?},"state":field(p,"state")?}),
    )
}
#[derive(Default)]
struct Fault {
    ordinary: bool,
    no_capacity: bool,
    wrong_commit: bool,
    wrong_actor: bool,
    wrong_action: bool,
    missing_head: bool,
    extra: bool,
}
fn fund(f: &mut Fixture, p: Value, fault: Fault) -> TestResult<PreparedTxn> {
    fund_with(f, p, fault, None)
}
/// `fund` with `index` in place of the default index write (17g C2f).
fn fund_with(
    f: &mut Fixture,
    p: Value,
    fault: Fault,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let id = token(field(&p, "operation")?)?;
    let inv = invocation(if fault.wrong_actor { 6 } else { 7 }, 5)?;
    let provider = f
        .providers
        .get_exact(&f.actions[if fault.wrong_action { 2 } else { 1 }])?;
    let head = pending_head(&p)?;
    let exists = f.store.get(handle(f, HEAD)?, &charter_key(100))?.is_some();
    let mut txn = if fault.ordinary {
        f.store
            .transaction(f.handles.get(PENDING).ok_or("pending")?)?
    } else {
        f.store
            .transaction_for_provider(provider.registration_ordinal())?
    };
    if !fault.ordinary {
        txn.bind_registered_invocation(provider, &inv, id);
    }
    let base = txn.read_base();
    // 17g C2f: the Fund takes its offer out of the buyer's index, when the
    // index still holds it (a reused offer's has gone: the protocol refuses).
    let offer = token(field(&p, "offer")?)?;
    let indexed = txn
        .get(
            f.handles
                .get(owner_index::PENDING_OFFERS)
                .ok_or("index handle")?,
            &buyer_offers_key(),
        )?
        .is_some_and(|row| {
            row["offers"]
                .as_array()
                .is_some_and(|o| o.contains(&json!(offer)))
        });
    txn.insert(f.handles.get(PENDING).ok_or("pending")?, &id.to_hex(), p)?;
    match index {
        Some(write) => write(&mut txn, &f.handles)?,
        None if indexed => owner_index::tests::consume_offer(
            &mut txn,
            &f.handles,
            owner_index::PENDING_OFFERS,
            &buyer_offers_key(),
            offer,
        )?,
        None => {}
    }
    if !fault.missing_head {
        if exists {
            txn.replace(f.handles.get(HEAD).ok_or("head")?, &charter_key(100), head)?;
        } else {
            txn.insert(f.handles.get(HEAD).ok_or("head")?, &charter_key(100), head)?;
        }
    }
    if fault.extra {
        txn.insert(
            f.handles.get(STATUS).ok_or("status")?,
            &"unrelated".to_owned(),
            json!({}),
        )?;
    }
    if !fault.no_capacity {
        let mut ctx = crate::resolution::PrepareCtx::new(
            inv.idempotency(),
            if fault.wrong_commit { commit(99) } else { id },
            None,
        )
        .with_registered_transaction(&base, provider.registration_ordinal());
        let r = ctx.reserve_charter_pending_completion(&mut txn);
        if fault.wrong_commit {
            assert!(r.is_err());
            drop(ctx);
            assert!(txn.prepare().is_err());
            return Err("caught mismatch poisoned".into());
        }
        r?;
    }
    Ok(txn.prepare()?)
}
fn terminal_row(f: &Fixture, p: &Value, terminal: CommitToken, kind: &str) -> TestResult<Value> {
    let mut p = p.clone();
    let settles = kind == "last_claim_closed" || kind == "deadline_reached";
    let reason = match kind {
        "last_claim_closed" => json!({"kind":kind,"next_transition":2}),
        "deadline_reached" => json!({"kind":kind,"next_transition":2,"open_owned":1}),
        "registry_revoked" => {
            json!({"kind":kind,"sanction":terminal,"licence":4,"generation":1,"term_digest":field(field(&p,"owner_term")?,"digest")?})
        }
        _ => json!({"kind":kind}),
    };
    let action = if kind == "registry_revoked" {
        f.sanction.clone()
    } else {
        f.actions[if settles { 2 } else { 3 }].clone()
    };
    let decision = json!({"format":1,"operation":terminal,"action":action,"actor":9,"at_ic":at(if kind=="clock_changed"{2}else{1},220),"posted_at_utc":1020,"reason":reason});
    let mut hold = field(field(field(&p, "economic")?, "funding")?, "hold")?.clone();
    hold["status"] = if settles {
        json!({"Consumed":{"transaction":6,"amount":100}})
    } else {
        json!("Released")
    };
    hold["revision"] = json!(2);
    hold["last_journal_seq"] = json!(11);
    let title = if settles {
        json!({"object":100,"title_owner":{"Object":7},"scope":null,"assignment":null,"maintenance_account":null,"catalog":"charter","encumbrances":[],"acquisition":{"transaction":6,"at":1020},"lifecycle":"Active","last_journal_seq":11,"revision":4,"stolen":null,"frozen":false,"frozen_reason":null})
    } else {
        Value::Null
    };
    p["economic"]["terminal"] = json!({"decision":decision,"transaction":6,"journal_seq":11,"plan_hash":vec![8u8;32],"hold":hold,"title":title});
    p["state"] = if settles {
        json!({"phase":"settled","terminal":terminal,"transfer":{"generation":num(field(&p,"ownership")?,"generation")?+1,"operation":terminal,"digest":hash(0)}})
    } else {
        json!({"phase":"refunded","terminal":terminal,"reason":reason})
    };
    if settles {
        let transfer = transfer_from_row(&p)?;
        p["state"]["transfer"]["digest"] =
            json!(digest("roll.charter_ownership.transfer.v2", &transfer)?);
    }
    Ok(p)
}
fn transfer_from_row(p: &Value) -> TestResult<Value> {
    let economic = field(p, "economic")?;
    let decision = field(field(economic, "terminal")?, "decision")?;
    let consent = field(field(economic, "intent")?, "consent")?;
    let terms = field(consent, "terms")?;
    Ok(
        json!({"format":2,"operation":field(decision,"operation")?,"action":field(decision,"action")?,"charter":field(p,"charter")?,"generation":num(field(p,"ownership")?,"generation")?+1,"birth":field(terms,"birth")?,"predecessor":field(p,"ownership")?,"offer":field(p,"offer")?,"offer_digest":field(p,"offer_digest")?,"accepted_at_ic":field(p,"accepted_at_ic")?,"economic":{"pending":economic}}),
    )
}
fn sanction_rows(p: &Value) -> TestResult<(Value, Value)> {
    let d = field(field(field(p, "economic")?, "terminal")?, "decision")?;
    let actual = owner_term();
    let accepted = json!({"kind":"owner","term":actual});
    let cause = json!({"format":1,"operation":field(d,"operation")?,"action":field(d,"action")?,"authority":authority(),"actor":9,"authorization":{"fixture":"inert"},"term":accepted,"charter":"100","line":"1","birth_operation":commit(1),"initial_operation":commit(1),"initial_issuer":"fixture","filed":"1","revision":{"fixture":"exact"},"reason":"registry_review","at_ic":field(d,"at_ic")?});
    let status = json!({"format":1,"licence":"4","generation":"1","operation":field(d,"operation")?,"term_digest":digest("roll.registry_revocation.accepted_term.v1",&accepted)?,"window":field(&actual,"window")?});
    Ok((cause, status))
}
fn finish(
    f: &mut Fixture,
    id: CommitToken,
    p: Value,
    omit: Option<&str>,
) -> TestResult<PreparedTxn> {
    finish_limited(f, id, p, omit, None)
}
fn finish_limited(
    f: &mut Fixture,
    id: CommitToken,
    p: Value,
    omit: Option<&str>,
    limit: Option<ActionTxnLimits>,
) -> TestResult<PreparedTxn> {
    finish_full(f, id, p, omit, limit, None)
}
/// `finish_limited` with `index`'s extra writes (17g C2f: a terminal writes
/// no index row, so the default is none).
fn finish_full(
    f: &mut Fixture,
    id: CommitToken,
    p: Value,
    omit: Option<&str>,
    limit: Option<ActionTxnLimits>,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let decision = field(field(field(&p, "economic")?, "terminal")?, "decision")?;
    let terminal = token(field(decision, "operation")?)?;
    let action = reference(field(decision, "action")?)?;
    let provider = f.providers.get_exact(&action)?;
    let inv = invocation(9, 6)?;
    let settles = text(field(&p, "state")?, "phase")? == "settled";
    let compound = action == f.sanction;
    let head = pending_head(&p)?;
    let transfer = if settles {
        Some(transfer_from_row(&p)?)
    } else {
        None
    };
    let owner_exists = f
        .store
        .get(handle(f, OWNERSHIP)?, &charter_key(100))?
        .is_some();
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, terminal);
    let base = txn.read_base();
    if let Some(limit) = limit {
        txn.set_limits(limit)?;
    }
    if omit != Some(PENDING) {
        txn.replace(
            f.handles.get(PENDING).ok_or("pending")?,
            &id.to_hex(),
            p.clone(),
        )?;
    }
    if omit != Some(HEAD) {
        txn.replace(f.handles.get(HEAD).ok_or("head")?, &charter_key(100), head)?;
    }
    if let Some(t) = transfer {
        let g = num(&t, "generation")?;
        if omit != Some(TRANSFER) {
            txn.insert(
                f.handles.get(TRANSFER).ok_or("transfer")?,
                &transfer_key(100, g),
                t,
            )?;
        }
        if omit != Some(OWNERSHIP) {
            let h =
                json!({"format":1,"charter":"100","current":field(field(&p,"state")?,"transfer")?});
            if owner_exists {
                txn.replace(
                    f.handles.get(OWNERSHIP).ok_or("ownership")?,
                    &charter_key(100),
                    h,
                )?;
            } else {
                txn.insert(
                    f.handles.get(OWNERSHIP).ok_or("ownership")?,
                    &charter_key(100),
                    h,
                )?;
            }
        }
    }
    if compound {
        let (cause, status) = sanction_rows(&p)?;
        if omit != Some(SANCTION) {
            txn.insert(
                f.handles.get(SANCTION).ok_or("cause")?,
                &terminal.to_hex(),
                cause,
            )?;
        }
        if omit != Some(STATUS) {
            txn.insert(
                f.handles.get(STATUS).ok_or("status")?,
                &"licence/0000000000000004/term/0000000000000001".into(),
                status,
            )?;
        }
    }
    if let Some(write) = index {
        write(&mut txn, &f.handles)?;
    }
    let mut ctx = crate::resolution::PrepareCtx::new(inv.idempotency(), terminal, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    ctx.close_charter_pending_completion(&mut txn, id)?;
    drop(ctx);
    Ok(txn.prepare()?)
}
/// 17g C2f: the owner-index rig: a configured pending-charter composition.
pub(in crate::action_state::terminal_capacity) fn pending_rig() -> TestResult<Fixture> {
    fixture(true)
}
/// The seller's Offer `id` naming buyer 7 for charter 100.
pub(in crate::action_state::terminal_capacity) fn pending_offer(
    f: &mut Fixture,
    id: CommitToken,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    offer_prepared(f, id, 1, index)
}
/// Buyer 7's Fund of offer `offer` as operation `id` (the charter's first
/// pending attempt).
pub(in crate::action_state::terminal_capacity) fn pending_fund(
    f: &mut Fixture,
    offer: CommitToken,
    id: CommitToken,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let p = pending_row(f, id, offer, 1, Value::Null)?;
    fund_with(f, p, Fault::default(), index)
}
/// The service Settle (`settle`) or Refund of the funded pending operation
/// `id` as `terminal`, with `index`'s extra writes.
pub(in crate::action_state::terminal_capacity) fn pending_terminal(
    f: &mut Fixture,
    id: CommitToken,
    terminal: CommitToken,
    settle: bool,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let funded = row(f, PENDING, &id.to_hex())?;
    let closed = terminal_row(
        f,
        &funded,
        terminal,
        if settle {
            "deadline_reached"
        } else {
            "clock_changed"
        },
    )?;
    finish_full(f, id, closed, None, None, index)
}
/// Install a prepared step and prove the store reboots.
pub(in crate::action_state::terminal_capacity) fn pending_commit(
    f: &mut Fixture,
    prepared: PreparedTxn,
) -> TestResult {
    install(f, prepared)?;
    hydrate(f, &f.disk)?;
    Ok(())
}
/// The rig as a seed: its store, handles and disk image.
pub(in crate::action_state::terminal_capacity) fn pending_seed(f: Fixture) -> Seed {
    (f.store, f.handles, f.disk)
}
/// The rig's store, family handles and disk image.
type Seed = (
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
);
fn funded() -> TestResult<Fixture> {
    let mut f = fixture(true)?;
    publish_offer(&mut f, commit(4), 1)?;
    let p = pending_row(&f, commit(5), commit(4), 1, Value::Null)?;
    let prepared = fund(&mut f, p, Fault::default())?;
    install(&mut f, prepared)?;
    Ok(f)
}

#[test]
fn pending_four_reserved_rows_settle_and_recover_exact_native_pair() -> TestResult {
    let mut f = funded()?;
    let r = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(r.record.targets.len(), 4);
    assert_eq!(
        r.record.targets.iter().map(|x| x.ceiling).sum::<usize>(),
        90112
    );
    assert_eq!(
        r.record
            .targets
            .iter()
            .filter(|x| x.mode == SlotMode::Create)
            .count(),
        2
    );
    assert_eq!(hydrate(&f, &f.disk)?.capacity_usage, f.store.capacity_usage);
    let p = row(&f, PENDING, &commit(5).to_hex())?;
    let p = terminal_row(&f, &p, commit(6), "last_claim_closed")?;
    let prepared = finish(&mut f, commit(5), p, None)?;
    install(&mut f, prepared)?;
    assert!(f.store.capacity.is_empty());
    hydrate(&f, &f.disk)?;
    for (family, k) in [
        (PENDING, commit(5).to_hex()),
        (HEAD, charter_key(100)),
        (TRANSFER, transfer_key(100, 2)),
        (OWNERSHIP, charter_key(100)),
    ] {
        let mut disk = f.disk.clone();
        let path = key(family, k)?
            .resolve(&f.store.registry)?
            .persistence_key();
        assert!(disk.remove(&("roll".into(), path)).is_some());
        assert!(
            hydrate(&f, &disk).is_err(),
            "exact missing completed target accepted"
        );
    }
    Ok(())
}

#[test]
fn pending_completed_owner_head_can_advance_but_cannot_lose_or_cross_its_chain() -> TestResult {
    let mut f = funded()?;
    let before = row(&f, PENDING, &commit(5).to_hex())?;
    let settled = terminal_row(&f, &before, commit(6), "last_claim_closed")?;
    let prepared = finish(&mut f, commit(5), settled.clone(), None)?;
    install(&mut f, prepared)?;
    let first_ownership = field(field(&settled, "state")?, "transfer")?.clone();
    let mut next_offer = offer_row(&f, commit(7), 2)?;
    next_offer["ownership"] = first_ownership.clone();
    let provider = f.providers.get_exact(&f.actions[0])?;
    let inv = invocation(6, 7)?;
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &inv, commit(7));
    txn.insert(
        f.handles.get(OFFER).ok_or("offer handle")?,
        &commit(7).to_hex(),
        next_offer,
    )?;
    index_offer(&mut txn, &f.handles, commit(7))?;
    let prepared = txn.prepare()?;
    install(&mut f, prepared)?;
    let next = pending_row(
        &f,
        commit(8),
        commit(7),
        2,
        field(&pending_head(&settled)?, "current")?.clone(),
    )?;
    let prepared = fund(&mut f, next, Fault::default())?;
    install(&mut f, prepared)?;
    let second = row(&f, PENDING, &commit(8).to_hex())?;
    let second = terminal_row(&f, &second, commit(9), "last_claim_closed")?;
    let prepared = finish(&mut f, commit(8), second.clone(), None)?;
    install(&mut f, prepared)?;
    hydrate(&f, &f.disk)?;
    let mut rolled_back = f.disk.clone();
    let head_path = key(OWNERSHIP, charter_key(100))?
        .resolve(&f.store.registry)?
        .persistence_key();
    rolled_back
        .get_mut(&("roll".into(), head_path))
        .ok_or("head")?["value"]["current"] = first_ownership;
    assert!(
        hydrate(&f, &rolled_back).is_err(),
        "head before later settlement accepted"
    );
    let mut current = field(field(&second, "state")?, "transfer")?.clone();
    let transfer_handle = handle(&f, TRANSFER)?.clone();
    let ownership_handle = handle(&f, OWNERSHIP)?.clone();
    // Inert later immediate-transfer rows exercise shared-family coexistence.
    // They are not native title receipts; the actual Roll/E4 audit owns that.
    for generation in [4u64, 5] {
        let transfer = json!({"format":1,"operation":commit(generation as u8+10),"action":{"fixture":"immediate"},"charter":"100","generation":generation,"birth":commit(1),"predecessor":current,"offer":commit(generation as u8+20),"offer_digest":hash(9),"accepted_at_ic":at(1,300),"economic":{"fixture":"native-owner-proof-external"}});
        current = json!({"generation":generation,"operation":field(&transfer,"operation")?,"digest":digest("roll.charter_ownership.transfer.v1",&transfer)?});
        let mut txn = f.store.transaction(&transfer_handle)?;
        txn.insert(&transfer_handle, &transfer_key(100, generation), transfer)?;
        txn.replace(
            &ownership_handle,
            &charter_key(100),
            json!({"format":1,"charter":"100","current":current}),
        )?;
        let prepared = txn.prepare()?;
        install(&mut f, prepared)?;
    }
    hydrate(&f, &f.disk)?;
    let path = |family, encoded| -> TestResult<(SmolStr, SmolStr)> {
        Ok((
            "roll".into(),
            key(family, encoded)?
                .resolve(&f.store.registry)?
                .persistence_key(),
        ))
    };
    for (family, encoded) in [
        (OWNERSHIP, charter_key(100)),
        (TRANSFER, transfer_key(100, 4)),
        (TRANSFER, transfer_key(100, 5)),
    ] {
        let mut disk = f.disk.clone();
        assert!(disk.remove(&path(family, encoded)?).is_some());
        assert!(hydrate(&f, &disk).is_err());
    }
    for drift in 0..3 {
        let mut disk = f.disk.clone();
        let k = path(TRANSFER, transfer_key(100, 5))?;
        let transfer = disk
            .get_mut(&k)
            .ok_or("last transfer")?
            .get_mut("value")
            .ok_or("value")?;
        match drift {
            0 => transfer["predecessor"]["digest"] = json!(hash(77)),
            1 => transfer["birth"] = json!(commit(88)),
            _ => transfer["predecessor"]["generation"] = json!(2),
        }
        let digest = digest("roll.charter_ownership.transfer.v1", transfer)?;
        // Re-stamp today's head to ensure the deeper lineage join refuses.
        disk.get_mut(&path(OWNERSHIP, charter_key(100))?)
            .ok_or("head")?["value"]["current"]["digest"] = json!(digest);
        assert!(hydrate(&f, &disk).is_err());
    }
    assert_eq!(row(&f, PENDING, &commit(5).to_hex())?, settled);
    Ok(())
}
#[test]
fn pending_refund_retry_preserves_history_and_offer_is_permanently_one_use() -> TestResult {
    let mut f = funded()?;
    let old = row(&f, PENDING, &commit(5).to_hex())?;
    let closed = terminal_row(&f, &old, commit(6), "clock_changed")?;
    let prepared = finish(&mut f, commit(5), closed.clone(), None)?;
    install(&mut f, prepared)?;
    assert!(f.store.capacity.is_empty());
    assert!(
        f.store
            .get(handle(&f, TRANSFER)?, &transfer_key(100, 2))?
            .is_none()
    );
    hydrate(&f, &f.disk)?;
    let previous = field(&pending_head(&closed)?, "current")?.clone();
    let reused = pending_row(&f, commit(9), commit(4), 2, previous.clone())?;
    // 17g C2f: the first Fund took offer 4 out of the buyer's index, so the
    // index alone would refuse a second Fund of it. Put a plausible row
    // holding it back (an ordinary write outside the protocol) so that only
    // the protocol's own one-use rule can refuse, then take it out again.
    let index_handle = handle(&f, owner_index::PENDING_OFFERS)?.clone();
    let mut txn = f.store.transaction(&index_handle)?;
    txn.insert(
        &index_handle,
        &buyer_offers_key(),
        json!({"format":1,"buyer":7,"charter":"100","offers":[commit(4)]}),
    )?;
    let prepared = txn.prepare()?;
    install(&mut f, prepared)?;
    let refused = fund(&mut f, reused, Fault::default());
    assert!(
        matches!(
            refused
                .as_ref()
                .err()
                .and_then(|e| e.downcast_ref::<ActionStateError>()),
            Some(ActionStateError::TerminalCapacity(
                "pending offer already accepted"
            ))
        ),
        "a reused offer refuses by the one-use rule: {:?}",
        refused.as_ref().err()
    );
    let mut txn = f.store.transaction(&index_handle)?;
    txn.remove(&index_handle, &buyer_offers_key())?;
    let prepared = txn.prepare()?;
    install(&mut f, prepared)?;
    publish_offer(&mut f, commit(7), 1)?;
    let retry = pending_row(&f, commit(8), commit(7), 2, previous)?;
    let p = fund(&mut f, retry, Fault::default())?;
    install(&mut f, p)?;
    let retry = row(&f, PENDING, &commit(8).to_hex())?;
    let retry = terminal_row(&f, &retry, commit(9), "deadline_reached")?;
    let p = finish(&mut f, commit(8), retry, None)?;
    install(&mut f, p)?;
    assert_eq!(row(&f, PENDING, &commit(5).to_hex())?, closed);
    assert!(
        f.store
            .get(handle(&f, TRANSFER)?, &transfer_key(100, 2))?
            .is_some()
    );
    hydrate(&f, &f.disk)?;
    Ok(())
}
#[test]
fn pending_funding_refuses_bypass_partial_union_wrong_actor_action_and_caught_context() -> TestResult
{
    let faults = [
        Fault {
            ordinary: true,
            ..Fault::default()
        },
        Fault {
            no_capacity: true,
            ..Fault::default()
        },
        Fault {
            wrong_commit: true,
            ..Fault::default()
        },
        Fault {
            wrong_actor: true,
            ..Fault::default()
        },
        Fault {
            wrong_action: true,
            ..Fault::default()
        },
        Fault {
            missing_head: true,
            ..Fault::default()
        },
        Fault {
            extra: true,
            ..Fault::default()
        },
    ];
    for fault in faults {
        let mut f = fixture(true)?;
        publish_offer(&mut f, commit(4), 1)?;
        let p = pending_row(&f, commit(5), commit(4), 1, Value::Null)?;
        let rev = f.store.revision();
        let disk = f.disk.clone();
        assert!(fund(&mut f, p, fault).is_err());
        assert_eq!(f.store.revision(), rev);
        assert_eq!(f.disk, disk);
        assert!(f.store.capacity.is_empty());
    }
    Ok(())
}
#[test]
fn pending_terminal_requires_complete_union_and_immutable_funding() -> TestResult {
    for omitted in [PENDING, HEAD, TRANSFER, OWNERSHIP] {
        let mut f = funded()?;
        let p = row(&f, PENDING, &commit(5).to_hex())?;
        let p = terminal_row(&f, &p, commit(6), "last_claim_closed")?;
        assert!(finish(&mut f, commit(5), p, Some(omitted)).is_err());
        assert_eq!(f.store.capacity.records.len(), 1);
    }
    for changed in ["funding", "intent", "time", "commit"] {
        let mut f = funded()?;
        let p = row(&f, PENDING, &commit(5).to_hex())?;
        let mut p = terminal_row(&f, &p, commit(6), "last_claim_closed")?;
        match changed {
            "funding" => p["economic"]["funding"]["plan_hash"] = json!(vec![9u8; 32]),
            "intent" => p["economic"]["intent"]["issuer"] = json!("crossed"),
            "time" => p["economic"]["terminal"]["decision"]["posted_at_utc"] = json!(1009),
            _ => {
                p["economic"]["terminal"]["decision"]["operation"] = json!(commit(4));
                p["state"]["terminal"] = json!(commit(4));
            }
        }
        // Rebuild the outer transfer digest: failure must reach immutable/evidence
        // checks rather than merely depending on an unrepaired outer digest.
        let t = transfer_from_row(&p)?;
        p["state"]["transfer"]["digest"] = json!(digest("roll.charter_ownership.transfer.v2", &t)?);
        assert!(finish(&mut f, commit(5), p, None).is_err());
        assert_eq!(f.store.capacity.records.len(), 1);
    }
    Ok(())
}
#[test]
fn pending_compound_registry_refund_allows_only_two_exact_normal_capacity_rows() -> TestResult {
    for omit in [Some(SANCTION), Some(STATUS), None] {
        let mut f = funded()?;
        let p = row(&f, PENDING, &commit(5).to_hex())?;
        let p = terminal_row(&f, &p, commit(6), "registry_revoked")?;
        let result = finish(&mut f, commit(5), p, omit);
        if omit.is_some() {
            assert!(result.is_err());
            continue;
        }
        install(&mut f, result?)?;
        assert!(f.store.capacity.is_empty());
        assert!(
            f.store
                .get(handle(&f, TRANSFER)?, &transfer_key(100, 2))?
                .is_none()
        );
        hydrate(&f, &f.disk)?;
        for (family, k) in [
            (SANCTION, commit(6).to_hex()),
            (
                STATUS,
                "licence/0000000000000004/term/0000000000000001".into(),
            ),
        ] {
            let mut disk = f.disk.clone();
            let path = key(family, k)?
                .resolve(&f.store.registry)?
                .persistence_key();
            assert!(disk.remove(&("roll".into(), path)).is_some());
            assert!(hydrate(&f, &disk).is_err());
        }
    }
    // Both extra sanction rows remain ordinary writes. A four-operation limit
    // admits the two-row refund plus metadata deletion but refuses the real
    // four-row compound refund plus its metadata deletion.
    for compound in [false, true] {
        let mut f = funded()?;
        let p = row(&f, PENDING, &commit(5).to_hex())?;
        let p = terminal_row(
            &f,
            &p,
            commit(6),
            if compound {
                "registry_revoked"
            } else {
                "recipient_unavailable"
            },
        )?;
        let result = finish_limited(
            &mut f,
            commit(5),
            p,
            None,
            Some(ActionTxnLimits::tightened(4, MAX_ACTION_BUNDLE_BYTES)),
        );
        if compound {
            assert!(result.is_err());
            assert_eq!(f.store.capacity.records.len(), 1);
        } else {
            let prepared = result?;
            assert_eq!(prepared.operation_count(), 3);
            install(&mut f, prepared)?;
        }
    }
    Ok(())
}
#[test]
fn pending_cold_reconstruction_rejects_missing_crossed_and_disabled_evidence() -> TestResult {
    let f = funded()?;
    for target in [OFFER, PENDING, HEAD] {
        let mut disk = f.disk.clone();
        let encoded = match target {
            OFFER => commit(4).to_hex(),
            PENDING => commit(5).to_hex(),
            _ => charter_key(100),
        };
        let path = key(target, encoded)?
            .resolve(&f.store.registry)?
            .persistence_key();
        assert!(disk.remove(&("roll".into(), path)).is_some());
        assert!(hydrate(&f, &disk).is_err());
    }
    let mut disk = f.disk.clone();
    let pk = key(PENDING, commit(5).to_hex())?
        .resolve(&f.store.registry)?
        .persistence_key();
    let hk = key(HEAD, charter_key(100))?
        .resolve(&f.store.registry)?
        .persistence_key();
    let p = disk
        .get_mut(&("roll".into(), pk))
        .ok_or("pending document")?
        .get_mut("value")
        .ok_or("value")?;
    p["economic"]["intent"]["issuer"] = json!("crossed");
    let head = pending_head(p)?;
    *disk
        .get_mut(&("roll".into(), hk))
        .ok_or("head document")?
        .get_mut("value")
        .ok_or("value")? = head;
    assert!(
        hydrate(&f, &disk).is_err(),
        "repaired outer head cannot alter reservation fingerprint"
    );
    let absent = fixture(false)?;
    assert!(hydrate(&absent, &f.disk).is_err());
    let mut missing_reservation = f.disk.clone();
    let reservation = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert!(
        missing_reservation
            .remove(&("roll".into(), reservation.path.as_str().into()))
            .is_some()
    );
    assert!(hydrate(&f, &missing_reservation).is_err());
    Ok(())
}
#[test]
fn pending_final_generation_and_complete_terminal_envelopes_remain_bounded() -> TestResult {
    let mut f = fixture(true)?;
    publish_offer(&mut f, commit(4), 65536)?;
    let o = row(&f, OFFER, &commit(4).to_hex())?;
    // An inert pre-existing ownership head exercises the actual final numeric
    // domain. Native historical ownership proof is intentionally outside Core.
    let ownership_handle = handle(&f, OWNERSHIP)?.clone();
    let mut txn = f.store.transaction(&ownership_handle)?;
    txn.insert(
        &ownership_handle,
        &charter_key(100),
        json!({"format":1,"charter":"100","current":field(&o,"ownership")?}),
    )?;
    let p = txn.prepare()?;
    install(&mut f, p)?;
    let p = pending_row(&f, commit(5), commit(4), 1, Value::Null)?;
    let prepared = fund(&mut f, p, Fault::default())?;
    install(&mut f, prepared)?;
    let view = View {
        registry: &f.store.registry,
        records: &f.store.records,
        mutations: None,
    };
    let pair = pair(&view, commit(5))?;
    require_terminal_room(
        &pair,
        f.store
            .registry
            .charter_pending
            .as_ref()
            .ok_or("protocol")?,
    )?;
    assert!(
        targets(&pair)?
            .iter()
            .any(|t| t.key.encoded == transfer_key(100, 65537))
    );
    let p = terminal_row(&f, &pair.row, commit(6), "last_claim_closed")?;
    let prepared = finish(&mut f, commit(5), p, None)?;
    install(&mut f, prepared)?;
    hydrate(&f, &f.disk)?;
    assert!(publish_offer(&mut f, commit(7), 65537).is_err());
    Ok(())
}
#[test]
fn pending_closed_declaration_rejects_full_reference_drift_and_duplicate_registration() -> TestResult
{
    let f = fixture(true)?;
    let p = f
        .store
        .registry
        .charter_pending
        .as_ref()
        .ok_or("protocol")?;
    let mut actions = p.actions.clone();
    actions.swap(1, 2);
    assert!(
        Protocol::new(
            &OwnerId::new("roll")?,
            p.participants.clone(),
            actions,
            None
        )
        .is_err()
    );
    assert!(
        Protocol::new(
            &OwnerId::new("market")?,
            p.participants.clone(),
            p.actions.clone(),
            None
        )
        .is_err()
    );
    let mut actions = p.actions.clone();
    actions[1] = DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(ACTIONS[1])?,
            1,
            json!({"different":true}),
        )?
        .with_references(vec![p.participants[1].clone()]),
    )?;
    assert!(
        Protocol::new(
            &OwnerId::new("roll")?,
            p.participants.clone(),
            actions,
            None
        )
        .is_err()
    );
    let mut b = StateFamilyRegistryBuilder::new();
    b.charter_pending_capacity(
        &OwnerId::new("roll")?,
        p.participants.clone(),
        p.actions.clone(),
        p.registry_revocation.clone(),
    )?;
    assert!(
        b.charter_pending_capacity(
            &OwnerId::new("roll")?,
            p.participants.clone(),
            p.actions.clone(),
            p.registry_revocation.clone()
        )
        .is_err()
    );
    // An omitted declaration contributes no new fingerprint field or domain.
    let absent = fixture(false)?;
    assert_ne!(
        absent.store.registry.fingerprint(),
        f.store.registry.fingerprint()
    );
    let expected = json!({"families":absent.store.registry.entries.values().map(|family|json!({"owner":family.owner(),"descriptor":family.descriptor().canonical_value()})).collect::<Vec<_>>(),"delegations":[]});
    assert_eq!(
        absent.store.registry.fingerprint(),
        DefinitionDigest::for_canonical_value("state_family_registry.v1", &expected)?
    );
    Ok(())
}

#[path = "registration_tests.rs"]
mod registration_tests;
