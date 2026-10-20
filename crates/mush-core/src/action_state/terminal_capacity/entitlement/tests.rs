//! Real coordinator preparation, installation and hydration; business values
//! are inert fixture data. This does not claim E4 payment or owner entitlement.
use super::*;
use crate::action::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
const QUOTE: &str = "market_mind_entitlement_quote";
const DEPLOYMENT: &str = "roll_mind_commerce";
const OLD_PURCHASE: &str = "market_mind_purchase";
const OLD_RECEIPT: &str = "market_mind_purchase_receipt";
const OLD_ALLOCATION: &str = "roll_mind_paid_purchase";
const OLD_QUOTE: &str = "market_mind_original_quote";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Mode {
    Legacy,
    Original,
    Derived,
    OriginalFollow,
    DerivedFollow,
}
impl Mode {
    fn kind(self) -> Option<Kind> {
        match self {
            Self::Legacy => None,
            Self::Original => Some(Kind::Original),
            Self::Derived => Some(Kind::Derived),
            Self::OriginalFollow => Some(Kind::OriginalFollow),
            Self::DerivedFollow => Some(Kind::DerivedFollow),
        }
    }
    fn families(self) -> [&'static str; 4] {
        if self == Self::Legacy {
            [OLD_PURCHASE, OLD_RECEIPT, OLD_ALLOCATION, OLD_QUOTE]
        } else {
            [PURCHASE, RECEIPT, ALLOCATION, QUOTE]
        }
    }
    fn quote(self) -> CommitToken {
        CommitToken::from_bytes(
            [match self {
                Self::Legacy => 2,
                Self::Original => 3,
                Self::Derived => 4,
                Self::OriginalFollow => 12,
                Self::DerivedFollow => 13,
            }; 16],
        )
    }
    fn operation(self) -> CommitToken {
        CommitToken::from_bytes(
            [match self {
                Self::Legacy => 5,
                Self::Original => 6,
                Self::Derived => 7,
                Self::OriginalFollow => 14,
                Self::DerivedFollow => 15,
            }; 16],
        )
    }
}
fn handle<'a>(handles: &'a BTreeMap<String, Handle>, family: &str) -> TestResult<&'a Handle> {
    handles.get(family).ok_or_else(|| "fixture family".into())
}
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
    actions: BTreeMap<Mode, [DefinitionRef; 3]>,
    invocation: ActionInvocation,
    disk: BTreeMap<(SmolStr, SmolStr), Value>,
}
impl Fixture {
    fn action(&self, mode: Mode, index: usize) -> TestResult<&DefinitionRef> {
        self.actions
            .get(&mode)
            .and_then(|actions| actions.get(index))
            .ok_or_else(|| "fixture action".into())
    }
    fn row(&self, family: &str, key: &str) -> TestResult<Value> {
        self.store
            .get(handle(&self.handles, family)?, &key.to_owned())?
            .cloned()
            .ok_or_else(|| "fixture row".into())
    }
}
fn fixture(manifest_generation: u64, economy_generation: u64) -> TestResult<Fixture> {
    fixture_with_follow(manifest_generation, economy_generation, false)
}
fn fixture_with_follow(
    manifest_generation: u64,
    economy_generation: u64,
    follow: bool,
) -> TestResult<Fixture> {
    let modes = [Mode::Legacy, Mode::Original, Mode::Derived]
        .into_iter()
        .chain(
            follow
                .then_some([Mode::OriginalFollow, Mode::DerivedFollow])
                .into_iter()
                .flatten(),
        )
        .collect::<Vec<_>>();
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
    let mut participants = Vec::new();
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
        participants.push(definitions.register(
            OwnerId::new(owner)?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")?,
                DefinitionId::new(id)?,
                1,
                payload,
            )?,
        )?);
    }
    let participants: [DefinitionRef; 2] = participants.try_into().map_err(|_| "participants")?;
    let mut actions: BTreeMap<Mode, [DefinitionRef; 3]> = BTreeMap::new();
    for mode in modes.iter().copied() {
        let (ids, version) = mode
            .kind()
            .map_or((ORIGINAL_ACTIONS, 1), |k| (k.actions(), k.version()));
        let mut group = Vec::new();
        for (id, transition) in ids.into_iter().zip(TRANSITIONS) {
            let payload = match mode.kind() {
                Some(kind) => action_payload(kind, transition),
                None => {
                    json!({"format":1,"protocol":"original_mind_purchase_v1","transition":transition})
                }
            };
            group.push(
                definitions.register(
                    market.clone(),
                    DefinitionDescriptor::new(
                        DefinitionKind::new("action")?,
                        DefinitionId::new(id)?,
                        version,
                        payload,
                    )?
                    .with_references(vec![
                        manifest.clone(),
                        participants[0].clone(),
                        participants[1].clone(),
                    ]),
                )?,
            );
        }
        actions.insert(mode, group.try_into().map_err(|_| "actions")?);
    }
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(readiness.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    for action in actions.values().flatten() {
        providers.register(market.clone(), action.clone(), Arc::new(Provider))?;
    }
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, readiness.clone())?;
    let mut builder = StateFamilyRegistryBuilder::new();
    let mut handles = BTreeMap::new();
    for (owner, family, bytes) in FAMILIES.into_iter().chain([
        ("market", QUOTE, 24576),
        ("roll", DEPLOYMENT, 4096),
        ("market", OLD_PURCHASE, 32768),
        ("market", OLD_RECEIPT, 8192),
        ("roll", OLD_ALLOCATION, 8192),
        ("market", OLD_QUOTE, 16384),
    ]) {
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
        manifest.clone(),
        participants.clone(),
        actions.get(&Mode::Legacy).ok_or("legacy")?.clone(),
    )?;
    if follow {
        builder.mind_entitlement_purchase_capacity_with_follow(
            &market,
            manifest,
            participants,
            actions[&Mode::Original].clone(),
            actions[&Mode::Derived].clone(),
            actions[&Mode::OriginalFollow].clone(),
            actions[&Mode::DerivedFollow].clone(),
        )?;
    } else {
        builder.mind_entitlement_purchase_capacity(
            &market,
            manifest,
            participants,
            actions.get(&Mode::Original).ok_or("original")?.clone(),
            actions.get(&Mode::Derived).ok_or("derived")?.clone(),
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
                namespace: CanonicalId::new("entitlement_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    let mut f = Fixture {
        store,
        handles,
        providers,
        actions,
        invocation,
        disk: BTreeMap::new(),
    };
    let quotes = modes
        .into_iter()
        .map(|m| Ok((m, quote(&f, m)?)))
        .collect::<TestResult<Vec<_>>>()?;
    let mut txn = f.store.transaction(handle(&f.handles, QUOTE)?)?;
    for (mode, q) in quotes {
        txn.insert(
            handle(&f.handles, mode.families()[3])?,
            &mode.quote().to_hex(),
            q,
        )?;
    }
    txn.insert(handle(&f.handles, DEPLOYMENT)?,&"registry".into(),json!({"format":1,"initialized_by":CommitToken::from_bytes([9;16]),"next_line":1,"next_filed":1,"next_licence":1}))?;
    let prepared = txn.prepare()?;
    install(&mut f, prepared)?;
    Ok(f)
}
fn quote(f: &Fixture, mode: Mode) -> TestResult<Value> {
    let protocol = f
        .store
        .registry
        .entitlement_purchase
        .as_ref()
        .ok_or("protocol")?;
    let hash = DefinitionDigest::from_bytes([3; 32]);
    let revision = json!({"fixture":"exact-revision"});
    let pin = json!({"kind":"pin","filed":"1","revision":revision});
    if mode == Mode::Legacy {
        let product = json!({"kind":{"kind":"original_personal"},"seats":1,"form":{"kind":"sealed"},"class":"II","revision":pin,"transferable":true,"term_seconds":100,"grace_seconds":10});
        return Ok(
            json!({"format":1,"publication":mode.quote(),"buyer":7,"subject":{"charter":"1","line":"1","filed":"1","revision":revision,"rating":hash,"compatibility":protocol.manifest.digest()},"product_digest":digest("market.original_product.v1",&product)?,"product":product}),
        );
    }
    let kind = mode.kind().ok_or("kind")?;
    let pin = if kind.follow() {
        json!({"kind":"follow","line":"1"})
    } else {
        pin
    };
    let parent = if !kind.original() {
        json!({"filed":"9","revision":{"fixture":"actual-parent-identity"}})
    } else {
        Value::Null
    };
    let edges = if !kind.original() {
        json!([{"parent":parent,"charter":"9","line":"9","acquisition":{"licence":"9","term":"1","operation":CommitToken::from_bytes([11;16]),"product_digest":hash},"royalty_permille":100,"basis":{"kind":"explicit_term_economics_v1","digest":hash}}])
    } else {
        json!([])
    };
    let royalty = json!({"format":1,"filed":"1","revision":revision,"publication":CommitToken::from_bytes([10;16]),"edges":edges});
    let terms = json!({"seats":1,"class":"II","form":{"kind":"open_resale"},"transferable":true});
    let policy = if kind.follow() {
        json!(DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("entitlement_policy")?,
                DefinitionId::new("personal")?,
                2,
                json!({"fixture":"exact-policy"})
            )?
        )?)
    } else {
        json!({"fixture":"exact-policy"})
    };
    let instrument = json!("cr");
    let grant = json!({"format":kind.economics_version(),"downstream":{"kind":"source","royalty_permille":50},"renewal":{"format":kind.economics_version(),"instrument":instrument,"price":125,"term_seconds":100,"grace_seconds":10,"terms":terms,"revision":pin,"policy":policy},"royalty_chain":{"filed":"1","digest":digest("roll.mind.filed_royalties.v1",&royalty)?}});
    let product = json!({"protocol":kind.wire(),"terms":terms,"revision":pin,"term_seconds":100,"grace_seconds":10,"grant":grant});
    Ok(
        json!({"format":1,"publication":mode.quote(),"buyer":7,"subject":{"charter":"1","line":"1","filed":"1","revision":revision,"rating":hash,"compatibility":protocol.manifest.digest(),"parent":parent},"product_digest":digest(kind.product_domain(),&product)?,"product":product,"royalty":royalty,"instrument":instrument,"policy":policy}),
    )
}
fn rows(f: &Fixture, mode: Mode, licence: u64) -> TestResult<(Value, Value)> {
    let q = f.row(mode.families()[3], &mode.quote().to_hex())?;
    let protocol = f
        .store
        .registry
        .entitlement_purchase
        .as_ref()
        .ok_or("protocol")?;
    let refs = json!({"economy":protocol.participants[0],"retention":protocol.participants[1]});
    let funding = json!({"contract":1,"hold":2,"transaction":3,"journal_seq":0,"consent_hash":[1,2,3],"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":100}});
    let quote_domain = mode
        .kind()
        .map_or("market.original_quote.v1", |k| k.quote_domain());
    let purchase = json!({"format":1,"operation":mode.operation(),"effect":f.action(mode,0)?,"participants":refs,"quote":q,"quote_digest":digest(quote_domain,&q)?,"licence":licence.to_string(),"term":"1","funding":funding,"phase":{"kind":"funded"},"updated_by":mode.operation()});
    let terms = if mode == Mode::Legacy {
        json!({"seats":1,"class":"II","form":{"kind":"sealed"},"transferable":true})
    } else {
        field(field(&q, "product")?, "terms")?.clone()
    };
    let mut allocation = json!({"format":1,"operation":mode.operation(),"participants":refs,"licence":licence.to_string(),"term":"1","quote":mode.quote(),"quote_digest":field(&purchase,"quote_digest")?,"product_digest":field(&q,"product_digest")?,"holder":7,"charter":"1","line":"1","filed":"1","revision":field(field(&q,"subject")?,"revision")?,"rating":field(field(&q,"subject")?,"rating")?,"terms":terms,"funding":funding,"phase":{"kind":"funded"},"updated_by":mode.operation()});
    if let Some(kind) = mode.kind() {
        let object = allocation.as_object_mut().ok_or("allocation")?;
        object.insert("protocol".into(), json!(kind.wire()));
        object.insert("economics".into(),json!({"format":kind.economics_version(),"licence":licence.to_string(),"generation":"1","operation":mode.operation(),"product_digest":field(&q,"product_digest")?,"grant":field(field(&q,"product")?,"grant")?}));
    }
    Ok((purchase, allocation))
}
#[derive(Default)]
struct FundFault {
    wrong_allocator: bool,
    keep_quote: bool,
    wrong_commit: bool,
    foreign_base: bool,
    no_capacity: bool,
    ordinary: bool,
    limits: Option<ActionTxnLimits>,
}
fn fund(f: &mut Fixture, mode: Mode, fault: FundFault) -> TestResult<PreparedTxn> {
    let [purchase, _, allocation, quote_family] = mode.families();
    let provider = f.providers.get_exact(f.action(mode, 0)?)?;
    let next = number(&f.row(DEPLOYMENT, "registry")?, "next_licence")?;
    let (p, a) = rows(f, mode, next)?;
    let foreign_base = if fault.foreign_base {
        let mut other = fixture(0, 0)?;
        let provider = other.providers.get_exact(other.action(mode, 0)?)?;
        let txn = other
            .store
            .transaction_for_provider(provider.registration_ordinal())?;
        Some(txn.read_base())
    } else {
        None
    };
    let mut txn = if fault.ordinary {
        f.store.transaction(handle(&f.handles, purchase)?)?
    } else {
        f.store
            .transaction_for_provider(provider.registration_ordinal())?
    };
    if !fault.ordinary {
        txn.bind_registered_invocation(provider, &f.invocation, mode.operation());
    }
    let base = txn.read_base();
    if let Some(limits) = fault.limits {
        txn.set_limits(limits)?;
    }
    txn.insert(handle(&f.handles, purchase)?, &mode.operation().to_hex(), p)?;
    txn.insert(
        handle(&f.handles, allocation)?,
        &mode.operation().to_hex(),
        a,
    )?;
    let mut deployment = txn
        .get(handle(&f.handles, DEPLOYMENT)?, &"registry".into())?
        .cloned()
        .ok_or("deployment")?;
    *deployment.get_mut("next_licence").ok_or("next")? =
        json!(next + if fault.wrong_allocator { 2 } else { 1 });
    txn.replace(
        handle(&f.handles, DEPLOYMENT)?,
        &"registry".into(),
        deployment,
    )?;
    if !fault.keep_quote {
        txn.remove(handle(&f.handles, quote_family)?, &mode.quote().to_hex())?;
    }
    if !fault.no_capacity && !fault.ordinary {
        let commit = if fault.wrong_commit {
            CommitToken::from_bytes([99; 16])
        } else {
            mode.operation()
        };
        let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
            .with_registered_transaction(
                foreign_base.as_ref().unwrap_or(&base),
                provider.registration_ordinal(),
            );
        let result = if mode == Mode::Legacy {
            ctx.reserve_original_mind_purchase_completion(&mut txn)
        } else {
            ctx.reserve_mind_entitlement_purchase_completion(&mut txn)
        };
        if fault.wrong_commit || fault.foreign_base {
            assert!(result.is_err());
            drop(ctx);
            assert!(txn.prepare().is_err());
            return Err("caught context mismatch remains poisoned".into());
        }
        result?;
    }
    Ok(txn.prepare()?)
}
#[derive(Clone, Copy)]
enum FinishFault {
    None,
    MissingEconomics,
    CrossedEconomics,
    ChangedFunding,
    RefundEconomics,
    Clock,
    ReusedFundingCommit,
}
fn finish(f: &mut Fixture, mode: Mode, grant: bool, fault: FinishFault) -> TestResult<PreparedTxn> {
    let [purchase, receipt_family, allocation, _] = mode.families();
    let commit = if matches!(fault, FinishFault::ReusedFundingCommit) {
        mode.operation()
    } else {
        CommitToken::from_bytes(
            [20 + match mode {
                Mode::Legacy => 0,
                Mode::Original => 1,
                Mode::Derived => 2,
                Mode::OriginalFollow => 3,
                Mode::DerivedFollow => 4,
            }; 16],
        )
    };
    let provider = f
        .providers
        .get_exact(f.action(mode, if grant { 1 } else { 2 })?)?;
    let mut p = f.row(purchase, &mode.operation().to_hex())?;
    let mut a = f.row(allocation, &mode.operation().to_hex())?;
    let q = field(&p, "quote")?.clone();
    let id = licence(field(&p, "licence")?)?;
    let phase = if grant { "granted" } else { "refunded" };
    for row in [&mut p, &mut a] {
        *row.get_mut("phase").ok_or("phase")? = json!({"kind":phase,"commit":commit});
        *row.get_mut("updated_by").ok_or("updated")? = json!(commit);
    }
    let mut receipt = json!({"format":1,"operation":mode.operation(),"commit":commit,"outcome":{"kind":phase},"contract":1,"hold":2,"transaction":4,"journal_seq":1,"licence":id.to_string(),"term":"1","quote_digest":field(&p,"quote_digest")?,"product_digest":field(&a,"product_digest")?,"issued_at_ic":if grant {json!({"clock":{"kind":"world_ic","generation":if matches!(fault,FinishFault::Clock){2}else{1}},"at":120})}else{Value::Null}});
    if mode != Mode::Legacy {
        receipt.as_object_mut().ok_or("receipt")?.insert(
            "grant_economics_digest".into(),
            json!(digest(
                grant_domain(field(field(&q, "product")?, "grant")?)?,
                field(field(&q, "product")?, "grant")?
            )?),
        );
    }
    if matches!(fault, FinishFault::ChangedFunding) {
        for row in [&mut p, &mut a] {
            *row.pointer_mut("/funding/transaction").ok_or("funding")? = json!(999);
        }
    }
    let mut txn = f
        .store
        .transaction_for_provider(provider.registration_ordinal())?;
    txn.bind_registered_invocation(provider, &f.invocation, commit);
    let base = txn.read_base();
    txn.replace(handle(&f.handles, purchase)?, &mode.operation().to_hex(), p)?;
    txn.replace(
        handle(&f.handles, allocation)?,
        &mode.operation().to_hex(),
        a.clone(),
    )?;
    txn.insert(
        handle(&f.handles, receipt_family)?,
        &mode.operation().to_hex(),
        receipt,
    )?;
    let licence_key = format!("licence/{id:016x}");
    let term_key = format!("{licence_key}/term/0000000000000001");
    if grant {
        txn.insert(handle(&f.handles, LICENCE)?,&licence_key,json!({"format":1,"id":id.to_string(),"charter":"1","line":"1","holder":7,"term":"1","operation":mode.operation()}))?;
        txn.insert(handle(&f.handles, TERM)?,&term_key,json!({"format":1,"licence":id.to_string(),"generation":"1","window":{"format":1,"eligibility_start":120,"issued_at":120,"term_end":220,"grace_end":230},"terms":field(&a,"terms")?,"revision":field(field(&q,"product")?,"revision")?,"operation":mode.operation()}))?;
    }
    if mode != Mode::Legacy
        && ((grant && !matches!(fault, FinishFault::MissingEconomics))
            || matches!(fault, FinishFault::RefundEconomics))
    {
        let mut economics = field(&a, "economics")?.clone();
        if matches!(fault, FinishFault::CrossedEconomics) {
            *economics
                .pointer_mut("/grant/downstream/royalty_permille")
                .ok_or("rate")? = json!(51);
        }
        txn.insert(handle(&f.handles, ECONOMICS)?, &term_key, economics)?;
    }
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    if mode == Mode::Legacy {
        ctx.finish_original_mind_purchase_completion(&mut txn, mode.operation())?;
    } else {
        ctx.finish_mind_entitlement_purchase_completion(&mut txn, mode.operation())?;
    }
    drop(ctx);
    Ok(txn.prepare()?)
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
            _ => return Err("unexpected fixture persistence".into()),
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
    store.hydrate(disk.iter().map(|((ns, key), value)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
    }))?;
    Ok(store)
}
fn reboot(f: &Fixture) -> TestResult<ActionStateStore> {
    hydrate(f, &f.disk)
}

