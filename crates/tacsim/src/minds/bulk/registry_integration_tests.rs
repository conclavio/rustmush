#![cfg(test)]

//! Registered producer/owner integration, using the real worker for one paid
//! quantum. `install_composed_action` is core's fixture analogue of receipt
//! installation; these tests do not claim a server writer/Unknown transport run
//! or a completed Registry certification.
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, CommitToken, DefinitionRef, Engine, GOD,
    GameModule, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, Outbox,
    Registrar, Registry, World,
    account::{AccountIdentityEpoch, PersonalAccountSnapshot},
    action::{
        ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey,
        IdempotencyScope, PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
    },
    authoritative::ReplayWrite,
    composed::{ComposedInstallation, PreparedComposedTxn},
    mechanics::{
        CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
        OwnerId,
    },
};
use mush_economy::{Amount, CatalogRef, EconomyModule};
use mush_roll::{
    Envelope, ReferenceKind, Revision, RollModule, RollStore, WorkingCopy,
    commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::{
            RatingCompatibility, RatingJob, RatingJobState,
            execution::{ExecutionPhase, RatingExecution},
            protocol::{TicketId, Transition},
            registration::RegistryDefinitions,
        },
    },
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use super::{
    artifacts::{Artifacts, Key, Part},
    executor::{Executor, Ready},
    model::{Account, BulkConfig, Run, RunId, Status},
    owner::BulkStore,
    registry_tickets::Binding,
};
use crate::{
    minds::{
        authoring::RevisionRules,
        registry::{factory::Factory, integration_support::Resources},
    },
    store::TacsimStore,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Disk = BTreeMap<(String, String), Value>;
#[path = "registry_boot_tests.rs"]
mod registry_boot_tests;
const SOURCE: &str = "fn think(v,c) { stock::rules(); speed(0); this.landed = t_ms(); }";

fn module_error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
#[derive(Default)]
struct Accounts {
    values: Mutex<BTreeMap<ObjId, AccountId>>,
    epoch: AccountIdentityEpoch,
}
impl AccountIdentity for Accounts {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.values.lock().ok()?.get(&character).copied()
    }
    fn observe_distinct_accounts(
        &self,
        requester: ObjId,
        other: ObjId,
    ) -> Option<mush_core::account::AccountPairSnapshot> {
        let values = self.values.lock().ok()?;
        self.epoch.distinct_pair(
            requester,
            *values.get(&requester)?,
            other,
            *values.get(&other)?,
        )
    }
    fn capture_personal_account(&self, character: ObjId) -> Option<PersonalAccountSnapshot> {
        let values = self.values.lock().ok()?;
        self.epoch
            .personal_account(character, *values.get(&character)?)
    }
}
#[derive(Default)]
struct Registered {
    reconformance: bool,
    manifest: Option<DefinitionRef>,
    compatibility: Option<RatingCompatibility>,
    filing: Option<DefinitionRef>,
    factory: Option<Arc<Factory>>,
}
type Shared = Arc<Mutex<Registered>>;

