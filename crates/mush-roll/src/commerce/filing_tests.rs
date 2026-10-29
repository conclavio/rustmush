#![cfg(test)]

//! Real registered initial-filing composition. The rating is an explicitly
//! structural fixture installed through the real typed families, not a claim
//! that the Registry worker certified this test source. Source recovery, final
//! owner audit, shell reservation, fee journal, title and root installation all
//! follow their actual implementations; no readiness bit is set by the test.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken,
    IdempotencyKey, IdempotencyScope, PerceivedRef, PrincipalId as Actor, RequestUid,
    ScopedIdempotencyKey,
};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{CanonicalId, DefinitionDigest, OwnerId};
use mush_core::{
    ActionStateBootRecord, Engine, GOD, GameModule, ModError, ModuleBootData, ModuleCtx, ModuleOp,
    ModuleWrite, ObjId, Outbox, Registrar, Registry, World,
};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CatalogRef, CreditPolicy, EconomyModule,
    EconomyStore, InstrumentDef, InstrumentId, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde_json::{Value, json};

use super::bootstrap_tests::{self as setup, Manifest, descriptor, manifest_descriptor, reference};
use super::families::CommerceFamilies;
use super::filing::InitialFilingTerms;
use super::module::{CommerceConfig, CommerceMode};
use super::rating::{RatingClass, RatingCompatibility, RatingFamilies, RatingSubject};
use super::records::{canonical_duplicate_digest, schema_line_digest};
use super::roots::{RequiredRoot, RootAnchor};
use super::{FiledRevisionId, RevisionIdentity};
use crate::{
    Envelope, PreparedSource, Revision, RevisionAdapter, RollError, RollModule, RollStore,
    SourceAdapter, WorkingCopy, source_digest,
};

type Disk = BTreeMap<(String, String), Value>;
type Handles = (CommerceFamilies, RatingFamilies);
type ReadAnswers = Arc<
    Mutex<
        Option<
            Result<
                super::holding_reader::OriginalReadResult,
                mush_core::read_service::OwnerReadError,
            >,
        >,
    >,
>;

/// 17g C2a: a requester-side registration the fixture's "tacsim" module adds
/// with its one original-holdings grant (`command_reader`'s tests read the
/// folded command kinds through it, as TacSim's command slot does).
pub(super) type TacsimReads = fn(
    &mut Registrar<'_>,
    mush_core::read_service::OwnerReadHandle<super::holding_reader::OriginalOwnerReader>,
) -> Result<(), ModError>;
struct ReadManifest(Option<TacsimReads>);
struct HoldingQuery {
    reader: mush_core::read_service::OwnerReadHandle<super::holding_reader::OriginalOwnerReader>,
    answers: ReadAnswers,
}
impl mush_core::module::QueryHandler for HoldingQuery {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("live context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::module::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        use super::holding_reader::OriginalReadRequest;
        let request = match args.as_str() {
            Some("page") => OriginalReadRequest::own_page(None),
            Some("admit_foreign") => {
                OriginalReadRequest::admission(ObjId(0), super::LicenceId::new(1).unwrap())
            }
            _ => OriginalReadRequest::own(super::LicenceId::new(1).unwrap()),
        };
        *self.answers.lock().unwrap() = Some(ctx.read_owner(&self.reader, request));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
impl GameModule for ReadManifest {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        Manifest.register(reg)?;
        let reader = super::holding_reader::OriginalOwnerReader::request(
            reg,
            &reference("tacsim", manifest_descriptor()),
        )?;
        let answers: ReadAnswers = Arc::new(Mutex::new(None));
        reg.state(answers.clone());
        if let Some(reads) = self.0 {
            reads(reg, reader.clone())?;
        }
        reg.query_handler("holding.test", Arc::new(HoldingQuery { reader, answers }))
    }
}
fn read_holding(
    engine: &mut Engine,
    actor: ObjId,
    mode: &str,
) -> Result<super::holding_reader::OriginalReadResult, mush_core::read_service::OwnerReadError> {
    engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "holding.test".into(),
            args: json!(mode),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    engine
        .world
        .module_state::<ReadAnswers>()
        .unwrap()
        .lock()
        .unwrap()
        .take()
        .unwrap()
}
pub(super) fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}