/// Share one actual coordinator-installed first generation with the renewal
/// tests. Business rows remain inert fixtures; this is not an E4 money proof.
pub(in crate::action_state::terminal_capacity) fn renewal_birth_seed() -> TestResult<(
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
)> {
    let mut f = fixture(1, 1)?;
    let prepared = fund(&mut f, Mode::Original, FundFault::default())?;
    install(&mut f, prepared)?;
    let prepared = finish(&mut f, Mode::Original, true, FinishFault::None)?;
    install(&mut f, prepared)?;
    Ok((f.store, f.handles, f.disk))
}

#[test]
fn both_protocols_reserve_exact_six_targets_then_grant_and_hydrate() -> TestResult {
    for mode in [Mode::Original, Mode::Derived] {
        let mut f = fixture(0, 0)?;
        let prepared = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, prepared)?;
        let row = f
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?;
        assert_eq!(row.record.targets.len(), 6);
        assert_eq!(
            row.record.targets.iter().map(|t| t.ceiling).sum::<usize>(),
            69632
        );
        assert_eq!(
            row.record
                .targets
                .iter()
                .filter(|t| matches!(t.mode, SlotMode::UpdateExisting))
                .count(),
            2
        );
        assert_eq!(row.record.sponsor.family.as_str(), PURCHASE);
        assert_eq!(reboot(&f)?.capacity_usage, f.store.capacity_usage);
        let prepared = finish(&mut f, mode, true, FinishFault::None)?;
        install(&mut f, prepared)?;
        assert!(f.store.capacity.is_empty());
        reboot(&f)?;
        for family in [PURCHASE, RECEIPT, ALLOCATION, LICENCE, TERM, ECONOMICS] {
            let mut disk = f.disk.clone();
            let encoded = match family {
                LICENCE => "licence/0000000000000001".to_owned(),
                TERM | ECONOMICS => "licence/0000000000000001/term/0000000000000001".to_owned(),
                _ => mode.operation().to_hex(),
            };
            let owner = if matches!(family, PURCHASE | RECEIPT) {
                "market"
            } else {
                "roll"
            };
            let path = key(owner, family, encoded)?
                .resolve(&f.store.registry)?
                .persistence_key();
            assert!(disk.remove(&(owner.into(), path)).is_some());
            assert_eq!(disk.len() + 1, f.disk.len(), "one exact omitted target");
            assert!(hydrate(&f, &disk).is_err(), "missing {family} for {mode:?}");
        }
    }
    Ok(())
}
#[test]
fn both_refunds_keep_allocations_without_rights_or_economics() -> TestResult {
    for mode in [Mode::Original, Mode::Derived] {
        let mut f = fixture(0, 0)?;
        let p = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, p)?;
        assert!(finish(&mut f, mode, false, FinishFault::RefundEconomics).is_err());
        let p = finish(&mut f, mode, false, FinishFault::None)?;
        install(&mut f, p)?;
        assert!(f.store.capacity.is_empty());
        reboot(&f)?;
        assert_eq!(
            text(
                field(&f.row(ALLOCATION, &mode.operation().to_hex())?, "phase")?,
                "kind"
            )?,
            "refunded"
        );
        for family in [LICENCE, TERM, ECONOMICS] {
            assert!(f.disk.keys().all(|(_, key)| !key.contains(family)));
        }
    }
    Ok(())
}
#[test]
fn no_metadata_or_half_pair_cannot_hide_a_retained_obligation() -> TestResult {
    let mut f = fixture(0, 0)?;
    let p = fund(&mut f, Mode::Derived, FundFault::default())?;
    install(&mut f, p)?;
    for missing in ["action-state/capacity/", PURCHASE, ALLOCATION] {
        let mut disk = f.disk.clone();
        disk.retain(|(_, key), _| !key.contains(missing));
        assert!(hydrate(&f, &disk).is_err(), "missing {missing}");
    }
    Ok(())
}
#[test]
fn economics_and_funding_cannot_change_at_terminal() -> TestResult {
    let mut f = fixture(0, 0)?;
    let p = fund(&mut f, Mode::Derived, FundFault::default())?;
    install(&mut f, p)?;
    for fault in [
        FinishFault::MissingEconomics,
        FinishFault::CrossedEconomics,
        FinishFault::ChangedFunding,
        FinishFault::Clock,
        FinishFault::ReusedFundingCommit,
    ] {
        let before = f.disk.clone();
        let usage = f.store.capacity_usage;
        assert!(finish(&mut f, Mode::Derived, true, fault).is_err());
        assert_eq!(f.disk, before);
        assert_eq!(f.store.capacity_usage, usage);
        assert!(!f.store.has_prepared());
    }
    let p = finish(&mut f, Mode::Derived, true, FinishFault::None)?;
    install(&mut f, p)?;
    reboot(&f)?;
    Ok(())
}
#[test]
fn funding_requires_same_preparation_consumed_quote_and_shared_allocator() -> TestResult {
    for fault in [
        FundFault {
            wrong_allocator: true,
            ..Default::default()
        },
        FundFault {
            keep_quote: true,
            ..Default::default()
        },
        FundFault {
            wrong_commit: true,
            ..Default::default()
        },
        FundFault {
            foreign_base: true,
            ..Default::default()
        },
        FundFault {
            no_capacity: true,
            ..Default::default()
        },
        FundFault {
            ordinary: true,
            ..Default::default()
        },
    ] {
        let mut f = fixture(0, 0)?;
        let before = f.disk.clone();
        assert!(fund(&mut f, Mode::Original, fault).is_err());
        assert_eq!(f.disk, before);
        assert!(f.store.capacity.is_empty());
        assert!(!f.store.has_prepared());
    }
    Ok(())
}
#[test]
fn legacy_and_new_grants_share_allocator_but_never_economics_routing() -> TestResult {
    for order in [
        [Mode::Legacy, Mode::Original, Mode::Derived],
        [Mode::Derived, Mode::Legacy, Mode::Original],
    ] {
        let mut f = fixture(0, 0)?;
        for (index, mode) in order.into_iter().enumerate() {
            let p = fund(&mut f, mode, FundFault::default())?;
            install(&mut f, p)?;
            let p = finish(&mut f, mode, true, FinishFault::None)?;
            install(&mut f, p)?;
            reboot(&f)?;
            let id = u64::try_from(index)? + 1;
            let term_key = format!("licence/{id:016x}/term/0000000000000001");
            assert_eq!(
                f.store
                    .get(handle(&f.handles, ECONOMICS)?, &term_key)?
                    .is_some(),
                mode != Mode::Legacy
            );
            assert_eq!(
                token(field(&f.row(TERM, &term_key)?, "operation")?)?,
                mode.operation()
            );
        }
        assert_eq!(number(&f.row(DEPLOYMENT, "registry")?, "next_licence")?, 4);
    }
    Ok(())
}
#[test]
fn changed_manifest_and_participant_allow_only_historical_refund() -> TestResult {
    for mode in [Mode::Original, Mode::Derived] {
        let mut old = fixture(0, 0)?;
        let p = fund(&mut old, mode, FundFault::default())?;
        install(&mut old, p)?;
        for (manifest, economy) in [(1, 0), (0, 1), (1, 1)] {
            let mut current = fixture(manifest, economy)?;
            current.store = hydrate(&current, &old.disk)?;
            current.disk = old.disk.clone();
            assert!(finish(&mut current, mode, true, FinishFault::None).is_err());
            let p = finish(&mut current, mode, false, FinishFault::None)?;
            install(&mut current, p)?;
            reboot(&current)?;
        }
        let p = finish(&mut old, mode, true, FinishFault::None)?;
        install(&mut old, p)?;
        let current = fixture(1, 1)?;
        assert!(hydrate(&current, &old.disk)?.capacity.is_empty());
    }
    Ok(())
}
#[test]
fn exact_reservation_charge_and_bundle_metadata_hit_existing_limits() -> TestResult {
    let mut measured = fixture(0, 0)?;
    let old = measured.store.live_canonical_bytes;
    let p = fund(&mut measured, Mode::Derived, FundFault::default())?;
    install(&mut measured, p)?;
    let after = measured.store.capacity_usage.bytes;
    let added = after
        .checked_sub(old)
        .ok_or("positive reservation charge")?;
    assert!(added > 0);
    // Only the unrelated aggregate is injected, as in the existing core bound
    // test. All six targets, metadata and measured charge above are real.
    let mut exact = fixture(0, 0)?;
    exact.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added;
    let p = fund(&mut exact, Mode::Derived, FundFault::default())?;
    install(&mut exact, p)?;
    assert_eq!(
        exact.store.capacity_usage.bytes,
        MAX_ACTION_STATE_BOOT_BYTES
    );
    let mut f = fixture(0, 0)?;
    f.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added + 1;
    let error = fund(&mut f, Mode::Derived, FundFault::default())
        .err()
        .ok_or("byte bound")?;
    assert!(
        matches!(error.downcast_ref::<ActionStateError>(),Some(ActionStateError::LiveBytesExceeded{attempted,..}) if *attempted==MAX_ACTION_STATE_BOOT_BYTES+1)
    );
    let mut f = fixture(0, 0)?;
    // Four ordinary writes (two inserts, allocator replace, quote delete) fit;
    // the fifth, actual capacity metadata write must also be counted.
    let error = fund(
        &mut f,
        Mode::Original,
        FundFault {
            limits: Some(ActionTxnLimits::tightened(4, MAX_ACTION_BUNDLE_BYTES)),
            ..Default::default()
        },
    )
    .err()
    .ok_or("metadata operation bound")?;
    assert!(matches!(
        error.downcast_ref::<ActionStateError>(),
        Some(ActionStateError::OperationLimit {
            attempted: 5,
            max: 4
        })
    ));
    // The real reservation remains installed while an unrelated retained-row
    // aggregate reaches the global count ceiling. No million-row fixture is claimed.
    measured.store.capacity_usage.records = MAX_ACTION_STATE_BOOT_RECORDS;
    let extra = measured.row(QUOTE, &Mode::Original.quote().to_hex())?;
    let quote_handle = handle(&measured.handles, QUOTE)?;
    let mut txn = measured.store.transaction(quote_handle)?;
    txn.insert(
        quote_handle,
        &CommitToken::from_bytes([66; 16]).to_hex(),
        extra,
    )?;
    assert!(
        matches!(txn.prepare(), Err(ActionStateError::LiveRecordLimit { attempted, max })
        if attempted == MAX_ACTION_STATE_BOOT_RECORDS + 1 && max == MAX_ACTION_STATE_BOOT_RECORDS)
    );
    Ok(())
}