struct Tacsim {
    real: Option<crate::TacsimRuleset>,
    resources: Resources,
    active: bool,
    shared: Shared,
    accounts: Arc<Accounts>,
}
impl GameModule for Tacsim {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if let Some(real) = &mut self.real {
            real.register(reg)?;
        } else {
            reg.state(TacsimStore::default());
            reg.state(BulkStore::new(BulkConfig::default()).map_err(module_error)?);
            reg.state(Artifacts::default());
        }
        reg.state(mush_sim::AccountOracle::new(self.accounts.clone()));
        let (manifest, compatibility, factory) = if self.real.is_some() {
            self.resources.register_after_content(reg, self.active)?
        } else {
            self.resources.register(reg, self.active)?
        };
        if self.real.is_some() {
            // Only an input driver for the real production drain; it neither
            // invents a receipt nor alters a retained owner image.
            reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
            reg.command(
                mush_core::CommandSpec {
                    name: "@recovery-pulse".into(),
                    shape: mush_core::ArgShape::Raw,
                    ..mush_core::CommandSpec::default()
                },
                Arc::new(|ctx: &mut mush_core::Ctx<'_>| {
                    super::world::drain(ctx);
                    Ok(())
                }),
            )?;
        }
        let mut shared = self.shared.lock().map_err(module_error)?;
        shared.reconformance = self.resources.reconformance();
        shared.manifest = Some(manifest);
        shared.compatibility = Some(compatibility);
        shared.factory = factory;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        if let Some(real) = &mut self.real {
            return real.on_world_loaded(ctx, data);
        }
        *ctx.state_mut::<TacsimStore>()
            .ok_or_else(|| ModError::msg("source owner"))? =
            crate::store::load(&data.kv, self.resources.config());
        ctx.state_mut::<Artifacts>()
            .ok_or_else(|| ModError::msg("artifact owner"))?
            .load(&data.kv);
        for (key, value) in &data.kv {
            if key.starts_with("tacsim:mindbulk:account:") {
                let account: Account =
                    serde_json::from_value(value.clone()).map_err(module_error)?;
                if super::publication::account_key(account.id) != *key {
                    return Err(ModError::msg("account key mismatch"));
                }
                ctx.state_mut::<BulkStore>()
                    .ok_or_else(|| ModError::msg("quota owner"))?
                    .load(
                        account.id,
                        &serde_json::to_vec(value).map_err(module_error)?,
                    )
                    .map_err(module_error)?;
            }
        }
        Ok(())
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        if let Some(real) = &mut self.real {
            return real.on_all_loaded(ctx);
        }
        crate::store::audit_mind_revisions(ctx).map_err(module_error)
    }
}
struct Roll {
    mode: CommerceMode,
    shared: Shared,
    inner: Option<RollModule>,
}
impl GameModule for Roll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let config = {
            let shared = self.shared.lock().map_err(module_error)?;
            CommerceConfig::new(
                self.mode,
                InitialFilingTerms::new(90 * 86_400, 3 * 86_400).map_err(module_error)?,
                shared
                    .manifest
                    .clone()
                    .ok_or_else(|| ModError::msg("actual manifest"))?,
                shared
                    .compatibility
                    .clone()
                    .ok_or_else(|| ModError::msg("compatibility"))?,
            )?
        };
        self.shared.lock().map_err(module_error)?.filing = Some(config.initial_filing_action()?);
        let module = RollModule::with_writers(["tacsim"]).with_commerce(config);
        let mut module = if self.mode == CommerceMode::Active {
            Resources::roll_owners(module)
        } else {
            module
        };
        module.register(reg)?;
        self.inner = Some(module);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("registered Roll"))?
            .on_world_loaded(ctx, data)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("registered Roll"))?
            .finalize_owner_composition(ctx)
    }
}
struct Economy {
    shared: Shared,
    inner: Option<EconomyModule>,
}
impl GameModule for Economy {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let issuer = mush_org::SystemPrincipalId::new("roll.registry")
            .ok_or_else(|| ModError::msg("valid Registry system principal"))?;
        let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
            issuer.clone(),
            mush_economy::AccountId(1),
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )
        .map_err(module_error)?;
        let action = self
            .shared
            .lock()
            .map_err(module_error)?
            .filing
            .clone()
            .ok_or_else(|| ModError::msg("actual filing action"))?;
        let mut module = EconomyModule::new()
            .with_title_issuers(vec![issuer])
            .with_initial_filing(policy, action);
        module.register(reg)?;
        self.inner = Some(module);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("registered Economy"))?
            .on_world_loaded(ctx, data)
    }
}
fn engine(
    resources: &Resources,
    accounts: Arc<Accounts>,
    mode: CommerceMode,
) -> TestResult<(Box<Engine>, Shared)> {
    composed_engine(resources, accounts, mode, false)
}
// Keep Engine construction in this frame; callers retain only its heap owner.
#[inline(never)]
fn composed_engine(
    resources: &Resources,
    accounts: Arc<Accounts>,
    mode: CommerceMode,
    real: bool,
) -> TestResult<(Box<Engine>, Shared)> {
    let shared = Arc::new(Mutex::new(Registered::default()));
    let mut engine = Box::new(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(Tacsim {
                real: real.then(|| crate::TacsimRuleset::with_config(resources.config())),
                resources: resources.clone(),
                active: mode == CommerceMode::Active,
                shared: shared.clone(),
                accounts: accounts.clone(),
            }),
            Box::new(Roll {
                mode,
                shared: shared.clone(),
                inner: None,
            }),
            Box::new(Economy {
                shared: shared.clone(),
                inner: None,
            }),
        ],
    )?);
    engine.set_account_identity(accounts);
    Ok((engine, shared))
}
fn load(engine: &mut Engine, disk: &Disk) -> TestResult {
    let mut typed = Vec::new();
    let mut modules = HashMap::<SmolStr, ModuleBootData>::new();
    for ((owner, key), value) in disk {
        if mush_core::action_state::is_action_state_persistence_key(key) {
            typed.push(ActionStateBootRecord::new(
                owner.as_str(),
                key.as_str(),
                value.clone(),
            ));
        } else {
            modules
                .entry(owner.clone().into())
                .or_default()
                .kv
                .push((key.clone().into(), value.clone()));
        }
    }
    engine.hydrate_action_state(typed)?;
    engine.modules_loaded(
        modules,
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    Ok(())
}
fn module_ops(disk: &mut Disk, operations: &[ModuleOp]) -> TestResult {
    for operation in operations {
        match &operation.op {
            ModuleWrite::JsonSet { key, value } => {
                disk.insert((operation.ns.to_string(), key.to_string()), value.clone());
            }
            ModuleWrite::Del { key } => {
                disk.remove(&(operation.ns.to_string(), key.to_string()));
            }
            ModuleWrite::StreamAppend { .. } => return Err("unexpected source stream".into()),
        }
    }
    Ok(())
}
fn fixture_envelope(class: &str) -> Envelope {
    Envelope {
        schema: json!({"sections":[]}),
        presets: json!({}),
        profiles: json!({"Default":{}}),
        rules: json!({}),
        declared_class: class.into(),
        mind_abi: crate::minds::MIND_ABI.into(),
    }
}
fn save_program(disk: &mut Disk, source: &str, envelope: Envelope) -> TestResult<Revision> {
    let mut roll = RollStore::default();
    let create = roll.prepare_create(
        GOD,
        "Candidate",
        WorkingCopy {
            source: source.into(),
            envelope,
        },
    )?;
    module_ops(disk, create.operations())?;
    roll.publish_with(create, |_| {})?;
    let base = roll.mind(GOD, "Candidate")?.base();
    let save = roll.prepare_save(
        GOD,
        "Candidate",
        base,
        "",
        None,
        &RevisionRules,
        &TacsimStore::default(),
    )?;
    let (change, source, reference) = save.into_parts();
    module_ops(disk, change.operations())?;
    module_ops(disk, &source.operations)?;
    roll.publish_with(change, |_| {})?;
    Ok(roll.revision(GOD, reference.key)?.clone())
}
fn invocation(number: u8, actor: ObjId, parameters: Value) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes([number; 16]);
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([number; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll")?,
                namespace: CanonicalId::new("registry_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn setup_action() -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new("initialize_commerce")?,
            1,
            json!({"format":1,"explicit_setup":true}),
        )?,
    )?)
}
fn install(engine: &mut Engine, disk: &mut Disk, prepared: PreparedComposedTxn) -> TestResult {
    let (writes, installation): (_, ComposedInstallation) = prepared.into_parts();
    // This retained union models the receipt's exact committed post-images.
    // Only core's real complete composed installation mutates the RAM owners.
    engine.install_composed_action(installation)?;
    for write in writes {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => {
                disk.insert((namespace.to_string(), key.to_string()), value);
            }
            ReplayWrite::ModuleDelete { namespace, key } => {
                disk.remove(&(namespace.to_string(), key.to_string()));
            }
            _ => return Err("Registry fixture unexpectedly changed objects or ledger".into()),
        }
    }
    Ok(())
}
struct Fixture {
    resources: Resources,
    accounts: Arc<Accounts>,
    account: AccountId,
    // This fixture and its Result move through several real boot helpers. Keep
    // the large Engine on the heap instead of nesting its by-value debug frames.
    engine: Box<Engine>,
    disk: Disk,
    definitions: RegistryDefinitions,
    factory: Arc<Factory>,
    revision: Revision,
    source: Arc<str>,
}
impl Fixture {
    fn new() -> TestResult<Self> {
        Self::with_source(SOURCE, "I")
    }
    fn with_source(source: &str, class: &str) -> TestResult<Self> {
        Self::with_program(source, fixture_envelope(class))
    }
    // This accepts source/envelope input only. The ordinary registered Save,
    // Registry worker and complete certificate publication remain mandatory.
    fn with_program(source: &str, envelope: Envelope) -> TestResult<Self> {
        let resources = Resources::capture()?;
        let account = AccountId::mint("registry-engine-fixture", 1)?;
        let accounts = Arc::new(Accounts::default());
        accounts
            .values
            .lock()
            .map_err(module_error)?
            .insert(GOD, account);
        let (mut setup, setup_shared) =
            engine(&resources, accounts.clone(), CommerceMode::Initialize)?;
        let mut disk = Disk::new();
        load(&mut setup, &disk)?;
        let (prepared, _) = setup.prepare_registered_composed_authoritative_action(
            &setup_action()?,
            &invocation(1, GOD, json!({}))?,
            CommitToken::from_bytes([1; 16]),
        )?;
        assert!(setup.action_state().is_empty());
        install(&mut setup, &mut disk, prepared)?;
        // Initialize installs only the deployment rows. The real Registry
        // producer remains unavailable until Active boot and its final audits.
        let setup_definitions = RegistryDefinitions::new(
            setup_shared
                .lock()
                .map_err(module_error)?
                .manifest
                .clone()
                .ok_or("manifest")?,
        )?;
        let admit = setup_definitions
            .actions()
            .iter()
            .find(|action| action.key().id.as_str() == Transition::Admit.action_id())
            .ok_or("Registry admission definition")?;
        assert!(matches!(
            setup.prepare_registered_composed_authoritative_action(
                admit,
                &invocation(2, GOD, json!({}))?,
                CommitToken::from_bytes([2; 16]),
            ),
            Err(mush_core::action::ActionProviderPrepareError::Lookup(_))
        ));
        let revision = save_program(&mut disk, source, envelope)?;
        let (mut engine, shared) = engine(&resources, accounts.clone(), CommerceMode::Active)?;
        load(&mut engine, &disk)?;
        // Readiness is exercised by actual admission below, after the real
        // all-loaded composition audit; no test-only readiness access is used.
        let shared = shared.lock().map_err(module_error)?;
        let definitions = RegistryDefinitions::new(shared.manifest.clone().ok_or("manifest")?)?;
        let factory = shared.factory.clone().ok_or("registered factory")?;
        Ok(Self {
            resources,
            accounts,
            account,
            engine,
            disk,
            definitions,
            factory,
            revision,
            source: Arc::from(source),
        })
    }
    fn artifacts(&self) -> TestResult<&Artifacts> {
        self.engine
            .world
            .module_state::<Artifacts>()
            .ok_or_else(|| "artifacts".into())
    }
    fn artifacts_mut(&mut self) -> TestResult<&mut Artifacts> {
        self.engine
            .world
            .module_state_mut::<Artifacts>()
            .ok_or_else(|| "artifacts".into())
    }
    fn quota(&self) -> TestResult<&BulkStore> {
        self.engine
            .world
            .module_state::<BulkStore>()
            .ok_or_else(|| "quota".into())
    }
    fn roots(&self) -> TestResult<Vec<String>> {
        Ok(self
            .engine
            .world
            .module_state::<RollStore>()
            .ok_or("Roll")?
            .reference_keys("roll", ReferenceKind::Commercial)
            .into_iter()
            .map(|k| k.record)
            .collect())
    }
    fn action(&self, transition: Transition) -> TestResult<DefinitionRef> {
        self.definitions
            .actions()
            .iter()
            .find(|r| r.key().id.as_str() == transition.action_id())
            .cloned()
            .ok_or_else(|| "actual registered action".into())
    }
    fn prepare(
        &mut self,
        transition: Transition,
        ticket: TicketId,
        job: CommitToken,
        operation: u8,
    ) -> TestResult<PreparedComposedTxn> {
        let params = if transition == Transition::Admit {
            let r = self.revision.reference();
            json!({"ticket":ticket.get(),"mind":r.key.mind,"revision":r.key.number,"revision_digest":r.digest})
        } else {
            json!({"ticket":ticket.get(),"job":job})
        };
        let action = self.action(transition)?;
        let invocation = invocation(operation, GOD, params)?;
        Ok(self
            .engine
            .prepare_registered_composed_authoritative_action(
                &action,
                &invocation,
                CommitToken::from_bytes([operation; 16]),
            )?
            .0)
    }
    fn ack(
        &mut self,
        prepared: PreparedComposedTxn,
        ticket: TicketId,
        job: CommitToken,
        operation: u8,
        transition: Transition,
    ) -> TestResult {
        install(&mut self.engine, &mut self.disk, prepared)?;
        let ack = self
            .artifacts_mut()?
            .registry
            .take_acknowledged()
            .ok_or("actual artifact Ack")?;
        assert_eq!(ack.ticket, ticket);
        assert_eq!(ack.job, job);
        assert_eq!(ack.operation, CommitToken::from_bytes([operation; 16]));
        assert_eq!(ack.transition, transition);
        assert!(self.artifacts()?.registry.get(ticket).is_err());
        Ok(())
    }
    fn retained<T: serde::de::DeserializeOwned>(&self, family: &str) -> TestResult<T> {
        let values = self
            .disk
            .iter()
            .filter(|((owner, key), value)| {
                owner == "roll"
                    && mush_core::action_state::is_action_state_persistence_key(key)
                    && value
                        .get("family")
                        .and_then(|f| f.get("id"))
                        .and_then(Value::as_str)
                        == Some(family)
            })
            .map(|(_, v)| v.get("value").ok_or("typed envelope"))
            .collect::<Result<Vec<_>, _>>()?;
        let [value] = values.as_slice() else {
            return Err(format!("one actual {family} row").into());
        };
        Ok(serde_json::from_value((*value).clone())?)
    }
    fn run(&self, binding: Binding) -> TestResult<Run> {
        Ok(self
            .quota()?
            .run(binding.account, binding.character, binding.run)?
            .clone())
    }
    fn admitted(&mut self) -> TestResult<(Binding, Ready)> {
        let source = self
            .engine
            .world
            .module_state::<TacsimStore>()
            .ok_or("sources")?;
        let source = mush_roll::SourceAdapter::read_source(source, self.revision.source())?;
        assert_eq!(source, self.source.as_ref());
        let prepared = self.factory.prepare(
            self.account,
            GOD,
            1,
            self.revision.clone(),
            Arc::from(source),
        )?;
        let start = prepared.data.start.descriptor().sha256;
        let ticket = self.artifacts_mut()?.registry.admit(prepared.data)?;
        let captured = self.artifacts()?.registry.get(ticket)?;
        let job = CommitToken::from_bytes([2; 16]);
        let binding = Binding {
            account: self.account,
            character: GOD,
            run: RunId(1),
            job,
        };
        let root_before = self.roots()?;
        let revision_before = self.engine.action_state().revision();
        let change = self.prepare(Transition::Admit, ticket, job, 2)?;
        assert_eq!(self.engine.action_state().revision(), revision_before);
        assert!(self.quota()?.account_ids().is_empty());
        assert_eq!(self.roots()?, root_before);
        assert!(
            self.artifacts()?
                .get(Key {
                    account: self.account,
                    run: RunId(1),
                    part: Part::Start
                })
                .is_err()
        );
        self.artifacts()?.registry.check(ticket, &captured)?;
        assert!(self.artifacts()?.registry.peek_acknowledged().is_none());
        self.ack(change, ticket, job, 2, Transition::Admit)?;
        let run = self.run(binding)?;
        assert_eq!(run.registry_job, Some(job));
        assert_eq!(run.status, Status::Queued);
        assert_eq!(run.charged_unit_ms, 0);
        assert!(run.flight.is_none());
        assert!(self.roots()?.contains(&format!("v1/rating_job/{job}")));
        assert!(self.roots()?.contains(&format!("v1/rating_pending/{job}")));
        assert_eq!(
            self.roots()?.len(),
            2,
            "only actual job and pending obligations"
        );
        assert_eq!(
            self.disk
                .keys()
                .filter(|(_, key)| key.starts_with("action-state/capacity/v1/"))
                .count(),
            1,
            "actual registered admission reserves its terminal typed writes"
        );
        let row: RatingJob = self.retained("rating_jobs")?;
        assert_eq!(row.job(), job);
        assert!(matches!(row.state(), RatingJobState::Queued {}));
        let execution: RatingExecution = self.retained("rating_executions")?;
        assert_eq!(execution.job(), row.job());
        assert_eq!(execution.admission().subject, row.subject().digest()?);
        execution.digest()?;
        assert_eq!(execution.observation().phase, ExecutionPhase::Queued);
        assert_eq!(
            self.artifacts()?
                .get(Key {
                    account: self.account,
                    run: RunId(1),
                    part: Part::Start
                })?
                .artifact
                .sha256,
            start
        );
        assert!(
            self.prepare(Transition::Admit, ticket, job, 3).is_err(),
            "consumed admission cannot replay"
        );
        Ok((binding, Ready::registry(binding, start, prepared.machine)))
    }
    fn grant(
        &mut self,
        binding: Binding,
        ready: &Ready,
        operation: u8,
    ) -> TestResult<super::owner::Lease> {
        let ticket = self
            .artifacts_mut()?
            .registry
            .grant(binding, ready.next_grant_ms())?;
        let before = self.run(binding)?;
        let roots = self.roots()?;
        let revision = self.engine.action_state().revision();
        let prepared = self.prepare(Transition::Grant, ticket, binding.job, operation)?;
        assert_eq!(self.run(binding)?, before);
        assert_eq!(self.roots()?, roots);
        assert_eq!(self.engine.action_state().revision(), revision);
        assert!(
            self.engine
                .world
                .module_state_mut::<BulkStore>()
                .ok_or("quota")?
                .take_registry_lease(CommitToken::from_bytes([operation; 16]))
                .is_none()
        );
        self.ack(prepared, ticket, binding.job, operation, Transition::Grant)?;
        let lease = self
            .engine
            .world
            .module_state_mut::<BulkStore>()
            .ok_or("quota")?
            .take_registry_lease(CommitToken::from_bytes([operation; 16]))
            .ok_or("post-install paid lease")?;
        assert!(self.run(binding)?.flight.is_some());
        assert!(lease.maximum_sim_ms() <= ready.next_grant_ms());
        assert!(
            self.prepare(Transition::Grant, ticket, binding.job, operation + 1)
                .is_err()
        );
        Ok(lease)
    }
    fn restart_terminal(&self, binding: Binding, expected: Status) -> TestResult {
        let (mut reboot, _) = engine(&self.resources, self.accounts.clone(), CommerceMode::Active)?;
        load(&mut reboot, &self.disk)?;
        // load runs the real final owner audit before these durable joins.
        let run = reboot
            .world
            .module_state::<BulkStore>()
            .ok_or("quota")?
            .run(binding.account, binding.character, binding.run)?;
        assert_eq!(run, &self.run(binding)?);
        assert_eq!(run.status, expected);
        assert!(run.flight.is_none());
        assert!(
            reboot
                .world
                .module_state::<Artifacts>()
                .ok_or("artifacts")?
                .registry
                .peek_acknowledged()
                .is_none(),
            "volatile Ack does not replay"
        );
        Ok(())
    }
}

