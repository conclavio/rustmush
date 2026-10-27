//! Real coordinator preparation, installation and hydration; business values
//! are inert fixture data. This does not claim E4 payment or owner entitlement.
use super::*;
use crate::action::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
/// A birth seed: the action store, its family handles by name, and the
/// disk image a reboot reads.
type Seed = (
    ActionStateStore,
    BTreeMap<String, Handle>,
    BTreeMap<(SmolStr, SmolStr), Value>,
);

#[test]
fn two_seat_pin_uses_same_six_targets_and_exact_versioned_economics() -> TestResult {
    let mut f = fixture_with_two_seats(0, 0, true)?;
    let p = fund(&mut f, Mode::TwoSeatPin, FundFault::default())?;
    install(&mut f, p)?;
    let saved = f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    assert_eq!(saved.record.targets.len(), 6);
    assert_eq!(
        saved
            .record
            .targets
            .iter()
            .map(|t| t.ceiling)
            .sum::<usize>(),
        69632
    );
    assert_eq!(saved.record.effect.version(), 2);
    reboot(&f)?;
    let p = finish(&mut f, Mode::TwoSeatPin, true, FinishFault::None)?;
    install(&mut f, p)?;
    assert!(f.store.capacity.is_empty());
    reboot(&f)?;
    let a = f.row(ALLOCATION, &Mode::TwoSeatPin.operation().to_hex())?;
    assert_eq!(allocation_version(&a)?, 2);
    assert_eq!(a["terms"]["seats"], json!(2));
    let economics = f.row(ECONOMICS, "licence/0000000000000001/term/0000000000000001")?;
    assert_eq!(
        economics_domain(&economics)?,
        "roll.mind.house_term_economics.v2"
    );
    Ok(())
}

#[test]
fn enabling_two_seat_product_preserves_exact_old_recipe_and_pending_reservation() -> TestResult {
    let mut old = fixture(0, 0)?;
    let enabled = fixture_with_two_seats(0, 0, true)?;
    let old_protocol = old
        .store
        .registry
        .house_purchase
        .as_ref()
        .ok_or("protocol")?;
    let enabled_protocol = enabled
        .store
        .registry
        .house_purchase
        .as_ref()
        .ok_or("protocol")?;
    assert_eq!(old_protocol.house, enabled_protocol.house);
    let serialized = serde_json::to_value(old_protocol)?;
    assert!(serialized.get("two_seat").is_none());
    assert_eq!(serialized.as_object().ok_or("protocol")?.len(), 3);
    assert_eq!(quote(&old, Mode::House)?, quote(&enabled, Mode::House)?);
    let p = fund(&mut old, Mode::House, FundFault::default())?;
    install(&mut old, p)?;
    let saved = old
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?
        .record
        .clone();
    let hydrated = hydrate(&enabled, &old.disk)?;
    assert_eq!(
        hydrated
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?
            .record,
        saved
    );
    Ok(())
}

#[test]
fn old_only_composition_reads_two_seat_history_but_refuses_new_admission() -> TestResult {
    let old = fixture(0, 0)?;
    let mut enabled = fixture_with_two_seats(0, 0, true)?;
    let p = fund(&mut enabled, Mode::TwoSeatPin, FundFault::default())?;
    install(&mut enabled, p)?;
    let restored = hydrate(&old, &enabled.disk)?;
    let saved = restored
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?;
    let view = View {
        registry: &restored.registry,
        records: &restored.records,
        mutations: None,
    };
    verify(Phase::Retained, &saved.record, &view)?;
    assert!(verify(Phase::Funding, &saved.record, &view).is_err());
    let allocation = enabled.row(ALLOCATION, &Mode::TwoSeatPin.operation().to_hex())?;
    assert!(
        old.store
            .registry
            .house_purchase
            .as_ref()
            .ok_or("protocol")?
            .require_product(&allocation)
            .is_err()
    );
    enabled
        .store
        .registry
        .house_purchase
        .as_ref()
        .ok_or("protocol")?
        .require_product(&allocation)?;
    Ok(())
}