#[test]
fn refunded_operation_cannot_belong_to_both_recipes_even_without_shared_outputs() -> TestResult {
    for mode in [Mode::Original, Mode::Derived] {
        let mut old = fixture(0, 0)?;
        let p = fund(&mut old, Mode::Legacy, FundFault::default())?;
        install(&mut old, p)?;
        let p = finish(&mut old, Mode::Legacy, false, FinishFault::None)?;
        install(&mut old, p)?;
        let mut remapped = old.disk.clone();
        let mut old_rows = Vec::new();
        for family in [OLD_PURCHASE, OLD_RECEIPT, OLD_ALLOCATION] {
            let owner = if family == OLD_ALLOCATION {
                "roll"
            } else {
                "market"
            };
            let old_path = key(owner, family, Mode::Legacy.operation().to_hex())?
                .resolve(&old.store.registry)?
                .persistence_key();
            let mut document = remapped
                .remove(&(owner.into(), old_path))
                .ok_or("old refunded row")?;
            *document.get_mut("key").ok_or("document key")? = json!(mode.operation().to_hex());
            *document
                .pointer_mut("/value/operation")
                .ok_or("operation")? = json!(mode.operation());
            let owner = if family == OLD_ALLOCATION {
                "roll"
            } else {
                "market"
            };
            let path = key(owner, family, mode.operation().to_hex())?
                .resolve(&old.store.registry)?
                .persistence_key();
            let entry = ((SmolStr::new(owner), path), document);
            remapped.insert(entry.0.clone(), entry.1.clone());
            old_rows.push(entry);
        }
        // This is an independently coherent historical refund, with no licence
        // or term. Its only unusual property is the chosen operation token.
        hydrate(&old, &remapped)?;
        let mut new = fixture(0, 0)?;
        let p = fund(&mut new, mode, FundFault::default())?;
        install(&mut new, p)?;
        let p = finish(&mut new, mode, false, FinishFault::None)?;
        install(&mut new, p)?;
        reboot(&new)?;
        assert!(
            new.disk
                .keys()
                .all(|(_, path)| !path.contains(LICENCE) && !path.contains(TERM))
        );
        let mut crossed = new.disk.clone();
        for (key, document) in old_rows {
            crossed.insert(key, document);
        }
        assert!(
            hydrate(&new, &crossed).is_err(),
            "two refunded allocations for {mode:?}"
        );
    }
    Ok(())
}