#[derive(Clone, Default)]
struct Sources(BTreeMap<DefinitionDigest, String>);
impl SourceAdapter for Sources {
    type Token = (DefinitionDigest, String);
    fn prepare_source(
        &self,
        source: &str,
        _: &str,
    ) -> Result<PreparedSource<Self::Token>, RollError> {
        let digest = source_digest(source);
        Ok(PreparedSource {
            digest,
            operations: vec![],
            token: (digest, source.into()),
        })
    }
    fn read_source(&self, id: DefinitionDigest) -> Result<&str, RollError> {
        self.0
            .get(&id)
            .map(String::as_str)
            .ok_or(RollError::Missing)
    }
}
impl RevisionAdapter for Sources {
    fn normalize_source(&self, source: &str) -> Result<String, RollError> {
        Ok(source.into())
    }
    fn validate(&self, _: &str, envelope: &Envelope) -> Result<(), RollError> {
        envelope.validate()
    }
}
fn retain_ops(disk: &mut Disk, ops: &[ModuleOp]) {
    for op in ops {
        match &op.op {
            ModuleWrite::JsonSet { key, value } => {
                disk.insert((op.ns.to_string(), key.to_string()), value.clone());
            }
            ModuleWrite::Del { key } => {
                disk.remove(&(op.ns.to_string(), key.to_string()));
            }
            ModuleWrite::StreamAppend { .. } => panic!("closed fixture has no stream writes"),
        }
    }
}
fn retain(disk: &mut Disk, writes: &[ReplayWrite]) {
    for write in writes {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => {
                disk.insert((namespace.to_string(), key.to_string()), value.clone());
            }
            ReplayWrite::ModuleDelete { namespace, key } => {
                disk.remove(&(namespace.to_string(), key.to_string()));
            }
            ReplayWrite::LedgerChunkSet { write } => {
                disk.insert(
                    (write.namespace().to_string(), write.key().to_string()),
                    serde_json::to_value(write.chunk()).unwrap(),
                );
            }
            ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => {}
            ReplayWrite::ObjectDelete { .. } => panic!("initial filing never deletes objects"),
        }
    }
}
fn saved(disk: &mut Disk) -> (Sources, Revision) {
    let mut sources = Sources::default();
    let mut roll = RollStore::default();
    let draft = WorkingCopy {
        source: "fn think(view, card, memory) { memory }".into(),
        envelope: Envelope {
            schema: json!({"sections":[]}),
            presets: json!({}),
            profiles: json!({"Default":{}}),
            rules: json!({}),
            declared_class: "I".into(),
            mind_abi: "structural=1".into(),
        },
    };
    let create = roll.prepare_create(GOD, "Original", draft).unwrap();
    roll.publish_with(create, |ops| retain_ops(disk, ops))
        .unwrap();
    let base = roll.mind(GOD, "Original").unwrap().base();
    let prepared = roll
        .prepare_save(GOD, "Original", base, "", None, &sources, &sources)
        .unwrap();
    let (change, source, revision) = prepared.into_parts();
    sources.0.insert(source.token.0, source.token.1);
    roll.publish_with(change, |ops| retain_ops(disk, ops))
        .unwrap();
    (sources, roll.revision(GOD, revision.key).unwrap().clone())
}
struct HydratedRoll {
    module: RollModule,
    sources: Sources,
    handles: Arc<Mutex<Option<Handles>>>,
}
impl GameModule for HydratedRoll {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, r: &mut Registrar<'_>) -> Result<(), ModError> {
        self.module.register(r)?;
        *self.handles.lock().unwrap() = self.module.commerce_test_handles();
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.module.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        ctx.state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("missing Roll"))?
            .audit_sources(&self.sources, &self.sources)
            .map_err(|e| ModError::msg(e.to_string()))
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.module.finalize_owner_composition(ctx)
    }
}
pub(super) fn config() -> CommerceConfig {
    let manifest = reference("tacsim", manifest_descriptor());
    CommerceConfig::new(
        CommerceMode::Active,
        InitialFilingTerms::new(90 * 86400, 3 * 86400).unwrap(),
        manifest.clone(),
        RatingCompatibility::combat_v1(manifest.digest()),
    )
    .unwrap()
}
fn engine(world: World, sources: Sources) -> (Box<Engine>, Handles) {
    engine_with_market(world, sources, false)
}
fn engine_with_market(world: World, sources: Sources, market: bool) -> (Box<Engine>, Handles) {
    engine_with(world, sources, market, None, false)
}
fn engine_with(
    world: World,
    sources: Sources,
    market: bool,
    reads: Option<TacsimReads>,
    commands: bool,
) -> (Box<Engine>, Handles) {
    try_engine(world, sources, market, market, reads, commands).unwrap()
}
fn try_engine_with_market(
    world: World,
    sources: Sources,
    market: bool,
    requester: bool,
) -> Result<(Box<Engine>, Handles), ModError> {
    try_engine(world, sources, market, requester, None, false)
}
/// `commands` opens the folded command kinds on Roll's original-holdings
/// read (`CommerceConfig::with_mind_commands`); `reads` adds the requester.
fn try_engine(
    world: World,
    sources: Sources,
    market: bool,
    requester: bool,
    reads: Option<TacsimReads>,
    commands: bool,
) -> Result<(Box<Engine>, Handles), ModError> {
    let config = if market {
        config().with_original_market_quotes()
    } else {
        config()
    };
    let config = if commands {
        config.with_mind_commands()
    } else {
        config
    };
    let issuer = SystemPrincipalId::new("roll.registry").unwrap();
    let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(1),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .unwrap();
    let economy = EconomyModule::new()
        .with_title_issuers(vec![issuer])
        .with_initial_filing(policy, config.initial_filing_action().unwrap());
    let handles = Arc::new(Mutex::new(None));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(ReadManifest(reads)),
        Box::new(HydratedRoll {
            module: RollModule::with_writers(["tacsim"]).with_commerce(config),
            sources,
            handles: handles.clone(),
        }),
        Box::new(economy),
    ];
    if requester {
        modules.push(Box::new(sale_reads::Market));
    }
    // M8c F11: the command kinds' fixture composes the real org module, so
    // an organisation's holdings read asks a real grant.
    if commands {
        modules.push(Box::new(mush_org::OrgModule::new()));
    }
    let engine = boxed_test_engine(world, Registry::new(), modules)?;
    let handles = handles.lock().unwrap().clone().unwrap();
    Ok((engine, handles))
}
fn load(engine: &mut Engine, disk: &Disk) -> Result<(), ModError> {
    let mut modules: HashMap<_, ModuleBootData> = HashMap::new();
    let mut typed = Vec::new();
    for ((ns, key), value) in disk {
        if mush_core::action_state::is_action_state_persistence_key(key) {
            typed.push(ActionStateBootRecord::new(
                ns.as_str(),
                key.as_str(),
                value.clone(),
            ));
        } else {
            modules
                .entry(ns.as_str().into())
                .or_default()
                .kv
                .push((key.as_str().into(), value.clone()));
        }
    }
    engine.hydrate_action_state(typed).unwrap();
    engine.modules_loaded(
        modules,
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )
}
fn invocation(n: u8, parameters: Value) -> ActionInvocation {
    let request = RequestUid::from_bytes([n; 16]);
    let principal = Actor::Object { id: GOD };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([n; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll").unwrap(),
                namespace: CanonicalId::new("filing_fixture").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
fn seed_accounts(disk: &mut Disk, credit: bool) {
    let instrument = InstrumentDef {
        id: InstrumentId::new("cr").unwrap(),
        ruleset: "fixture".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::Allow,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    };
    disk.insert(
        ("economy".into(), "instrument:cr".into()),
        json!(instrument),
    );
    for (id, owner) in [
        (0, PrincipalId::Object(GOD)),
        (
            1,
            PrincipalId::System(SystemPrincipalId::new("roll.registry").unwrap()),
        ),
    ] {
        let account = Account {
            id: AccountId(id),
            owner,
            instrument: instrument.id.clone(),
            scope: None,
            name: format!("fixture {id}").into(),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            credit: if id == 0 && credit {
                CreditPolicy {
                    allow_negative: true,
                    floor: Some(Amount::from_minor(-1000)),
                }
            } else {
                CreditPolicy::forbid()
            },
            last_journal_seq: 0,
            revision: 1,
            approval: None,
            last_hold_seq: 0,
        };
        disk.insert(("economy".into(), format!("account:{id}")), json!(account));
    }
}
pub(super) struct Fixture {
    pub(super) engine: Box<Engine>,
    pub(super) families: CommerceFamilies,
    ratings: RatingFamilies,
    disk: Disk,
    sources: Sources,
    pub(super) revision: Revision,
    rating: super::rating::RatingRef,
    pub(super) shell: ObjId,
}
impl Fixture {
    pub(super) fn new(credit: bool) -> Self {
        Self::new_with_market(credit, false)
    }
    fn new_with_market(credit: bool, market: bool) -> Self {
        Self::build(credit, market, None, false)
    }
    /// 17g C2a: the same filing fixture with `reads` on the fixture's
    /// "tacsim" module, holding its original-holdings grant; `commands` opens
    /// the folded command kinds on that read, as the player market does.
    pub(super) fn new_with_reads(credit: bool, reads: TacsimReads, commands: bool) -> Self {
        Self::build(credit, false, Some(reads), commands)
    }
    fn build(credit: bool, market: bool, reads: Option<TacsimReads>, commands: bool) -> Self {
        let mut disk = Disk::new();
        let mut initialize = setup::engine(CommerceMode::Initialize);
        setup::boot(&mut initialize).unwrap();
        let (prepared, _) = initialize
            .prepare_registered_composed_authoritative_action(
                &setup::setup_action(),
                &setup::invocation(),
                token(1),
            )
            .unwrap();
        let (writes, install) = prepared.into_parts();
        retain(&mut disk, &writes);
        initialize.install_composed_action(install).unwrap();
        let (sources, revision) = saved(&mut disk);
        seed_accounts(&mut disk, credit);
        let (mut staging, (_, ratings)) = engine_with(
            World::new_minimal(),
            sources.clone(),
            market,
            reads,
            commands,
        );
        let identity = RevisionIdentity::capture(&revision).unwrap();
        // Structural subject hashes are exact bounded fixture values. This is
        // an owner-composition test, not the combat definition/worker gate.
        let definition = DefinitionDigest::for_canonical_value(
            "roll.filing.fixture.definition.v1",
            &revision.envelope().profiles,
        )
        .unwrap();
        let subject = RatingSubject::new(
            identity,
            DefinitionDigest::for_canonical_value(
                "roll.filing.fixture.envelope.v1",
                &json!(revision.envelope()),
            )
            .unwrap(),
            schema_line_digest(revision.envelope()).unwrap(),
            definition,
            definition,
            RatingClass::I,
        );
        let plan = DefinitionDigest::for_canonical_value(
            "roll.filing.fixture.plan.v1",
            &json!({"one_structural_case":true}),
        )
        .unwrap();
        let manifest = reference("tacsim", manifest_descriptor());
        let complete = super::rating::test_support::complete_rating(
            subject,
            RatingCompatibility::combat_v1(manifest.digest()),
            plan,
            token(2),
            staging.world.world_ic_now(),
        )
        .unwrap();
        let rating = complete.receipt.reference().unwrap();
        let mut txn = staging.action_transaction(&ratings.receipts).unwrap();
        txn.insert(
            &ratings.receipts,
            &complete.receipt.publication().job(),
            complete.receipt.clone(),
        )
        .unwrap();
        for page in &complete.pages {
            txn.insert(&ratings.pages, &page.key(), page.clone())
                .unwrap();
        }
        txn.insert(&ratings.jobs, &complete.job.job(), complete.job.clone())
            .unwrap();
        txn.insert(
            &ratings.executions,
            &complete.job.job(),
            complete.execution.clone(),
        )
        .unwrap();
        txn.insert(
            &ratings.completions,
            &complete.completion.job(),
            complete.completion.clone(),
        )
        .unwrap();
        txn.insert(
            &ratings.status,
            &complete.receipt.publication().job(),
            complete.status.clone(),
        )
        .unwrap();
        let (ops, install) = txn.prepare().unwrap().into_parts();
        retain_ops(&mut disk, &ops);
        staging.install_prepared_action(install).unwrap();
        // Matching retained root records are part of this structural boot
        // fixture. No test API opens readiness or claims a worker completion.
        for anchor in [
            RootAnchor::Rating {
                job: complete.receipt.publication().job(),
                id: complete.receipt.id(),
            },
            RootAnchor::RatingJob(complete.job.job()),
        ] {
            let row = crate::model::ReferenceRecord {
                format: crate::FORMAT,
                key: anchor.key(),
                revisions: vec![revision.reference()],
            };
            row.validate().unwrap();
            disk.insert((crate::NS.into(), row.key.key()), json!(row));
        }
        let (mut engine, (families, ratings)) = engine_with(
            World::new_minimal(),
            sources.clone(),
            market,
            reads,
            commands,
        );
        load(&mut engine, &disk).unwrap();
        assert!(
            engine
                .world
                .module_state::<RollStore>()
                .unwrap()
                .commerce_ready()
                .is_ok()
        );
        let reserve = reference(
            "roll",
            descriptor(
                "action",
                mush_core::reserved_shell::RESERVE_ACTION,
                json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
            ),
        );
        let shell = ObjId(engine.world.next_id());
        let (prepared, _) = engine
            .prepare_registered_composed_authoritative_action(
                &reserve,
                &invocation(3, json!({})),
                token(3),
            )
            .unwrap();
        let (writes, install) = prepared.into_parts();
        retain(&mut disk, &writes);
        engine.install_composed_action(install).unwrap();
        assert_eq!(engine.world.get(shell).unwrap().owner, GOD);
        Self {
            engine,
            families,
            ratings,
            disk,
            sources,
            revision,
            rating,
            shell,
        }
    }
    pub(super) fn request(&self) -> ActionInvocation {
        invocation(
            4,
            json!({"shell_request":RequestUid::from_bytes([3;16]), "mind":self.revision.reference().key.mind,
            "revision":self.revision.reference().key.number, "revision_digest":self.revision.reference().digest,
            "rating_job":self.rating.job(), "rating_id":self.rating.id(), "rating_digest":self.rating.receipt_digest(), "payer":0, "charter_name":"Original"}),
        )
    }
    fn economic_state(&self) -> mush_economy::EconomySnapshot {
        self.engine
            .world
            .module_state::<EconomyStore>()
            .unwrap()
            .snapshot()
            .clone()
    }
    fn no_filing(&self) {
        assert!(
            self.engine
                .action_state()
                .get(
                    &self.families.filed,
                    &FiledRevisionId::new(1).unwrap().key()
                )
                .unwrap()
                .is_none()
        );
        assert!(
            self.engine
                .world
                .module_state::<EconomyStore>()
                .unwrap()
                .snapshot()
                .asset(self.shell)
                .is_none()
        );
        assert_eq!(
            self.engine
                .world
                .module_state::<RollStore>()
                .unwrap()
                .commercial_roots()
                .count(),
            2
        );
    }
}

#[test]
fn actual_rating_reader_separates_identical_measured_content_by_admitted_job() {
    let mut f = Fixture::new(true);
    let expected =
        RatingCompatibility::combat_v1(reference("tacsim", manifest_descriptor()).digest());
    let first = {
        let mut txn = f.engine.action_transaction(&f.ratings.receipts).unwrap();
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
            .unwrap()
    };
    let second = super::rating::test_support::complete_rating(
        first.subject().clone(),
        expected.clone(),
        first.plan(),
        token(20),
        f.engine.world.world_ic_now(),
    )
    .unwrap();
    assert_eq!(
        second.receipt.id(),
        first.id(),
        "same measured structural fixture"
    );
    let second_ref = second.receipt.reference().unwrap();
    assert_ne!(second_ref, f.rating);
    let first_page_key = super::rating::RatingPageKey::new(f.rating.job(), 0).unwrap();
    let first_page = f
        .engine
        .action_state()
        .get(&f.ratings.pages, &first_page_key)
        .unwrap()
        .unwrap()
        .clone();
    let second_page_key = super::rating::RatingPageKey::new(second_ref.job(), 0).unwrap();
    // Each independent read scenario stages exactly one image for each key.
    let stage_second = |txn: &mut mush_core::ActionTxn<'_>, crossed_page: bool| {
        txn.insert(
            &f.ratings.receipts,
            &second.job.job(),
            second.receipt.clone(),
        )
        .unwrap();
        txn.insert(&f.ratings.jobs, &second.job.job(), second.job.clone())
            .unwrap();
        txn.insert(
            &f.ratings.executions,
            &second.job.job(),
            second.execution.clone(),
        )
        .unwrap();
        txn.insert(
            &f.ratings.completions,
            &second.job.job(),
            second.completion.clone(),
        )
        .unwrap();
        txn.insert(&f.ratings.status, &second.job.job(), second.status.clone())
            .unwrap();
        for page in &second.pages {
            let value = if crossed_page && page.key() == second_page_key {
                first_page.clone()
            } else {
                page.clone()
            };
            txn.insert(&f.ratings.pages, &page.key(), value).unwrap();
        }
    };
    let mut txn = f.engine.action_transaction(&f.ratings.receipts).unwrap();
    stage_second(&mut txn, false);
    assert_eq!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
            .unwrap(),
        first
    );
    assert_eq!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &second_ref, &expected)
            .unwrap(),
        second.receipt
    );

    let mut crossed = json!(f.rating);
    crossed
        .as_object_mut()
        .unwrap()
        .insert("job".into(), json!(second.job.job()));
    let crossed = serde_json::from_value(crossed).unwrap();
    assert!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &crossed, &expected)
            .is_err(),
        "a content ID alone cannot cross publication attempts"
    );

    drop(txn);
    let mut txn = f.engine.action_transaction(&f.ratings.receipts).unwrap();
    stage_second(&mut txn, true);
    assert!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &second_ref, &expected)
            .is_err(),
        "the actual stored page must bind its lookup job even when content hashes match"
    );
    assert_eq!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
            .unwrap(),
        first
    );
    // This is read/admission evidence, not a Registry publication fixture.
    // Dropping the scratch transaction preserves all committed records.
    drop(txn);
    let mut txn = f.engine.action_transaction(&f.ratings.receipts).unwrap();
    assert!(
        txn.get(&f.ratings.receipts, &second_ref.job())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
            .unwrap(),
        first
    );
}