#[test]
fn registered_admit_grant_real_quantum_settle_cancel_and_restart() -> TestResult {
    let mut f = Fixture::new()?;
    let (binding, ready) = f.admitted()?;
    let lease = f.grant(binding, &ready, 4)?;
    let allowance = lease.maximum_sim_ms() * lease.units();
    let executor = Executor::with_wake(&BulkConfig::default(), None)?;
    executor
        .submit(lease, ready)
        .map_err(|(error, _, _)| error)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut done = loop {
        if let Some(done) = executor.poll() {
            break done;
        }
        if Instant::now() >= deadline {
            return Err("bounded actual Registry worker did not return".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(done.receipt.actual_unit_ms > 0);
    assert!(done.receipt.actual_unit_ms <= allowance);
    assert!(
        done.receipt.terminal.is_none(),
        "one paid case quantum is no certification"
    );
    assert!(done.receipt.result.is_none());
    assert!(done.artifact.is_none());
    let actual = done.receipt.actual_unit_ms;
    let progress = done.receipt.registry.as_ref().ok_or("real case progress")?;
    assert!(progress.cursor.index > 0 || progress.cursor.simulated_ms > 0);
    let settlement = done
        .registry_settlement
        .take()
        .ok_or("executor prepared Registry settlement")??;
    let ready = done.ready.take().ok_or("owned continuation")?;
    let ticket = f
        .artifacts_mut()?
        .registry
        .insert_settlement(settlement)
        .map_err(|(error, _)| error)?;
    let before = f.run(binding)?;
    let prepared = f.prepare(Transition::Settle, ticket, binding.job, 6)?;
    assert_eq!(
        f.run(binding)?,
        before,
        "worker completion alone does not refund/install"
    );
    f.ack(prepared, ticket, binding.job, 6, Transition::Settle)?;
    let settled = f.run(binding)?;
    assert!(settled.flight.is_none());
    assert_eq!(settled.charged_unit_ms, actual);
    assert_eq!(settled.registry, Some(progress.cursor));
    let ticket = f.artifacts_mut()?.registry.cancel(binding)?;
    let change = f.prepare(Transition::Cancel, ticket, binding.job, 8)?;
    assert_eq!(f.run(binding)?, settled);
    assert!(
        f.roots()?
            .contains(&format!("v1/rating_pending/{}", binding.job))
    );
    f.ack(change, ticket, binding.job, 8, Transition::Cancel)?;
    drop(ready); // The exact Machine is retained until its terminal installation.
    assert_eq!(f.run(binding)?.status, Status::Cancelled);
    assert_eq!(f.run(binding)?.charged_unit_ms, actual);
    assert!(
        !f.roots()?
            .contains(&format!("v1/rating_pending/{}", binding.job))
    );
    assert!(
        f.roots()?
            .contains(&format!("v1/rating_job/{}", binding.job))
    );
    assert!(
        f.disk
            .keys()
            .all(|(_, k)| !k.contains("rating_receipts") && !k.contains("rating_pages"))
    );
    assert_eq!(f.roots()?.len(), 1);
    assert!(
        f.disk
            .keys()
            .all(|(_, key)| !key.starts_with("action-state/capacity/v1/"))
    );
    let job: RatingJob = f.retained("rating_jobs")?;
    assert!(matches!(job.state(), RatingJobState::Cancelled {}));
    let execution: RatingExecution = f.retained("rating_executions")?;
    assert_eq!(execution.job(), job.job());
    assert_eq!(execution.admission().subject, job.subject().digest()?);
    execution.digest()?;
    assert_eq!(execution.observation().phase, ExecutionPhase::Cancelled);
    f.restart_terminal(binding, Status::Cancelled)?;
    Ok(())
}

#[test]
fn registered_paid_flight_recovers_only_exact_stopped_ticket_without_refund() -> TestResult {
    let mut f = Fixture::new()?;
    let (binding, ready) = f.admitted()?;
    let lease = f.grant(binding, &ready, 4)?;
    let paid = f.run(binding)?;
    let allowance = lease.maximum_sim_ms() * lease.units();
    assert_eq!(paid.charged_unit_ms, allowance);
    drop(lease);
    drop(ready); // This fixture owns and stops every possible worker/continuation.
    let ticket = registry_boot_tests::cold_restart(&mut f, binding)?;
    let captured = f.artifacts()?.registry.get(ticket)?;
    assert!(
        f.prepare(Transition::Cancel, ticket, binding.job, 6)
            .is_err(),
        "wrong transition cannot consume recovery"
    );
    assert!(
        f.prepare(
            Transition::Recover,
            ticket,
            CommitToken::from_bytes([99; 16]),
            7
        )
        .is_err(),
        "crossed job refuses"
    );
    assert_eq!(f.run(binding)?, paid);
    f.artifacts()?.registry.check(ticket, &captured)?;
    let roots = f.roots()?;
    let change = f.prepare(Transition::Recover, ticket, binding.job, 8)?;
    assert_eq!(f.run(binding)?, paid);
    assert_eq!(f.roots()?, roots);
    assert!(f.artifacts()?.registry.peek_acknowledged().is_none());
    f.ack(change, ticket, binding.job, 8, Transition::Recover)?;
    let recovered = f.run(binding)?;
    assert_eq!(recovered.status, Status::Interrupted);
    assert_eq!(
        recovered.charged_unit_ms, allowance,
        "unknown stopped progress keeps the full prepaid charge"
    );
    assert!(recovered.flight.is_none());
    assert!(recovered.result.is_none());
    assert!(
        !f.roots()?
            .contains(&format!("v1/rating_pending/{}", binding.job))
    );
    assert!(
        f.roots()?
            .contains(&format!("v1/rating_job/{}", binding.job))
    );
    assert!(
        f.prepare(Transition::Recover, ticket, binding.job, 9)
            .is_err(),
        "consumed recovery cannot replay"
    );
    assert_eq!(f.roots()?.len(), 1);
    assert!(
        f.disk
            .keys()
            .all(|(_, key)| !key.starts_with("action-state/capacity/v1/"))
    );
    f.restart_terminal(binding, Status::Interrupted)?;
    Ok(())
}

#[test]
fn registered_admission_account_epoch_change_refuses_before_every_owner_image() -> TestResult {
    let mut f = Fixture::new()?;
    let prepared = f
        .factory
        .prepare(f.account, GOD, 1, f.revision.clone(), Arc::from(SOURCE))?;
    let ticket = f.artifacts_mut()?.registry.admit(prepared.data)?;
    let captured = f.artifacts()?.registry.get(ticket)?;
    let roots = f.roots()?;
    let revision = f.engine.action_state().revision();
    let disk = f.disk.clone();
    let job = CommitToken::from_bytes([2; 16]);
    let change = f.prepare(Transition::Admit, ticket, job, 2)?;
    {
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        values.insert(GOD, AccountId::mint("changed-registry-account", 2)?);
    }
    let (_, installation) = change.into_parts();
    assert!(f.engine.install_composed_action(installation).is_err());
    assert_eq!(f.engine.action_state().revision(), revision);
    assert!(f.quota()?.account_ids().is_empty());
    assert_eq!(f.roots()?, roots);
    assert!(
        f.artifacts()?
            .get(Key {
                account: f.account,
                run: RunId(1),
                part: Part::Start
            })
            .is_err()
    );
    f.artifacts()?.registry.check(ticket, &captured)?;
    assert!(f.artifacts()?.registry.peek_acknowledged().is_none());
    assert_eq!(f.disk, disk);
    drop(prepared.machine);
    // Failed binding requires restart; replaying only actual durable rows
    // proves no typed admission or owner image escaped that refusal.
    let (mut reboot, _) = engine(&f.resources, f.accounts.clone(), CommerceMode::Active)?;
    load(&mut reboot, &disk)?;
    // Real final owner audit succeeded on exactly the pre-refusal disk image.
    assert!(
        reboot
            .world
            .module_state::<BulkStore>()
            .ok_or("quota")?
            .account_ids()
            .is_empty()
    );
    Ok(())
}

#[path = "original_purchase_integration_tests.rs"]
mod original_purchase;

#[test]
fn real_measured_maximum_class_rejection_publishes_and_survives_private_history_eviction()
-> TestResult {
    use mush_roll::commerce::rating::RatingPage;
    use mush_roll::commerce::rating::nonconformance::{
        MeasuredNonconformance, RatingNonconformance,
    };
    // Bound the inline fixture/result carried through this default-stack test.
    // The original real boot and worker assertions below remain unchanged.
    assert!(std::mem::size_of::<Fixture>() <= 16 * 1024);
    assert!(std::mem::size_of::<TestResult<Fixture>>() <= 16 * 1024);
    let diagnostic = rejection_diagnostic::Phases::default();
    let _tracing = tracing::subscriber::set_default(diagnostic.clone());
    for declared in ["I", "III"] {
        let mut f = Fixture::with_source("fn think(v,c) { while true {} }", declared)?;
        let (binding, ready) = f.admitted()?;
        let mut ready = Some(ready);
        let executor = Executor::with_wake(&BulkConfig::default(), None)?;
        let mut actual = 0_u64;
        let mut complete = false;
        for index in 0_u8..16 {
            let current = ready.take().ok_or("exact continuation")?;
            let operation = 4 + index * 4;
            let lease = f.grant(binding, &current, operation)?;
            executor
                .submit(lease, current)
                .map_err(|(error, _, _)| error)?;
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut done = loop {
                if let Some(done) = executor.poll() {
                    break done;
                }
                if Instant::now() >= deadline {
                    return Err("bounded real rejection worker did not return".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            actual = actual
                .checked_add(done.receipt.actual_unit_ms)
                .ok_or("charge")?;
            let terminal = done.receipt.terminal;
            if terminal.is_some() {
                assert_eq!(terminal, Some(Status::Complete));
                assert_eq!(
                    done.receipt
                        .registry
                        .as_ref()
                        .ok_or("measured progress")?
                        .ending,
                    super::registry_budget::RegistryEnding::Rejected
                );
                assert!(done.receipt.result.is_some());
            }
            ready = done.ready.take();
            let settlement = done
                .registry_settlement
                .take()
                .ok_or("actual measured settlement")??;
            let ticket = f
                .artifacts_mut()?
                .registry
                .insert_settlement(settlement)
                .map_err(|(error, _)| error)?;
            let before = f.run(binding)?;
            diagnostic.clear();
            let prepared = f
                .prepare(Transition::Settle, ticket, binding.job, operation + 2)
                .map_err(|_| diagnostic.error(index, terminal.is_some()))?;
            assert_eq!(f.run(binding)?, before);
            assert!(
                f.retained::<RatingNonconformance>("rating_nonconformance")
                    .is_err()
            );
            f.ack(
                prepared,
                ticket,
                binding.job,
                operation + 2,
                Transition::Settle,
            )?;
            assert_eq!(f.run(binding)?.charged_unit_ms, actual);
            if terminal.is_some() {
                complete = true;
                break;
            }
        }
        assert!(
            complete,
            "real measured rejection must finish within bounded quanta"
        );
        let header: RatingNonconformance = f.retained("rating_nonconformance")?;
        let page: RatingPage = f.retained("rating_pages")?;
        header.verify_pages(std::slice::from_ref(&page))?;
        let measured = MeasuredNonconformance::maximum_class_budget(
            header.subject().clone(),
            header.compatibility().clone(),
            header.plan(),
            vec![page.content().clone()],
        )?;
        let execution: RatingExecution = f.retained("rating_executions")?;
        assert_eq!(execution.observation().result, Some(measured.artifact()?));
        assert!(execution.admission().measured_rejection);
        let job: RatingJob = f.retained("rating_jobs")?;
        assert!(
            matches!(job.state(),RatingJobState::Nonconforming { receipt } if *receipt==header.reference()?)
        );
        for family in ["rating_receipts", "rating_completions", "rating_status"] {
            assert!(f.retained::<Value>(family).is_err());
        }
        assert_eq!(f.roots()?, vec![format!("v1/rating_job/{}", binding.job)]);
        assert!(
            f.disk
                .keys()
                .all(|(_, key)| !key.starts_with("action-state/capacity/v1/"))
        );
        f.restart_terminal(binding, Status::Complete)?;
        for missing in ["rating_nonconformance", "rating_pages"] {
            let mut broken = f.disk.clone();
            broken.retain(|_, value| {
                value
                    .get("family")
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str)
                    != Some(missing)
            });
            let (mut reboot, _) = engine(&f.resources, f.accounts.clone(), CommerceMode::Active)?;
            assert!(load(&mut reboot, &broken).is_err());
        }
        let mut evicted = f.disk.clone();
        evicted.retain(|(owner, key), _| owner != "tacsim" || !key.starts_with("tacsim:mindbulk:"));
        let (mut reboot, _) = engine(&f.resources, f.accounts.clone(), CommerceMode::Active)?;
        load(&mut reboot, &evicted)?;
    }
    Ok(())
}

/// A test-local subscriber captures only one fixed phase field from the closed
/// producer. No arbitrary diagnostic payload or source is formatted.
mod rejection_diagnostic {
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    pub(super) struct Phases(Arc<Mutex<Option<String>>>);
    impl Phases {
        pub(super) fn clear(&self) {
            if let Ok(mut value) = self.0.lock() {
                *value = None;
            }
        }
        pub(super) fn error(&self, quantum: u8, terminal: bool) -> String {
            let phase = self
                .0
                .lock()
                .ok()
                .and_then(|v| v.clone())
                .unwrap_or_else(|| "unknown".into());
            format!(
                "Registry rejection refusal phase={phase} quantum={quantum} terminal={terminal}"
            )
        }
    }
    struct PhaseVisitor(Option<String>);
    impl tracing::field::Visit for PhaseVisitor {
        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "stage"
                && value.len() <= 32
                && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            {
                self.0 = Some(value.to_owned());
            }
        }
    }
    impl tracing::Subscriber for Phases {
        fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
            m.target() == "registry_preparation"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if event.metadata().target() != "registry_preparation" {
                return;
            }
            let mut visitor = PhaseVisitor(None);
            event.record(&mut visitor);
            if let Ok(mut value) = self.0.lock() {
                *value = visitor.0;
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
}