#[test]
fn extra_economics_rows_and_crossed_grants_fail_reverse_boot_audit() -> TestResult {
    let mode = Mode::Derived;
    let mut complete = fixture(0, 0)?;
    let p = fund(&mut complete, mode, FundFault::default())?;
    install(&mut complete, p)?;
    let p = finish(&mut complete, mode, true, FinishFault::None)?;
    install(&mut complete, p)?;
    let (economics_path, template) = complete
        .disk
        .iter()
        .find(|((_, path), _)| path.contains(ECONOMICS))
        .map(|(path, value)| (path.clone(), value.clone()))
        .ok_or("economics document")?;
    let mut crossed = complete.disk.clone();
    let economics = crossed.get_mut(&economics_path).ok_or("economics")?;
    *economics
        .pointer_mut("/value/grant/downstream/royalty_permille")
        .ok_or("rate")? = json!(51);
    assert!(hydrate(&complete, &crossed).is_err());
    for terminal in [None, Some(false), Some(true)] {
        let mut f = fixture(0, 0)?;
        let p = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, p)?;
        if let Some(grant) = terminal {
            let p = finish(&mut f, mode, grant, FinishFault::None)?;
            install(&mut f, p)?;
        }
        reboot(&f)?;
        let encoded = "licence/0000000000000002/term/0000000000000001";
        let mut document = template.clone();
        *document.get_mut("key").ok_or("key")? = json!(encoded);
        *document.pointer_mut("/value/licence").ok_or("licence")? = json!("2");
        let path = key("roll", ECONOMICS, encoded.into())?
            .resolve(&f.store.registry)?
            .persistence_key();
        let mut disk = f.disk.clone();
        disk.insert(("roll".into(), path), document);
        assert!(
            hydrate(&f, &disk).is_err(),
            "extra economics at terminal={terminal:?}"
        );
    }
    Ok(())
}