#[test]
fn actual_rating_reader_requires_matching_execution_even_with_all_certificate_rows_present() {
    let mut f = Fixture::new(true);
    let expected =
        RatingCompatibility::combat_v1(reference("tacsim", manifest_descriptor()).digest());
    let mut txn = f.engine.action_transaction(&f.ratings.receipts).unwrap();
    f.ratings
        .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
        .unwrap();
    txn.remove(&f.ratings.executions, &f.rating.job()).unwrap();
    assert!(
        f.ratings
            .complete_active_receipt_in_txn(&mut txn, &f.rating, &expected)
            .is_err()
    );
    // Discard the missing-row scenario; committed evidence was never changed.
    drop(txn);
    let mut committed = f.engine.action_transaction(&f.ratings.receipts).unwrap();
    f.ratings
        .complete_active_receipt_in_txn(&mut committed, &f.rating, &expected)
        .unwrap();
}

#[test]
fn original_filing_installs_actual_fee_title_rows_roots_and_replays_together() {
    let mut f = Fixture::new(true);
    f.no_filing();
    let economic_before = f.economic_state();
    let object_before = f.engine.world.get(f.shell).unwrap().clone();
    let request = f.request();
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_authoritative_action(
            &config().initial_filing_action().unwrap(),
            &request,
            token(4),
        )
        .unwrap();
    f.no_filing();
    assert_eq!(
        f.economic_state(),
        economic_before,
        "prepare cannot debit or issue title"
    );
    let (writes, install) = prepared.into_parts();
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    assert!(writes.iter().any(|w| matches!(w, ReplayWrite::ModuleJsonSet { namespace, key, .. } if namespace.as_str()=="roll" && key.as_str().starts_with("action-state/"))));
    assert!(writes.iter().any(|w| matches!(w, ReplayWrite::ModuleJsonSet { namespace, key, .. } if namespace.as_str()=="roll" && key.as_str().starts_with("reference:"))));
    retain(&mut f.disk, &writes);
    f.engine.install_composed_action(install).unwrap();
    assert_eq!(
        f.engine.world.get(f.shell),
        Some(&object_before),
        "title never changes inert shell object owner"
    );
    let filed = f
        .engine
        .action_state()
        .get(&f.families.filed, &FiledRevisionId::new(1).unwrap().key())
        .unwrap()
        .unwrap();
    assert_eq!(
        filed.content.revision(),
        RevisionIdentity::capture(&f.revision).unwrap()
    );
    assert_eq!(
        filed.duplicate,
        canonical_duplicate_digest(&f.revision).unwrap()
    );
    let operation = f
        .engine
        .action_state()
        .get(&f.families.operations, &token(4).to_string())
        .unwrap()
        .unwrap();
    let economy = f.engine.world.module_state::<EconomyStore>().unwrap();
    assert_eq!(
        economy
            .snapshot()
            .account(AccountId(0))
            .unwrap()
            .posted_balance,
        Amount::from_minor(-100)
    );
    assert_eq!(
        economy
            .snapshot()
            .account(AccountId(1))
            .unwrap()
            .posted_balance,
        Amount::from_minor(100)
    );
    assert_eq!(
        economy.snapshot().asset(f.shell).unwrap().title_owner,
        PrincipalId::Object(GOD)
    );
    economy
        .filing_receipt_audit()
        .verify(&operation.audit_subject().unwrap())
        .unwrap();
    let roll = f.engine.world.module_state::<RollStore>().unwrap();
    assert_eq!(roll.commercial_roots().count(), 4);
    for anchor in [
        RootAnchor::Filed(filed.content.id()),
        RootAnchor::Operation(token(4)),
    ] {
        roll.check_required_root(&RequiredRoot {
            anchor,
            identity: RevisionIdentity::capture(&f.revision).unwrap(),
        })
        .unwrap();
    }
    let world =
        World::from_objects(f.engine.world.iter().cloned(), f.engine.world.next_id()).unwrap();
    let (mut rebooted, families) = engine(world, f.sources.clone());
    load(&mut rebooted, &f.disk).unwrap();
    assert!(
        rebooted
            .world
            .module_state::<RollStore>()
            .unwrap()
            .commerce_ready()
            .is_ok()
    );
    assert_eq!(
        rebooted
            .action_state()
            .get(&families.0.operations, &token(4).to_string())
            .unwrap(),
        Some(operation)
    );
    assert_eq!(
        rebooted
            .world
            .module_state::<EconomyStore>()
            .unwrap()
            .snapshot()
            .account(AccountId(0))
            .unwrap()
            .posted_balance,
        Amount::from_minor(-100)
    );
}