#[test]
fn two_seat_product_refuses_mixed_protocol_seats_policy_and_nested_formats() -> TestResult {
    let f = fixture_with_two_seats(0, 0, true)?;
    let q = quote(&f, Mode::TwoSeatPin)?;
    assert_eq!(quote_kind(&q)?, Kind::TwoSeatPin);
    for (path, value) in [
        ("/product/terms/seats", json!(1)),
        ("/product/protocol", json!("house_v1")),
        ("/product/grant/format", json!(1)),
        ("/product/grant/renewal/format", json!(1)),
        ("/policy", json!({"fixture":"not a policy"})),
    ] {
        let mut changed = q.clone();
        *changed.pointer_mut(path).ok_or("path")? = value;
        assert!(quote_kind(&changed).is_err());
    }
    let mut old = quote(&f, Mode::House)?;
    old["product"]["terms"]["seats"] = json!(2);
    assert!(quote_kind(&old).is_err());
    Ok(())
}

#[test]
fn two_seat_cleanup_retains_old_terms_and_never_creates_paid_rights() -> TestResult {
    let mut f = fixture_with_two_seats(0, 0, true)?;
    let p = fund(&mut f, Mode::TwoSeatPin, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, Mode::TwoSeatPin, false, FinishFault::None)?;
    install(&mut f, p)?;
    assert!(f.store.capacity.is_empty());
    reboot(&f)?;
    assert!(
        f.store
            .get(
                handle(&f.handles, LICENCE)?,
                &"licence/0000000000000001".into()
            )?
            .is_none()
    );
    assert_eq!(
        f.row(ALLOCATION, &Mode::TwoSeatPin.operation().to_hex())?["terms"]["seats"],
        json!(2)
    );
    Ok(())
}
const LEGACY_ACTIONS: [&str; 3] = [
    "purchase_original_mind",
    "settle_original_mind",
    "refund_original_mind",
];
const QUOTE: &str = "market_mind_house_quote";
const DEPLOYMENT: &str = "roll_mind_commerce";
const OLD_PURCHASE: &str = "market_mind_purchase";
const OLD_RECEIPT: &str = "market_mind_purchase_receipt";
const OLD_ALLOCATION: &str = "roll_mind_paid_purchase";
const OLD_QUOTE: &str = "market_mind_original_quote";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Mode {
    Legacy,
    House,
    TwoSeatPin,
    TwoSeatFollow,
}
impl Mode {
    fn kind(self) -> Option<Kind> {
        match self {
            Self::Legacy => None,
            Self::House => Some(Kind::House),
            Self::TwoSeatPin => Some(Kind::TwoSeatPin),
            Self::TwoSeatFollow => Some(Kind::TwoSeatFollow),
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
                Self::House => 3,
                Self::TwoSeatPin => 4,
                Self::TwoSeatFollow => 12,
            }; 16],
        )
    }
    fn operation(self) -> CommitToken {
        CommitToken::from_bytes(
            [match self {
                Self::Legacy => 5,
                Self::House => 6,
                Self::TwoSeatPin => 7,
                Self::TwoSeatFollow => 13,
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
pub(in crate::action_state::terminal_capacity) struct Fixture {
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
    fixture_with_two_seats(manifest_generation, economy_generation, false)
}
fn fixture_with_two_seats(
    manifest_generation: u64,
    economy_generation: u64,
    two_seats: bool,
) -> TestResult<Fixture> {
    fixture_with_follow(manifest_generation, economy_generation, two_seats, false)
}
fn fixture_with_follow(
    manifest_generation: u64,
    economy_generation: u64,
    two_seats: bool,
    follow: bool,
) -> TestResult<Fixture> {
    assert!(!follow || two_seats);
    let modes = [Mode::Legacy, Mode::House]
        .into_iter()
        .chain(two_seats.then_some(Mode::TwoSeatPin))
        .chain(follow.then_some(Mode::TwoSeatFollow))
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
            .map_or((LEGACY_ACTIONS, 1), |k| (k.actions(), k.version()));
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
        ("roll", "roll_mind_entitlement_purchase", 8192),
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
    // 17g C2f: the holder's holdings index, which every grant now writes.
    owner_index::tests::register_index_families(
        &mut builder,
        &mut handles,
        &[owner_index::HOLDINGS],
    )?;
    builder.original_mind_purchase_capacity(
        &market,
        manifest.clone(),
        participants.clone(),
        actions.get(&Mode::Legacy).ok_or("legacy")?.clone(),
    )?;
    if follow {
        builder.house_mind_purchase_capacity_with_follow(
            &market,
            manifest,
            participants,
            actions[&Mode::House].clone(),
            actions[&Mode::TwoSeatPin].clone(),
            actions[&Mode::TwoSeatFollow].clone(),
        )?;
    } else if two_seats {
        builder.house_mind_purchase_capacity_with_two_seat_pins(
            &market,
            manifest,
            participants,
            actions.get(&Mode::House).ok_or("house")?.clone(),
            actions.get(&Mode::TwoSeatPin).ok_or("two seat")?.clone(),
        )?;
    } else {
        builder.house_mind_purchase_capacity(
            &market,
            manifest,
            participants,
            actions.get(&Mode::House).ok_or("house")?.clone(),
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
    let protocol = f.store.registry.house_purchase.as_ref().ok_or("protocol")?;
    let hash = DefinitionDigest::from_bytes([3; 32]);
    let revision = json!({"fixture":"exact-revision"});
    let pin = json!({"kind":"pin","filed":"1","revision":revision});
    if mode == Mode::Legacy {
        let product = json!({"kind":{"kind":"original_personal"},"seats":1,"form":{"kind":"sealed"},"class":"II","revision":pin,"transferable":true,"term_seconds":100,"grace_seconds":10});
        return Ok(
            json!({"format":1,"publication":mode.quote(),"buyer":7,"subject":{"charter":"1","line":"1","filed":"1","revision":revision,"rating":hash,"compatibility":protocol.manifest.digest()},"product_digest":digest("market.original_product.v1",&product)?,"product":product}),
        );
    }
    let kind = mode.kind().ok_or("house kind")?;
    let pin = if kind.follow() {
        json!({"kind":"follow","line":"1"})
    } else {
        pin
    };
    let terms =
        json!({"seats":kind.seats(),"class":"II","form":{"kind":"sealed"},"transferable":true});
    let policy = if kind != Kind::House {
        json!(DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("entitlement_policy")?,
                DefinitionId::new("house_personal")?,
                kind.version(),
                json!({"fixture":"two-seat-policy"})
            )?
        )?)
    } else {
        json!({"fixture":"exact-house-policy"})
    };
    let instrument = json!("cr");
    let price = json!(125);
    let grant = json!({"format":kind.version(),"downstream":{"kind":"sealed"},"renewal":{"format":kind.version(),"instrument":instrument,"price":price,"term_seconds":100,"grace_seconds":10,"terms":terms,"revision":pin,"policy":policy}});
    let product = json!({"protocol":kind.wire(),"terms":terms,"revision":pin,"term_seconds":100,"grace_seconds":10,"grant":grant});
    Ok(
        json!({"format":1,"publication":mode.quote(),"buyer":7,"subject":{"issuer":{"format":1,"organization":0,"scope":0},"charter":"1","line":"1","filed":"1","revision":revision,"rating":hash,"compatibility":protocol.manifest.digest()},"product_digest":digest(kind.product_domain(),&product)?,"product":product,"price":price,"instrument":instrument,"policy":policy}),
    )
}

fn rows(f: &Fixture, mode: Mode, licence: u64) -> TestResult<(Value, Value)> {
    let q = f.row(mode.families()[3], &mode.quote().to_hex())?;
    let protocol = f.store.registry.house_purchase.as_ref().ok_or("protocol")?;
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
        object.insert("format".into(), json!(kind.version()));
        object.insert(
            "issuer".into(),
            field(field(&q, "subject")?, "issuer")?.clone(),
        );
        object.insert("economics".into(),json!({"format":kind.version(),"licence":licence.to_string(),"generation":"1","operation":mode.operation(),"product_digest":field(&q,"product_digest")?,"grant":field(field(&q,"product")?,"grant")?}));
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
            ctx.reserve_house_mind_purchase_completion(&mut txn)
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
    BackwardClock,
    ReusedFundingCommit,
}
fn finish(f: &mut Fixture, mode: Mode, grant: bool, fault: FinishFault) -> TestResult<PreparedTxn> {
    finish_with(f, mode, grant, fault, None)
}
/// `finish` with `index` in place of the owners' default index writes
/// (17g C2f).
fn finish_with(
    f: &mut Fixture,
    mode: Mode,
    grant: bool,
    fault: FinishFault,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let [purchase, receipt_family, allocation, _] = mode.families();
    let commit = if matches!(fault, FinishFault::ReusedFundingCommit) {
        mode.operation()
    } else {
        CommitToken::from_bytes(
            [20 + match mode {
                Mode::Legacy => 0,
                Mode::House => 1,
                Mode::TwoSeatPin => 2,
                Mode::TwoSeatFollow => 3,
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
    if matches!(fault, FinishFault::BackwardClock) {
        receipt["issued_at_ic"]["at"] = json!(99);
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
                .pointer_mut("/grant/renewal/price")
                .ok_or("price")? = json!(51);
        }
        txn.insert(handle(&f.handles, ECONOMICS)?, &term_key, economics)?;
    }
    // 17g C2f: every grant puts the holder's holdings row (`owner_index`).
    match index {
        Some(write) => write(&mut txn, &f.handles)?,
        None if grant => owner_index::tests::put_holding(&mut txn, &f.handles, 7, id)?,
        None => {}
    }
    let mut ctx = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), commit, None)
        .with_registered_transaction(&base, provider.registration_ordinal());
    if mode == Mode::Legacy {
        ctx.finish_original_mind_purchase_completion(&mut txn, mode.operation())?;
    } else {
        ctx.finish_house_mind_purchase_completion(&mut txn, mode.operation())?;
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

/// 17g C2f: the owner-index rig. A HouseV1 purchase by buyer 7 is funded
/// (licence 1).
pub(in crate::action_state::terminal_capacity) fn funded_house() -> TestResult<Fixture> {
    let mut f = fixture(0, 0)?;
    let p = fund(&mut f, Mode::House, FundFault::default())?;
    install(&mut f, p)?;
    Ok(f)
}
/// Its Grant (`grant`) or Refund, with `index` in place of the default rows.
pub(in crate::action_state::terminal_capacity) fn house_terminal(
    f: &mut Fixture,
    grant: bool,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    finish_with(f, Mode::House, grant, FinishFault::None, index)
}
/// A two-seat House purchase by buyer 7 funded (licence 1): the HouseV2
/// pin, or (`follow`) core's Follow v3 kind.
pub(in crate::action_state::terminal_capacity) fn funded_two_seat(
    follow: bool,
) -> TestResult<Fixture> {
    let mut f = fixture_with_follow(0, 0, true, follow)?;
    let mode = if follow {
        Mode::TwoSeatFollow
    } else {
        Mode::TwoSeatPin
    };
    let p = fund(&mut f, mode, FundFault::default())?;
    install(&mut f, p)?;
    Ok(f)
}
/// Its Grant (`grant`) or Refund, with `index` in place of the default rows.
pub(in crate::action_state::terminal_capacity) fn two_seat_terminal(
    f: &mut Fixture,
    follow: bool,
    grant: bool,
    index: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let mode = if follow {
        Mode::TwoSeatFollow
    } else {
        Mode::TwoSeatPin
    };
    finish_with(f, mode, grant, FinishFault::None, index)
}
/// Install a prepared step and prove the store reboots.
pub(in crate::action_state::terminal_capacity) fn house_commit(
    f: &mut Fixture,
    prepared: PreparedTxn,
) -> TestResult {
    install(f, prepared)?;
    reboot(f)?;
    Ok(())
}

// Acknowledged actual coordinator birth for the separate House renewal tests.
// Business rows are inert fixture data, not native E4 receipts or paid authority.
pub(in crate::action_state::terminal_capacity) fn renewal_birth_seed() -> TestResult<Seed> {
    let mut f = fixture(1, 1)?;
    let p = fund(&mut f, Mode::House, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, Mode::House, true, FinishFault::None)?;
    install(&mut f, p)?;
    Ok((f.store, f.handles, f.disk))
}

#[test]
fn house_reserve_exact_six_targets_then_grant_and_hydrate() -> TestResult {
    for mode in [Mode::House] {
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
fn house_refunds_keep_allocations_without_rights_or_economics() -> TestResult {
    for mode in [Mode::House] {
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
    let p = fund(&mut f, Mode::House, FundFault::default())?;
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
    let p = fund(&mut f, Mode::House, FundFault::default())?;
    install(&mut f, p)?;
    for fault in [
        FinishFault::MissingEconomics,
        FinishFault::CrossedEconomics,
        FinishFault::ChangedFunding,
        FinishFault::Clock,
        FinishFault::BackwardClock,
        FinishFault::ReusedFundingCommit,
    ] {
        let before = f.disk.clone();
        let usage = f.store.capacity_usage;
        assert!(finish(&mut f, Mode::House, true, fault).is_err());
        assert_eq!(f.disk, before);
        assert_eq!(f.store.capacity_usage, usage);
        assert!(!f.store.has_prepared());
    }
    let p = finish(&mut f, Mode::House, true, FinishFault::None)?;
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
        assert!(fund(&mut f, Mode::House, fault).is_err());
        assert_eq!(f.disk, before);
        assert!(f.store.capacity.is_empty());
        assert!(!f.store.has_prepared());
    }
    Ok(())
}

#[test]
fn changed_manifest_and_participant_allow_only_historical_refund() -> TestResult {
    for mode in [Mode::House] {
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
    let extra = measured.row(QUOTE, &Mode::House.quote().to_hex())?;
    let old = measured.store.live_canonical_bytes;
    let p = fund(&mut measured, Mode::House, FundFault::default())?;
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
    let p = fund(&mut exact, Mode::House, FundFault::default())?;
    install(&mut exact, p)?;
    assert_eq!(
        exact.store.capacity_usage.bytes,
        MAX_ACTION_STATE_BOOT_BYTES
    );
    let mut f = fixture(0, 0)?;
    f.store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES - added + 1;
    let error = fund(&mut f, Mode::House, FundFault::default())
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
        Mode::House,
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
fn legacy_and_house_share_allocator_without_crossing_birth_routes() -> TestResult {
    for order in [[Mode::Legacy, Mode::House], [Mode::House, Mode::Legacy]] {
        let mut f = fixture(0, 0)?;
        for (index, mode) in order.into_iter().enumerate() {
            let p = fund(&mut f, mode, FundFault::default())?;
            install(&mut f, p)?;
            let p = finish(&mut f, mode, true, FinishFault::None)?;
            install(&mut f, p)?;
            reboot(&f)?;
            let id = index + 1;
            let term_key = format!("licence/{id:016x}/term/0000000000000001");
            assert_eq!(
                f.store
                    .get(handle(&f.handles, ECONOMICS)?, &term_key)?
                    .is_some(),
                mode == Mode::House
            );
        }
        assert_eq!(number(&f.row(DEPLOYMENT, "registry")?, "next_licence")?, 3);
    }
    Ok(())
}
#[test]
fn actual_zero_issuer_is_data_but_personal_and_open_shapes_are_refused() -> TestResult {
    let f = fixture(0, 0)?;
    let original = quote(&f, Mode::House)?;
    quote_kind(&original)?;
    for pointer in ["/subject/issuer/format", "/product/terms/seats"] {
        let mut q = original.clone();
        *q.pointer_mut(pointer).ok_or("field")? = json!(2);
        assert!(quote_kind(&q).is_err());
    }
    let mut q = original.clone();
    q["subject"]["issuer"] = json!(7);
    assert!(quote_kind(&q).is_err());
    let mut q = original.clone();
    q["product"]["terms"]["form"] = json!({"kind":"open"});
    assert!(quote_kind(&q).is_err());
    let mut q = original.clone();
    q["product"]["grant"]["renewal"]["price"] = json!(126);
    q["product"]["grant"]["renewal"]["term_seconds"] = json!(7_776_000);
    q["product"]["grant"]["renewal"]["grace_seconds"] = json!(0);
    quote_kind(&q)?;
    for (name, value) in [
        ("price", json!(0)),
        ("price", json!(-1)),
        ("price", json!(u64::MAX)),
        ("term_seconds", json!(0)),
        ("term_seconds", json!(-1)),
        ("term_seconds", json!(7_776_001)),
        ("grace_seconds", json!(-1)),
        ("grace_seconds", json!(259_201)),
    ] {
        let mut q = original.clone();
        q["product"]["grant"]["renewal"][name] = value;
        assert!(quote_kind(&q).is_err());
    }
    Ok(())
}

#[test]
fn house_birth_requires_disjoint_allocation_and_enabled_recovery() -> TestResult {
    let mut f = fixture(0, 0)?;
    let p = fund(&mut f, Mode::House, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, Mode::House, true, FinishFault::None)?;
    install(&mut f, p)?;
    let mut missing = f.disk.clone();
    missing.retain(|(_, key), _| {
        ![PURCHASE, RECEIPT, ALLOCATION, ECONOMICS]
            .iter()
            .any(|family| key.contains(family))
    });
    assert!(
        hydrate(&f, &missing).is_err(),
        "shared rights cannot outlive every birth record"
    );
    for other in [OLD_ALLOCATION, "roll_mind_entitlement_purchase"] {
        let mut crossed = f.disk.clone();
        let other_key = key("roll", other, Mode::House.operation().to_hex())?
            .resolve(&f.store.registry)?
            .persistence_key();
        let original_key = key("roll", ALLOCATION, Mode::House.operation().to_hex())?
            .resolve(&f.store.registry)?
            .persistence_key();
        let mut record = crossed
            .get(&("roll".into(), original_key))
            .ok_or("allocation")?
            .clone();
        assert_eq!(record.pointer("/family/id"), Some(&json!(ALLOCATION)));
        record["family"]["id"] = json!(other);
        record["schema"]["id"] = json!(format!("{other}.value.v1"));
        crossed.insert(("roll".into(), other_key), record);
        assert!(
            hydrate(&f, &crossed).is_err(),
            "crossed allocation must refuse"
        );
    }
    let mut disabled = f.store.registry.clone();
    disabled.house_purchase = None;
    let mut cold = ActionStateStore::new(disabled);
    assert!(
        cold.hydrate(
            f.disk
                .iter()
                .map(|((ns, key), value)| ActionStateBootRecord::new(
                    ns.clone(),
                    key.clone(),
                    value.clone()
                ))
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn terminal_is_buyer_or_refund_service_only() -> TestResult {
    let mut f = fixture(0, 0)?;
    let p = fund(&mut f, Mode::House, FundFault::default())?;
    install(&mut f, p)?;
    for actor in [ObjId(8), crate::GOD] {
        let principal = PrincipalId::Object { id: actor };
        let request = RequestUid::from_bytes([44; 16]);
        f.invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("market")?,
                    namespace: CanonicalId::new("house_fixture")?,
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })?;
        assert!(finish(&mut f, Mode::House, true, FinishFault::None).is_err());
        if actor != crate::GOD {
            assert!(finish(&mut f, Mode::House, false, FinishFault::None).is_err());
        } else {
            let p = finish(&mut f, Mode::House, false, FinishFault::None)?;
            install(&mut f, p)?;
            reboot(&f)?;
        }
    }
    Ok(())
}

#[test]
fn paid_follow_house_uses_six_targets_two_seats_and_exact_cold_inverse() -> TestResult {
    let mode = Mode::TwoSeatFollow;
    for granted in [true, false] {
        let mut f = fixture_with_follow(0, 0, true, true)?;
        let p = fund(&mut f, mode, FundFault::default())?;
        install(&mut f, p)?;
        let r = f
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?;
        assert_eq!(r.record.targets.len(), 6);
        assert_eq!(
            r.record.targets.iter().map(|t| t.ceiling).sum::<usize>(),
            69632
        );
        reboot(&f)?;
        let old = fixture_with_two_seats(0, 0, true)?;
        assert!(hydrate(&old, &f.disk).is_err());
        let p = finish(&mut f, mode, granted, FinishFault::None)?;
        install(&mut f, p)?;
        reboot(&f)?;
        assert!(f.store.capacity.is_empty());
        assert!(hydrate(&old, &f.disk).is_err());
        if granted {
            let t = f.row(TERM, "licence/0000000000000001/term/0000000000000001")?;
            assert_eq!(t["terms"]["seats"], json!(2));
            assert_eq!(t["revision"], json!({"kind":"follow","line":"1"}));
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
    }
    Ok(())
}
#[test]
fn paid_follow_house_cannot_reinterpret_pin_or_change_acquisition_line() -> TestResult {
    let f = fixture_with_follow(0, 0, true, true)?;
    let old = fixture_with_two_seats(0, 0, true)?;
    for mode in [Mode::House, Mode::TwoSeatPin] {
        assert_eq!(quote(&f, mode)?, quote(&old, mode)?);
        assert_eq!(f.action(mode, 0)?, old.action(mode, 0)?);
    }
    assert!(
        serde_json::to_value(
            old.store
                .registry
                .house_purchase
                .as_ref()
                .ok_or("protocol")?
        )?
        .get("follow")
        .is_none()
    );
    let q = quote(&f, Mode::TwoSeatFollow)?;
    for (pointer, value) in [
        ("/product/revision/line", json!("2")),
        ("/product/terms/seats", json!(3)),
        ("/product/grant/format", json!(2)),
        ("/product/grant/renewal/format", json!(2)),
    ] {
        let mut broken = q.clone();
        *broken.pointer_mut(pointer).ok_or("pointer")? = value;
        assert!(quote_kind(&broken).is_err());
    }
    Ok(())
}

pub(in crate::action_state::terminal_capacity) fn follow_renewal_birth_seed() -> TestResult<Seed> {
    let mut f = fixture_with_follow(1, 1, true, true)?;
    let p = fund(&mut f, Mode::TwoSeatFollow, FundFault::default())?;
    install(&mut f, p)?;
    let p = finish(&mut f, Mode::TwoSeatFollow, true, FinishFault::None)?;
    install(&mut f, p)?;
    reboot(&f)?;
    Ok((f.store, f.handles, f.disk))
}