#[test]
fn ordinary_typed_writes_cannot_mutate_or_remove_a_granted_recipe() -> TestResult {
    let mut f = fixture(0, 0)?;
    let mode = Mode::Original;
    let p = fund(&mut f, mode, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, mode, true, FinishFault::None)?;
    install(&mut f, p)?;
    let before = f.disk.clone();
    for (family, encoded) in [
        (PURCHASE, mode.operation().to_hex()),
        (RECEIPT, mode.operation().to_hex()),
        (ALLOCATION, mode.operation().to_hex()),
        (LICENCE, "licence/0000000000000001".into()),
        (
            TERM,
            "licence/0000000000000001/term/0000000000000001".into(),
        ),
        (
            ECONOMICS,
            "licence/0000000000000001/term/0000000000000001".into(),
        ),
    ] {
        // Even republishing an identical row is protected, rather than merely
        // trusting a permissive fixture codec or an unchanged JSON postimage.
        let row = f.row(family, &encoded)?;
        let mut txn = f.store.transaction(handle(&f.handles, PURCHASE)?)?;
        txn.replace(handle(&f.handles, family)?, &encoded, row)?;
        assert!(txn.prepare().is_err(), "ordinary replacement of {family}");
        let mut txn = f.store.transaction(handle(&f.handles, PURCHASE)?)?;
        txn.remove(handle(&f.handles, family)?, &encoded)?;
        assert!(txn.prepare().is_err(), "ordinary delete of {family}");
    }
    assert_eq!(f.disk, before);
    reboot(&f)?;
    Ok(())
}