/// Bar `who` from World IC now for ten IC seconds (the record a principal revocation writes),
/// replacing any earlier one; the window's end.
fn revoke_for_ten(f: &mut Fixture, who: ObjId, operation: u8) -> i64 {
    let now = f.engine.world.world_ic_now().at;
    let family = f.families.revoked_principals.clone();
    let key = format!("{:020}", who.0);
    let held = f
        .engine
        .action_state()
        .get(&family, &key)
        .unwrap()
        .is_some();
    let record = super::registry_inspection::RevokedPrincipal {
        format: 1,
        principal: mush_org::PrincipalId::Object(who),
        from_ic: now,
        until_ic: now + 10,
        inspection: token(40),
        hull: ObjId(12),
        seq: 1,
        operation: token(operation),
        operator: ObjId(5),
    };
    let mut txn = f.engine.action_transaction(&family).unwrap();
    if held {
        txn.replace(&family, &key, record).unwrap();
    } else {
        txn.insert(&family, &key, record).unwrap();
    }
    let (_, installation) = txn.prepare().unwrap().into_parts();
    f.engine.install_prepared_action(installation).unwrap();
    now + 10
}

/// Move the world's clock until its IC second reaches `until`.
fn pass(f: &mut Fixture, until: i64) {
    let wall = f.engine.world.now();
    let mut step = 1;
    while f.engine.world.world_ic_now().at < until {
        f.engine.world.set_now(wall + step);
        step *= 2;
    }
}