#[test]
fn acknowledged_quote_pin_cannot_cross_its_exact_subject() -> TestResult {
    for mode in [Mode::Original, Mode::Derived] {
        for part in ["filed", "revision"] {
            let mut f = fixture(0, 0)?;
            let mut quote = f.row(QUOTE, &mode.quote().to_hex())?;
            let pin = quote.pointer_mut("/product/revision").ok_or("pin")?;
            *pin.get_mut(part).ok_or("pin field")? = if part == "filed" {
                json!("2")
            } else {
                json!({"fixture":"crossed-revision"})
            };
            let pin = pin.clone();
            *quote
                .pointer_mut("/product/grant/renewal/revision")
                .ok_or("renewal pin")? = pin;
            let hash = digest(
                mode.kind().ok_or("kind")?.product_domain(),
                field(&quote, "product")?,
            )?;
            *quote.get_mut("product_digest").ok_or("digest")? = json!(hash);
            let mut txn = f.store.transaction(handle(&f.handles, QUOTE)?)?;
            txn.replace(handle(&f.handles, QUOTE)?, &mode.quote().to_hex(), quote)?;
            let p = txn.prepare()?;
            install(&mut f, p)?;
            let before = f.disk.clone();
            assert!(fund(&mut f, mode, FundFault::default()).is_err());
            assert_eq!(f.disk, before);
            assert!(f.store.capacity.is_empty());
        }
    }
    Ok(())
}