/// **Doc 17h cut M8d D4 with fix round K3: the write and use bars at both ends of the window**
/// (the lead's ruling on option (c)):
/// - **K3**: a revoked filer files nothing — the filing that would write their owner licence is
///   refused while the window runs, and admitted once it ends;
/// - **the use bar**: the filed owner licence reads as held through the registered holding
///   reader (the dispatch holding's read); revoked again, it reads nothing from the window's
///   first instant, and the same held term reads again when the window ends, nothing destroyed.
#[test]
fn m8d_a_revoked_principal_files_nothing_and_uses_nothing_until_the_window_ends() {
    let mut f = Fixture::new(true);
    let end = revoke_for_ten(&mut f, GOD, 41);
    let request = f.request();
    assert!(
        f.engine
            .prepare_registered_composed_authoritative_action(
                &config().initial_filing_action().unwrap(),
                &request,
                token(4),
            )
            .is_err(),
        "K3: a revoked filer files nothing"
    );
    pass(&mut f, end);
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_authoritative_action(
            &config().initial_filing_action().unwrap(),
            &request,
            token(4),
        )
        .unwrap();
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install).unwrap();
    let held = |f: &mut Fixture| {
        read_holding(&mut f.engine, GOD, "own")
            .ok()
            .and_then(super::holding_reader::OriginalReadResult::into_holding)
            .is_some()
    };
    assert!(held(&mut f), "filed once the window ended, and held");
    let end = revoke_for_ten(&mut f, GOD, 42);
    assert!(!held(&mut f), "barred from its first instant");
    pass(&mut f, end);
    assert!(held(&mut f), "the held term resumes when the window ends");
}

#[test]
fn registered_holding_reader_uses_actual_title_actor_and_current_ready_records() {
    use mush_core::read_service::OwnerReadError;
    let mut f = Fixture::new(true);
    assert!(matches!(
        read_holding(&mut f.engine, GOD, "own"),
        Err(OwnerReadError::Refused)
    ));
    let request = f.request();
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_authoritative_action(
            &config().initial_filing_action().unwrap(),
            &request,
            token(4),
        )
        .unwrap();
    let (_, install) = prepared.into_parts();
    f.engine.install_composed_action(install).unwrap();
    let own = read_holding(&mut f.engine, GOD, "own")
        .unwrap()
        .into_holding()
        .unwrap();
    assert_eq!(own.actor(), GOD);
    assert_eq!(own.filed(), FiledRevisionId::new(1).unwrap());
    assert_eq!(
        own.identity(),
        RevisionIdentity::capture(&f.revision).unwrap()
    );
    assert_eq!(own.rating(), f.rating);
    let page = read_holding(&mut f.engine, GOD, "page").unwrap();
    assert_eq!(page.holding(), Some(&own));
    assert_eq!(page.next_after(), None);
    assert!(matches!(
        read_holding(&mut f.engine, ObjId(0), "own"),
        Err(OwnerReadError::Refused)
    ));
    assert!(matches!(
        read_holding(&mut f.engine, GOD, "admit_foreign"),
        Err(OwnerReadError::Refused)
    ));
    assert!(
        read_holding(&mut f.engine, ObjId(0), "page")
            .unwrap()
            .holding()
            .is_none()
    );
    // Real revalidation closes commerce even when the source bytes still match;
    // a formerly returned projection cannot reopen it or extend its own term.
    f.engine
        .world
        .module_state_mut::<RollStore>()
        .unwrap()
        .audit_sources(&f.sources, &f.sources)
        .unwrap();
    assert!(matches!(
        read_holding(&mut f.engine, GOD, "own"),
        Err(OwnerReadError::Unavailable)
    ));
}