#[test]
fn paid_follow_personal_reserves_six_targets_grants_and_cold_checks_every_output() -> TestResult {
    for mode in [Mode::OriginalFollow, Mode::DerivedFollow] {
        let mut f = fixture_with_follow(0, 0, true)?;
        let prepared = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, prepared)?;
        let reservation = f
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?;
        assert_eq!(reservation.record.targets.len(), 6);
        assert_eq!(
            reservation
                .record
                .targets
                .iter()
                .map(|t| t.ceiling)
                .sum::<usize>(),
            69632
        );
        reboot(&f)?;
        let prepared = finish(&mut f, mode, true, FinishFault::None)?;
        install(&mut f, prepared)?;
        assert!(f.store.capacity.is_empty());
        reboot(&f)?;
        assert_eq!(
            f.row(TERM, "licence/0000000000000001/term/0000000000000001")?["revision"],
            json!({"kind":"follow","line":"1"})
        );
        for family in [PURCHASE, RECEIPT, ALLOCATION, LICENCE, TERM, ECONOMICS] {
            let mut disk = f.disk.clone();
            let encoded = match family {
                LICENCE => "licence/0000000000000001".into(),
                TERM | ECONOMICS => "licence/0000000000000001/term/0000000000000001".into(),
                _ => mode.operation().to_hex(),
            };
            let owner = if matches!(family, PURCHASE | RECEIPT) {
                "market"
            } else {
                "roll"
            };
            let k = key(owner, family, encoded)?
                .resolve(&f.store.registry)?
                .persistence_key();
            assert!(disk.remove(&(owner.into(), k)).is_some());
            assert!(hydrate(&f, &disk).is_err());
        }
    }
    Ok(())
}
#[test]
fn paid_follow_personal_refund_and_disabled_recipe_are_distinct() -> TestResult {
    for mode in [Mode::OriginalFollow, Mode::DerivedFollow] {
        let mut f = fixture_with_follow(0, 0, true)?;
        let p = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, p)?;
        let old = fixture(0, 0)?;
        assert!(hydrate(&old, &f.disk).is_err());
        let p = finish(&mut f, mode, false, FinishFault::None)?;
        install(&mut f, p)?;
        reboot(&f)?;
        assert!(f.store.capacity.is_empty());
        assert!(
            f.store
                .get(
                    handle(&f.handles, TERM)?,
                    &"licence/0000000000000001/term/0000000000000001".to_owned()
                )?
                .is_none()
        );
        assert!(hydrate(&old, &f.disk).is_err());
    }
    Ok(())
}
#[test]
fn paid_follow_personal_cannot_cross_line_format_or_old_pin_policy() -> TestResult {
    let f = fixture_with_follow(0, 0, true)?;
    for mode in [Mode::OriginalFollow, Mode::DerivedFollow] {
        let q = quote(&f, mode)?;
        for (pointer, value) in [
            ("/product/revision/line", json!("2")),
            ("/product/grant/format", json!(1)),
            ("/product/grant/renewal/format", json!(1)),
            (
                "/product/revision",
                json!({"kind":"pin","filed":"1","revision":{"fixture":"exact-revision"}}),
            ),
        ] {
            let mut broken = q.clone();
            *broken.pointer_mut(pointer).ok_or("pointer")? = value;
            assert!(quote_kind(&broken).is_err());
        }
    }
    let old = fixture(0, 0)?;
    let encoded = serde_json::to_value(
        old.store
            .registry
            .entitlement_purchase
            .as_ref()
            .ok_or("protocol")?,
    )?;
    assert!(encoded.get("follow").is_none());
    for mode in [Mode::Original, Mode::Derived] {
        assert_eq!(quote(&old, mode)?, quote(&f, mode)?);
        assert_eq!(old.action(mode, 0)?, f.action(mode, 0)?);
    }
    Ok(())
}

pub(in crate::action_state::terminal_capacity) fn follow_renewal_birth_seed() -> TestResult<(
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
)> {
    let mut f = fixture_with_follow(1, 1, true)?;
    let p = fund(&mut f, Mode::OriginalFollow, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, Mode::OriginalFollow, true, FinishFault::None)?;
    install(&mut f, p)?;
    reboot(&f)?;
    Ok((f.store, f.handles, f.disk))
}