#[test]
fn insufficient_fee_refuses_without_consuming_shell_allocators_rows_or_roots() {
    let mut f = Fixture::new(false);
    let before = f.economic_state();
    let count = f.engine.action_state().len();
    let request = f.request();
    assert!(
        f.engine
            .prepare_registered_composed_authoritative_action(
                &config().initial_filing_action().unwrap(),
                &request,
                token(4)
            )
            .is_err()
    );
    f.no_filing();
    assert_eq!(f.economic_state(), before);
    assert_eq!(f.engine.action_state().len(), count);
    let deployment = f
        .engine
        .action_state()
        .get(&f.families.deployment, &"registry".into())
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            deployment.next_line,
            deployment.next_filed,
            deployment.next_licence
        ),
        (1, 1, 1)
    );
    assert!(!f.engine.action_state().reservation_lost());
}

#[test]
fn source_revalidation_after_custody_refuses_every_filing_image_and_fail_stops() {
    let mut f = Fixture::new(true);
    let before = f.economic_state();
    let count = f.engine.action_state().len();
    let request = f.request();
    let (prepared, _) = f
        .engine
        .prepare_registered_composed_authoritative_action(
            &config().initial_filing_action().unwrap(),
            &request,
            token(4),
        )
        .unwrap();
    let (_, install) = prepared.into_parts();
    f.engine
        .world
        .module_state_mut::<RollStore>()
        .unwrap()
        .audit_sources(&f.sources, &f.sources)
        .unwrap();
    assert!(f.engine.install_composed_action(install).is_err());
    f.no_filing();
    assert_eq!(f.economic_state(), before);
    assert_eq!(f.engine.action_state().len(), count);
    assert!(f.engine.action_state().reservation_lost());
}

#[path = "sale_reader/tests.rs"]
mod sale_reads;

#[inline(never)]
fn boxed_test_engine(
    world: mush_core::World,
    registry: mush_core::Registry,
    modules: Vec<Box<dyn mush_core::GameModule>>,
) -> std::result::Result<Box<mush_core::Engine>, mush_core::ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}
