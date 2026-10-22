#![cfg(test)]

//! Real Origin issuance, supplier offer, buyer payment, exact Inventory hold,
//! native K production and Core output. The unrelated Roll filing action is
//! registered only as a refusing prerequisite; it creates no filing or grant.
use super::*;
use crate::core_production::integration::{
    self as factory, CoreFactoryConfig, CoreRecipeAuthority, PublishedCoreRecipe,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, PrepareCtx,
};
use mush_economy::{
    AccountId, Amount, CatalogRef, InstrumentId,
    core_integration::{CorePaymentPolicy, CoreTerminalResult, ScopedCoreAccount},
    registry_filing::InitialFilingPolicy,
};
use mush_inventory::core_input::CoreInputBinding;
#[derive(Clone)]
struct Recipe {
    revision: u32,
    units: u64,
}
impl Recipe {
    fn descriptor(&self) -> Result<DefinitionDescriptor> {
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new("recipe")?,
            DefinitionId::new("core_integrate")?,
            1,
            json!({"format":1,"fixture_revision":self.revision,"work_units":self.units}),
        )?)
    }
    fn reference(&self) -> Result<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new()
            .register(OwnerId::new("tacsim")?, self.descriptor()?)?)
    }
    fn pack(&self) -> Result<crate::workorder::RecipeRef> {
        Ok(crate::workorder::RecipeRef::new(
            id("factory_fixture")?,
            self.revision,
            id("core_integrate")?,
            DefinitionDigest::for_canonical_value(
                "factory.fixture.pack",
                &json!({"revision":self.revision,"units":self.units}),
            )?,
        )?)
    }
}
impl CoreRecipeAuthority for Recipe {
    fn resolve(&self, _: &mush_core::World, class: DieClass) -> Option<PublishedCoreRecipe> {
        PublishedCoreRecipe::new(self.pack().ok()?, self.reference().ok()?, class, self.units).ok()
    }
    fn published_digest(
        &self,
        _: &mush_core::World,
        recipe: &crate::workorder::RecipeRef,
    ) -> Option<DefinitionDigest> {
        (recipe == &self.pack().ok()?).then(|| recipe.digest())
    }
}
struct FactoryContent {
    grades: [Grade; 3],
    recipe: Recipe,
    refits: Option<refit_rules::Rules>,
}
impl GameModule for FactoryContent {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        Content(self.grades).register(reg)?;
        let d = self
            .recipe
            .descriptor()
            .map_err(|e| ModError::msg(e.to_string()))?;
        let mut recipes = vec![d];
        if let Some(r) = &self.refits {
            recipes.push(r.recipe().map_err(|e| ModError::msg(e.to_string()))?);
            r.register(reg)?;
        }
        provider::exact(reg, "recipe", &recipes)?;
        for d in recipes {
            reg.definition(d)?;
        }
        delivery_tests::register_controls(reg)?;
        Ok(())
    }
}
fn unused_filing() -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action")?,
        DefinitionId::new("initial_filing")?,
        1,
        json!({"fixture":"unavailable_initial_filing"}),
    )?)
}
fn filing_ref() -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, unused_filing()?)?)
}
struct ClosedRoll;
struct Refuses;
impl ActionProvider for Refuses {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "Unrelated fixture filing is unavailable.",
        )
        .expect("bounded error"))
    }
    fn prepare_authoritative_with_context(
        &self,
        i: &ActionInvocation,
        _: &mut ActionTxn<'_>,
        _: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(i)?;
        Err(ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "Unrelated fixture filing is unavailable.",
        )
        .expect("bounded error"))
    }
}
impl GameModule for ClosedRoll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let d = unused_filing().map_err(|e| ModError::msg(e.to_string()))?;
        provider::exact(reg, "action", std::slice::from_ref(&d))?;
        let a = reg.definition(d)?;
        reg.action_provider(a, Refuses)
    }
}
#[derive(Clone)]
struct Composition {
    origin: OriginPolicy,
    factory: CoreFactoryConfig,
    filing: InitialFilingPolicy,
    recipe: Recipe,
    refits: Option<refit_rules::Rules>,
}
impl Composition {
    fn modules(
        &self,
        mode: CoreOriginMode,
        enabled: bool,
        reverse: bool,
    ) -> Result<Vec<Box<dyn GameModule>>> {
        let cell: InventoryHandleCell = Arc::default();
        let mut inventory = InventoryModule::new()
            .with_quantity_delegates(["industry"])
            .with_handle_cell(cell.clone());
        let mut industry = IndustryModule::new()
            .with_inventory(cell)
            .with_core_origins(CoreOriginConfig::new(
                self.origin.clone(),
                CoreOriginMode::Active,
            )?);
        let mut economy =
            EconomyModule::new().with_title_issuers(vec![self.filing.issuer().clone()]);
        let mut tactics = None;
        if enabled {
            let mut config = self.factory.clone();
            config.mode = mode;
            if let Some(r) = &self.refits {
                let fitting = r
                    .config(&config, mode)
                    .map_err(|e| format!("factory fixture refit configuration: {e}"))?;
                tactics = Some(
                    r.tactics(&fitting)
                        .map_err(|e| format!("factory fixture tactics configuration: {e}"))?,
                );
                config = config
                    .with_refits(fitting)
                    .map_err(|e| format!("factory fixture refit binding: {e}"))?;
                economy =
                    economy.with_core_refit(config.economy_refit_config(&self.origin).map_err(
                        |e| format!("factory fixture refit economy configuration: {e}"),
                    )?);
            }
            inventory = inventory.with_core_inputs(CoreInputBinding::new(
                config.action(factory::ACCEPT)?,
                config.action(factory::COMPLETE)?,
                config.action(factory::CANCEL)?,
            )?);
            economy = economy
                .with_initial_filing(self.filing.clone(), filing_ref()?)
                .with_core_integration(
                    config
                        .economy_config(&self.origin)
                        .map_err(|e| format!("factory fixture economy configuration: {e}"))?,
                );
            industry = industry.with_core_factory(config);
        }
        let mut modules: Vec<Box<dyn GameModule>> = vec![
            Box::new(PolicyOrg(OrgModule::new())),
            Box::new(economy),
            Box::new(inventory),
            Box::new(FactoryContent {
                grades: self.origin.grades,
                recipe: self.recipe.clone(),
                refits: self.refits.clone(),
            }),
            Box::new(ClosedRoll),
            Box::new(industry),
        ];
        if let Some(tactics) = tactics {
            modules.push(Box::new(mush_sim::SimulationModule::new()));
            modules.push(Box::new(tactics));
        }
        if reverse {
            modules.reverse();
            // Inventory publishes its shared handle during registration. Vary
            // the other owners while preserving this required dependency.
            let inventory = modules
                .iter()
                .position(|m| m.name() == "inventory")
                .unwrap();
            let industry = modules.iter().position(|m| m.name() == "industry").unwrap();
            if inventory > industry {
                let module = modules.remove(inventory);
                modules.insert(industry, module);
            }
        }
        Ok(modules)
    }
    fn changed_recipe(&self) -> Result<Self> {
        let mut next = self.clone();
        next.recipe.revision += 1;
        next.factory.payment.recipe = next.recipe.reference()?;
        next.factory.recipes = Arc::new(next.recipe.clone());
        Ok(next)
    }
}
struct FactoryRig {
    r: Box<Rig>,
    composition: Composition,
    buyer: ObjId,
    payer: AccountId,
    certificate: DieCertificate,
    reverse: bool,
}
impl FactoryRig {
    fn new(reverse: bool) -> Result<Box<Self>> {
        Self::new_delivery(reverse, false)
    }
    fn new_delivery(reverse: bool, delivery: bool) -> Result<Box<Self>> {
        Self::new_options(reverse, delivery, false)
    }
    #[inline(never)]
    fn new_options(reverse: bool, delivery: bool, refits: bool) -> Result<Box<Self>> {
        let mut r = Rig::new(true)?;
        let operator = r.policy.agents[1].actor;
        let certificate = r.issue(operator, 2, DieClass::I)?;
        let buyer = r.w.spawn_player("Core buyer");
        let factory_object =
            r.w.world_mut()
                .create(ObjKind::Thing, "Core factory", GOD, None)?;
        let container =
            r.w.world_mut()
                .create(ObjKind::Thing, "Core factory output", GOD, None)?;
        command(&mut r.w, "@instrument/register cr=2")?;
        command(&mut r.w, "@org/scope create Importer=core-factory")?;
        let integrator_scope =
            r.w.world()
                .module_state::<OrgStore>()
                .ok_or("org")?
                .state()
                .scopes_in(r.policy.importer)
                .find(|s| s.name.as_str() == "core-factory")
                .ok_or("factory scope")?
                .id;
        for scope in [r.policy.scope, integrator_scope] {
            command(
                &mut r.w,
                &format!("@account/open org:Importer/{}=cr", scope.0),
            )?;
        }
        command(&mut r.w, &format!("@account/open #{}=cr", buyer.0))?;
        command(&mut r.w, "@account/system core.fixture=cr")?;
        let snapshot =
            r.w.world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot();
        let account = |scope| {
            snapshot
                .accounts()
                .find(|a| a.scope == Some(scope))
                .map(|a| a.id)
                .ok_or("scoped account")
        };
        let supplier = account(r.policy.scope)?;
        let integrator = account(integrator_scope)?;
        let payer = snapshot
            .accounts()
            .find(|a| a.owner == PrincipalId::Object(buyer))
            .ok_or("buyer account")?
            .id;
        let faucet = snapshot
            .accounts()
            .find(|a| matches!(&a.owner,PrincipalId::System(s)if s.as_str()=="core.fixture"))
            .ok_or("faucet")?
            .id;
        command(
            &mut r.w,
            &format!("@econ/mint {}={} 1000000", faucet.0, payer.0),
        )?;
        command(
            &mut r.w,
            &format!(
                "@facility/open {}=2 core_integration{}",
                factory_object.0,
                if refits { " core_refit" } else { "" }
            ),
        )?;
        command(
            &mut r.w,
            &format!("@facility/owner {}=org:Importer", factory_object.0),
        )?;
        command(
            &mut r.w,
            &format!(
                "@org/grant #{}=Importer industry.core.integrate scope=#{}",
                operator.0, integrator_scope.0
            ),
        )?;
        r.flush();
        let recipe = Recipe {
            revision: 1,
            units: 2,
        };
        let issuer = mush_org::SystemPrincipalId::new("core.factory").ok_or("issuer")?;
        let catalog = |class: &str| CatalogRef {
            ruleset: "fixture".into(),
            code: format!("core_{class}").into(),
        };
        let filing = InitialFilingPolicy::new(
            issuer.clone(),
            integrator,
            Amount::from_minor(0),
            catalog("unused"),
        )?;
        let economy_owner = DefinitionRegistryBuilder::new().register(
            OwnerId::new("economy")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")?,
                DefinitionId::new("initial_filing")?,
                1,
                filing.definition_payload(),
            )?,
        )?;
        let policy = CorePaymentPolicy {
            delivery: delivery
                .then_some(mush_economy::core_integration::CoreDeliveryPromise::PersonalCustodyV1),
            issuer,
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            supplier: ScopedCoreAccount {
                account: supplier,
                organization: r.policy.importer,
                scope: r.policy.scope,
            },
            integrator: ScopedCoreAccount {
                account: integrator,
                organization: r.policy.importer,
                scope: integrator_scope,
            },
            supplier_stock: [r.policy.agents[0].stock, r.policy.agents[1].stock],
            factory: factory_object,
            output_container: container,
            recipe: recipe.reference()?,
            catalogs: [catalog("i"), catalog("ii"), catalog("iii")],
        };
        let factory = CoreFactoryConfig::new(
            CoreOriginMode::Initialize,
            policy,
            Arc::new(recipe.clone()),
            economy_owner,
        )?;
        let composition = Composition {
            origin: r.policy.clone(),
            factory,
            filing,
            recipe,
            refits: refits.then(refit_rules::Rules::new).transpose()?,
        };
        let mut f = Box::new(Self {
            r,
            composition,
            buyer,
            payer,
            certificate,
            reverse,
        });
        f.restart(CoreOriginMode::Initialize)
            .map_err(|e| format!("factory fixture initialization boot: {e}"))?;
        f.commit(0, GOD, 10, json!({}))
            .map_err(|e| format!("factory fixture initialization action: {e}"))?;
        // Initialize only publishes the two actual empty deployment images.
        check!(f.prepare(1, operator, 11, f.offer_parameters()).is_err());
        if refits {
            f.initialize_refit()
                .map_err(|e| format!("factory fixture refit initialization action: {e}"))?;
        }
        f.restart(CoreOriginMode::Active)
            .map_err(|e| format!("factory fixture active boot: {e}"))?;
        Ok(f)
    }
    fn initialize_refit(&mut self) -> Result {
        let action = self
            .r
            .runtime
            .refit
            .as_ref()
            .ok_or("refit runtime")?
            .actions
            .first()
            .cloned()
            .ok_or("refit initialization action")?;
        let inv = invocation(GOD, 12, json!({}));
        let prepared = self
            .r
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&action, &inv, token(12))?;
        self.stage_action(action, &inv, 12, prepared)?;
        self.take_install()?;
        Ok(())
    }
    #[inline(never)]
    fn runtime(&self) -> Result<Box<factory::runtime::Runtime>> {
        self.r
            .runtime
            .integration
            .clone()
            .map(Box::new)
            .ok_or_else(|| "factory runtime".into())
    }
    fn cold(
        &self,
        boot: Boot,
        composition: &Composition,
        mode: CoreOriginMode,
        enabled: bool,
    ) -> Result<Box<TestWorld>> {
        let (_, registry) = mush_testkit::boot_world();
        let mut e = Engine::with_modules(
            self.r.w.world().clone(),
            registry,
            composition.modules(mode, enabled, self.reverse)?,
        )?;
        let mut boot = boot;
        let mut typed = vec![];
        for (namespace, data) in &mut boot {
            data.kv.retain(|(key, value)| {
                if mush_core::is_action_state_persistence_key(key) {
                    typed.push(mush_core::ActionStateBootRecord::new(
                        namespace.clone(),
                        key.clone(),
                        value.clone(),
                    ));
                    false
                } else {
                    true
                }
            })
        }
        e.hydrate_action_state(typed)?;
        e.modules_loaded(
            boot,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        Ok(Box::new(TestWorld::from_engine(e)))
    }
    fn restart(&mut self, mode: CoreOriginMode) -> Result {
        self.r.w = self.cold(self.r.boot.clone(), &self.composition, mode, true)?;
        self.r.flush();
        self.r.runtime = self
            .r
            .w
            .world()
            .module_state::<TestHandles>()
            .ok_or("handles")?
            .0
            .clone();
        Ok(())
    }
    fn phase_action(&self, phase: usize) -> Result<DefinitionRef> {
        let runtime = self.runtime()?;
        if phase == 7 {
            Ok(runtime
                .delivery
                .as_ref()
                .ok_or("delivery disabled")?
                .action
                .clone())
        } else {
            Ok(runtime.actions.get(phase).ok_or("phase")?.clone())
        }
    }
    fn prepare_inv(&mut self, phase: usize, inv: &ActionInvocation, n: u8) -> Result<Prepared> {
        let a = self.phase_action(phase)?;
        if phase == 4 {
            let order = serde_json::from_value(inv.parameters()["order"].clone())?;
            return self.prepare_scheduled(&a, self.order(order)?.progress.cursor().id(), inv, n);
        }
        Ok(self
            .r
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&a, inv, token(n))?)
    }
    fn prepare_scheduled(
        &mut self,
        action: &DefinitionRef,
        schedule: mush_core::ScheduleId,
        inv: &ActionInvocation,
        n: u8,
    ) -> Result<Prepared> {
        let cause = if inv.parameters()["restart"].as_bool() == Some(true) {
            mush_core::ScheduleDriveCause::Restart
        } else {
            mush_core::ScheduleDriveCause::Live
        };
        let page = self
            .r
            .w
            .engine()
            .discover_scheduled_actions(cause, None, 256)?;
        let candidate = page
            .candidates()
            .iter()
            .find(|candidate| {
                candidate.action() == action && candidate.advance().schedule() == schedule
            })
            .ok_or("actual due schedule candidate")?;
        Ok(self
            .r
            .w
            .engine_mut()
            .prepare_registered_composed_scheduled_authoritative_action(candidate, inv, token(n))?)
    }
    fn scheduled_invocation(
        &self,
        action: &DefinitionRef,
        schedule: mush_core::ScheduleId,
    ) -> Result<ActionInvocation> {
        let page = self.r.w.engine().discover_scheduled_actions(
            mush_core::ScheduleDriveCause::Live,
            None,
            256,
        )?;
        let candidate = page
            .candidates()
            .iter()
            .find(|candidate| {
                candidate.action() == action && candidate.advance().schedule() == schedule
            })
            .ok_or("actual due schedule candidate")?;
        let ingress = mush_core::scheduled_action_ingress(action, candidate.advance(), 1)?;
        let (_, invocation, _) = self
            .r
            .w
            .engine()
            .plan_scheduled_action(candidate, &ingress)?
            .into_parts();
        Ok(invocation)
    }
    fn prepare(
        &mut self,
        phase: usize,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<Prepared> {
        self.prepare_inv(phase, &invocation(actor, n, parameters), n)
    }
    fn stage_inv(&mut self, phase: usize, inv: &ActionInvocation, n: u8, p: Prepared) -> Result {
        let action = self.phase_action(phase)?;
        self.stage_action(action, inv, n, p)
    }
    fn stage_action(
        &mut self,
        action: DefinitionRef,
        inv: &ActionInvocation,
        n: u8,
        p: Prepared,
    ) -> Result {
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        definitions.push(action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([3; 32]),
            action,
            provider: OwnerId::new("industry")?,
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })?;
        let guards = self
            .r
            .w
            .engine()
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())?
            .finish()?;
        self.r.w.world_mut().take_dirty();
        self.r
            .w
            .engine_mut()
            .stage_authoritative_action(p.0, guards, bundle)?;
        Ok(())
    }
    fn take_install(&mut self) -> Result<Vec<ReplayWrite>> {
        let held = self
            .r
            .w
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged")?;
        let (bundle, install) = held.into_parts();
        let writes = bundle.writes().to_vec();
        self.r.w.engine_mut().install_composed_action(install)?;
        let mut ops = vec![];
        for write in &writes {
            match write {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => ops.push(ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value: value.clone(),
                    },
                }),
                ReplayWrite::ModuleDelete { namespace, key } => ops.push(ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::Del {
                        key: key.as_str().into(),
                    },
                }),
                ReplayWrite::LedgerChunkSet { write } => ops.push(ModuleOp {
                    ns: write.namespace().as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: write.key().as_str().into(),
                        value: serde_json::to_value(write.chunk())?,
                    },
                }),
                ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => {}
                ReplayWrite::ObjectDelete { .. } => return Err("factory deleted object".into()),
            }
        }
        fold_module_ops_into(&mut self.r.boot, ops);
        Ok(writes)
    }
    fn commit(
        &mut self,
        phase: usize,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<Vec<ReplayWrite>> {
        let inv = invocation(actor, n, parameters);
        let p = self.prepare_inv(phase, &inv, n)?;
        self.stage_inv(phase, &inv, n, p)?;
        self.take_install()
    }
    fn offer_parameters(&self) -> Value {
        json!({"requester":self.buyer,"class":"i","supplier_lot":self.certificate.lot.to_string()})
    }
    fn offer(&mut self, n: u8) -> Result {
        self.commit(1, self.r.policy.agents[1].actor, n, self.offer_parameters())?;
        Ok(())
    }
    fn accept_parameters(&self, n: u8) -> Value {
        json!({"offer":token(n),"payer":self.payer.0.to_string()})
    }
    fn accepted(&mut self) -> Result<crate::workorder::WorkOrderId> {
        self.offer(20)?;
        self.commit(2, self.buyer, 21, self.accept_parameters(20))?;
        Ok(crate::workorder::derive_work_order_id(
            RequestUid::from_bytes([21; 16]),
            0,
        )?)
    }
    fn order(
        &self,
        id: crate::workorder::WorkOrderId,
    ) -> Result<factory::records::CoreIntegrationOrder> {
        Ok(self
            .r
            .w
            .engine()
            .action_state()
            .get(&self.runtime()?.families.orders, &id)?
            .ok_or("order")?
            .clone())
    }
    fn order_parameters(id: crate::workorder::WorkOrderId) -> Value {
        json!({"order":id.to_string()})
    }
    fn advance_invocation(
        &mut self,
        id: crate::workorder::WorkOrderId,
    ) -> Result<ActionInvocation> {
        let before = self.order(id)?;
        let due = before.progress.cursor().next_due().at;
        self.r.w.world_mut().set_now(due);
        self.scheduled_invocation(&self.runtime()?.actions[4], before.progress.cursor().id())
    }
    fn advance(&mut self, id: crate::workorder::WorkOrderId, n: u8) -> Result {
        let inv = self.advance_invocation(id)?;
        let p = self.prepare_inv(4, &inv, n)?;
        self.stage_inv(4, &inv, n, p)?;
        self.take_install()?;
        Ok(())
    }
    fn balance(&self, id: AccountId) -> Result<(Amount, Amount)> {
        let a = self
            .r
            .w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .account(id)
            .ok_or("account")?;
        Ok((a.posted_balance, a.held_amount))
    }
}

#[test]
fn genuine_supplier_buyer_order_runs_native_schedule_and_consumes_exact_certified_die() -> Result {
    for reverse in [false, true] {
        let mut f = FactoryRig::new(reverse)?;
        let before = f.balance(f.payer)?;
        let id = f.accepted()?;
        let accepted = f.order(id)?;
        // Actual supplier consent keeps the historical flat wire despite heap storage.
        let offer =
            f.r.w
                .engine()
                .action_state()
                .get(&f.runtime()?.families.offers, &token(20))?
                .ok_or("actual supplier offer")?;
        let input_wire = json!({
            "kind": "supplier_stock",
            "certificate": accepted.intent.certificate,
            "lot": accepted.intent.lot,
        });
        check_eq!(serde_json::to_value(&offer.input)?, input_wire);
        fn decode_like<T: serde::de::DeserializeOwned>(
            value: Value,
            _: &T,
        ) -> serde_json::Result<T> {
            serde_json::from_value(value)
        }
        let decoded = decode_like(input_wire.clone(), &offer.input)?;
        check_eq!(decoded, offer.input);
        let mut unknown = input_wire;
        unknown
            .as_object_mut()
            .ok_or("supplier input wire")?
            .insert("unregistered".into(), json!(true));
        check!(decode_like(unknown, &offer.input).is_err());
        let requester = decode_like(json!({"kind":"requester_supplied"}), &offer.input)?;
        check_eq!(
            serde_json::to_value(requester)?,
            json!({"kind":"requester_supplied"})
        );
        check_eq!(
            accepted.intent.expires_at.at - accepted.intent.accepted_at.at,
            7 * 86400
        );
        check_eq!(f.balance(f.payer)?, (before.0, Amount::from_minor(300_000)));
        check!(
            f.r.w
                .engine()
                .action_state()
                .get(&f.r.runtime.families.inventory.lots, &f.certificate.lot)?
                .is_some()
        );
        let birth = f.commit(3, f.buyer, 22, FactoryRig::order_parameters(id))?;
        check_eq!(
            birth
                .iter()
                .filter(|w| matches!(w, ReplayWrite::ObjectAllocatorSet { .. }))
                .count(),
            1
        );
        check!(
            f.prepare(5, f.buyer, 23, FactoryRig::order_parameters(id))
                .is_err()
        );
        f.advance(id, 24)?;
        f.advance(id, 25)?;
        let rows = f.commit(5, f.buyer, 26, FactoryRig::order_parameters(id))?;
        check!(
            rows.iter()
                .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
        );
        let complete = f.order(id)?;
        let terminal = complete.terminal.as_ref().ok_or("terminal")?;
        let CoreTerminalResult::Completed { title, .. } = terminal.result() else {
            return Err("completed title".into());
        };
        check_eq!(title.title_owner, PrincipalId::Object(f.buyer));
        check_eq!(
            f.balance(f.payer)?,
            (
                before
                    .0
                    .checked_sub(Amount::from_minor(300_000))
                    .ok_or("balance")?,
                Amount::from_minor(0)
            )
        );
        check_eq!(
            f.balance(f.composition.factory.payment.supplier.account)?.0,
            Amount::from_minor(270_000)
        );
        check_eq!(
            f.balance(f.composition.factory.payment.integrator.account)?
                .0,
            Amount::from_minor(30_000)
        );
        check!(
            f.r.w
                .engine()
                .action_state()
                .get(&f.r.runtime.families.inventory.lots, &f.certificate.lot)?
                .is_none()
        );
        check_eq!(
            f.r.w.world().get(title.object).ok_or("output")?.location,
            Some(f.composition.factory.payment.output_container)
        );
        check!(
            f.prepare(5, f.buyer, 27, FactoryRig::order_parameters(id))
                .is_err()
        );
        f.restart(CoreOriginMode::Active)?;
        check_eq!(f.order(id)?, complete);
    }
    Ok(())
}
#[test]
fn accepted_input_and_cash_release_before_or_after_output_then_remain_cold_proven() -> Result {
    for output in [false, true] {
        let mut f = FactoryRig::new(false)?;
        let balance = f.balance(f.payer)?;
        let id = f.accepted()?;
        if output {
            f.commit(3, f.buyer, 22, FactoryRig::order_parameters(id))?;
        }
        let slot = f.order(id)?.capacity;
        f.commit(6, f.buyer, 23, FactoryRig::order_parameters(id))?;
        check_eq!(f.balance(f.payer)?, balance);
        check!(matches!(
            f.order(id)?.terminal.as_ref().map(|r| r.result()),
            Some(CoreTerminalResult::Cancelled {})
        ));
        check!(
            f.r.w
                .engine()
                .action_state()
                .get(&f.r.runtime.families.inventory.lots, &f.certificate.lot)?
                .is_some()
        );
        let key = crate::facility::facility_capacity_key(f.composition.factory.payment.factory)?;
        let runtime = f.runtime()?;
        let book =
            f.r.w
                .engine()
                .action_state()
                .get(&runtime.industry.facility_books, &key)?
                .ok_or("book")?;
        check!(
            book.records()
                .iter()
                .any(|r| r.id() == slot && r.state() == mush_core::ReservationState::Released)
        );
        f.restart(CoreOriginMode::Active)?;
        check!(
            f.prepare(6, f.buyer, 24, FactoryRig::order_parameters(id))
                .is_err()
        );
        f.offer(30)?;
        f.commit(2, f.buyer, 31, f.accept_parameters(30))?;
        f.restart(CoreOriginMode::Active)?;
    }
    Ok(())
}
#[test]
fn supplier_authority_is_live_until_accept_custody_and_does_not_replace_buyer() -> Result {
    for phase in 0..3 {
        let mut f = FactoryRig::new(false)?;
        f.offer(20)?;
        let params = f.accept_parameters(20);
        let before = f.balance(f.payer)?;
        let revision = f.r.w.engine().action_state().revision();
        check!(
            f.prepare(2, f.r.policy.agents[1].actor, 21, params.clone())
                .is_err()
        );
        let mut p = if phase > 0 {
            Some(f.prepare(2, f.buyer, 22, params.clone())?)
        } else {
            None
        };
        if phase == 2 {
            f.stage_inv(
                2,
                &invocation(f.buyer, 22, params.clone()),
                22,
                p.take().ok_or("prepared")?,
            )?;
        }
        // Real acknowledged scoped Deny overrides the still-present explicit grant.
        f.r.deny(f.r.policy.agents[1].actor)?;
        f.r.w.world_mut().take_dirty();
        match phase {
            0 => check!(f.prepare(2, f.buyer, 22, params).is_err()),
            1 => check!(
                f.stage_inv(
                    2,
                    &invocation(f.buyer, 22, params),
                    22,
                    p.take().ok_or("prepared")?
                )
                .is_err()
            ),
            _ => check!(f.r.w.engine_mut().take_staged_action().is_err()),
        }
        check_eq!(f.balance(f.payer)?, before);
        check_eq!(f.r.w.engine().action_state().revision(), revision);
    }
    Ok(())
}
#[test]
fn supplier_departure_after_accept_does_not_reprice_or_cancel_captured_work() -> Result {
    let mut f = FactoryRig::new(false)?;
    let id = f.accepted()?;
    f.r.deny(f.r.policy.agents[1].actor)?;
    f.commit(3, f.buyer, 22, FactoryRig::order_parameters(id))?;
    f.advance(id, 24)?;
    f.advance(id, 25)?;
    f.commit(5, f.buyer, 26, FactoryRig::order_parameters(id))?;
    f.restart(CoreOriginMode::Active)?;
    check!(matches!(
        f.order(id)?.terminal.as_ref().map(|r| r.result()),
        Some(CoreTerminalResult::Completed { .. })
    ));
    Ok(())
}
#[test]
fn original_seven_day_expiry_and_changed_recipe_only_allow_stable_cancel() -> Result {
    let mut f = FactoryRig::new(false)?;
    let before = f.balance(f.payer)?;
    let id = f.accepted()?;
    f.commit(3, f.buyer, 22, FactoryRig::order_parameters(id))?;
    f.advance(id, 24)?;
    f.advance(id, 25)?;
    let row = f.order(id)?;
    f.r.w.world_mut().set_now(row.intent.expires_at.at);
    check!(
        f.prepare(5, f.buyer, 26, FactoryRig::order_parameters(id))
            .is_err()
    );
    f.composition = f.composition.changed_recipe()?;
    f.restart(CoreOriginMode::Active)?;
    check_eq!(f.order(id)?.intent, row.intent);
    check!(
        f.prepare(5, f.buyer, 27, FactoryRig::order_parameters(id))
            .is_err()
    );
    f.commit(6, f.buyer, 28, FactoryRig::order_parameters(id))?;
    check_eq!(f.balance(f.payer)?, before);
    f.restart(CoreOriginMode::Active)?;
    Ok(())
}
#[test]
fn exact_owner_and_economy_cold_inverse_refuse_lost_or_crossed_factory_evidence() -> Result {
    let mut f = FactoryRig::new(false)?;
    let id = f.accepted()?;
    f.commit(3, f.buyer, 22, FactoryRig::order_parameters(id))?;
    for family in [
        "industry_core_orders",
        "industry_core_integration_deployment",
        "inventory_core_inputs",
        "core_output_reservations",
    ] {
        let mut boot = f.r.boot.clone();
        let mut removed = 0;
        for data in boot.values_mut() {
            data.kv.retain(|(_, v)| {
                let matches = v
                    .get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some(family);
                if matches {
                    removed += 1
                }
                !matches
            });
        }
        check!(removed > 0, "fixture must remove an actual exact family");
        check!(
            f.cold(boot, &f.composition, CoreOriginMode::Active, true)
                .is_err()
        );
    }
    let mut boot = f.r.boot.clone();
    let data = boot.get_mut("industry").ok_or("industry")?;
    let before = data.kv.len();
    data.kv
        .retain(|(k, _)| k.as_str() != factory::records::INDEX_KEY);
    check_eq!(before - data.kv.len(), 1);
    check!(
        f.cold(boot, &f.composition, CoreOriginMode::Active, true)
            .is_err()
    );
    check!(
        f.cold(
            f.r.boot.clone(),
            &f.composition,
            CoreOriginMode::Active,
            false
        )
        .is_err()
    );
    f.advance(id, 24)?;
    f.advance(id, 25)?;
    f.commit(5, f.buyer, 26, FactoryRig::order_parameters(id))?;
    let mut boot = f.r.boot.clone();
    let mut changed = 0;
    for data in boot.values_mut() {
        for (_, v) in &mut data.kv {
            if v.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                == Some("industry_core_orders")
            {
                let r = v.get_mut("value").ok_or("typed value")?;
                r["intent"]["offer"] = json!(token(99));
                changed += 1
            }
        }
    }
    check_eq!(changed, 1);
    check!(
        f.cold(boot, &f.composition, CoreOriginMode::Active, true)
            .is_err()
    );
    Ok(())
}
#[test]
fn source_free_selectors_reject_alternates_and_unfunded_completion() -> Result {
    let mut f = FactoryRig::new(false)?;
    f.offer(20)?;
    for value in [
        json!({"offer":token(20),"payer":f.payer.0}),
        json!({"offer":token(20),"payer":format!("0{}",f.payer.0)}),
        json!({"offer":token(20),"payer":f.payer.0.to_string(),"input_lot":null}),
        json!({"offer":token(20),"payer":f.payer.0.to_string(),"price":1}),
    ] {
        check!(f.prepare(2, f.buyer, 21, value).is_err());
    }
    let id = crate::workorder::WorkOrderId(0);
    check!(
        f.prepare(5, f.buyer, 22, FactoryRig::order_parameters(id))
            .is_err()
    );
    check!(
        f.prepare(
            4,
            f.buyer,
            23,
            json!({"order":id,"observed":f.r.w.world().world_ic_now(),"restart":false})
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn completed_core_birth_blocks_generic_title_mutations_before_and_after_cold_boot() -> Result {
    use mush_economy::{
        AllowAll, AssetOp, EconomicCause, EconomyError, EconomyPlan, Encumbrance, IdempotencyKey,
        MapOnlyKeys, NoCustody, NoFacts, PlanResult, ValidateEnv,
    };
    let mut f = FactoryRig::new(false)?;
    let order = f.accepted()?;
    f.commit(3, f.buyer, 22, FactoryRig::order_parameters(order))?;
    f.advance(order, 24)?;
    f.advance(order, 25)?;
    f.commit(5, f.buyer, 26, FactoryRig::order_parameters(order))?;
    let row = f.order(order)?;
    let Some(CoreTerminalResult::Completed { output, .. }) =
        row.terminal.as_ref().map(|t| t.result())
    else {
        return Err("actual completed output".into());
    };
    let object = output.object;
    for cold in [false, true] {
        if cold {
            f.restart(CoreOriginMode::Active)?;
        }
        let snapshot =
            f.r.w
                .world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot();
        let before = snapshot.clone();
        let env = ValidateEnv {
            auth: &AllowAll,
            custody: &NoCustody,
            facts: &NoFacts,
            now: f.r.w.world().now(),
            ic_now: f.r.w.world().game_now_unix(),
            next_hold_id: 0,
            spent: Default::default(),
            keys: &MapOnlyKeys,
        };
        let operations = vec![
            AssetOp::Title {
                object,
                owner: PrincipalId::Object(GOD),
                scope: None,
                catalog: None,
            },
            AssetOp::TransferTitle {
                object,
                to: PrincipalId::Object(GOD),
                to_scope: None,
            },
            AssetOp::SetCatalog {
                object,
                catalog: None,
            },
            AssetOp::Abandon { object },
            AssetOp::WriteOff {
                object,
                reason: "generic mutation".into(),
            },
            AssetOp::Encumber {
                object,
                encumbrance: Encumbrance::Lien {
                    creditor: PrincipalId::Object(GOD),
                    amount: Amount::from_minor(1),
                    instrument: f.composition.factory.payment.instrument.clone(),
                },
            },
        ];
        for op in operations {
            let mut plan = EconomyPlan::new(
                IdempotencyKey::new("factory.guard.fixture", "attempt"),
                PrincipalId::Object(GOD),
                EconomicCause::Ruleset {
                    namespace: "fixture".into(),
                    correlation: "ordinary".into(),
                },
                vec![],
            );
            plan.asset_ops.push(op);
            check!(
                matches!(
                    mush_economy::validate(&plan, snapshot, &env),
                    PlanResult::Refused(EconomyError::Unauthorized)
                ),
                "actual Core birth must remain governed"
            );
        }
        check!(snapshot == &before, "generic refusal changed the snapshot");
    }
    Ok(())
}
#[test]
fn class_three_stock_cannot_charge_a_buyer_without_actual_personal_entitlement() -> Result {
    let mut f = FactoryRig::new(false)?;
    let actor = f.r.policy.agents[1].actor;
    let cert = f.r.issue(actor, 40, DieClass::III)?;
    f.commit(
        1,
        actor,
        41,
        json!({"requester":f.buyer,"class":DieClass::III,"supplier_lot":cert.lot.to_string()}),
    )?;
    let before = f.balance(f.payer)?;
    let revision = f.r.w.engine().action_state().revision();
    check!(f.prepare(2, f.buyer, 42, f.accept_parameters(41)).is_err());
    check_eq!(f.balance(f.payer)?, before);
    check_eq!(f.r.w.engine().action_state().revision(), revision);
    check!(
        f.r.w
            .engine()
            .action_state()
            .get(&f.r.runtime.families.inventory.lots, &cert.lot)?
            .is_some()
    );
    Ok(())
}

#[test]
fn projected_class_values_enter_actual_origin_and_factory_providers() -> Result {
    let mut f = FactoryRig::new(false)?;
    let actor = f.r.policy.agents[1].actor;
    let keys = mush_core::OfferKeyRing::new(mush_core::OfferSecret::new(1, [7; 32])?, Vec::new())?;
    let query = mush_core::LegalActionQuery::new(
        mush_core::PrincipalId::Object { id: actor },
        actor,
        mush_core::SubjectKey::Object { id: actor },
        PerceivedRef::SelfActor,
        1,
        Some(f.r.w.world().world_ic_now()),
    )?;
    let sink =
        f.r.w
            .engine()
            .registry
            .action_providers()
            .ok_or("providers")?
            .project_legal_actions(f.r.w.engine(), &query, &keys)?;
    let choices = |action: &DefinitionRef| -> Result<Vec<String>> {
        let offer = sink
            .projected_for_action(action)
            .ok_or("actual projected offer")?;
        let field = offer
            .parameters()
            .parameters()
            .iter()
            .find(|p| p.id.as_str() == "class")
            .ok_or("class selector")?;
        let mush_core::ProjectedParameterKind::Choice { choices } = &field.kind else {
            return Err("class choices".into());
        };
        Ok(choices.clone())
    };
    let origin = choices(&f.r.runtime.actions[1])?;
    let factory = choices(&f.runtime()?.actions[1])?;
    check_eq!(origin, factory);
    check_eq!(origin.len(), 3);
    for (i, value) in origin.iter().enumerate() {
        let n = 50 + u8::try_from(i)?;
        let p = f.r.prepare(1, actor, n, json!({"class":value}))?;
        f.r.ack(1, actor, n, json!({"class":value}), p)?;
        let cert = f.r.cert(n)?;
        check_eq!(serde_json::to_value(cert.class)?, json!(value));
        f.commit(
            1,
            actor,
            60 + u8::try_from(i)?,
            json!({"class":value,"requester":f.buyer,"supplier_lot":cert.lot.to_string()}),
        )?;
    }
    f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[path = "integration_delivery_tests.rs"]
mod delivery_tests;

#[inline(never)]
fn apply_scheduler_ingress(f: &mut FactoryRig, actual: &ActionInvocation) -> Result {
    let p = f.prepare_inv(4, actual, 24)?;
    f.stage_inv(4, actual, 24, p)?;
    f.take_install()?;
    Ok(())
}

#[inline(never)]
fn scheduler_ingress_case(wrong_request: bool) -> Result {
    let mut f = FactoryRig::new(false)?;
    let id = f.accepted()?;
    let actual = f.advance_invocation(id)?;
    let before = f.order(id)?;
    let balance = f.balance(f.payer)?;
    let revision = f.r.w.engine().action_state().revision();
    let forged = if wrong_request {
        ActionInvocation::new(ActionInvocationDraft {
            request: RequestUid::from_bytes([99; 16]),
            principal: actual.principal().clone(),
            actor: actual.actor(),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: actual.parameters().clone(),
            idempotency: actual.idempotency().clone(),
        })?
    } else {
        invocation(f.buyer, 24, actual.parameters().clone())
    };
    check!(
        f.prepare_inv(4, &forged, 24).is_err(),
        "wrong scheduler ingress accepted"
    );
    check!(
        f.order(id)? == before,
        "refused scheduler changed the order"
    );
    check_eq!(f.balance(f.payer)?, balance);
    check_eq!(f.r.w.engine().action_state().revision(), revision);
    // Exact K-shaped ingress alone is insufficient: only the scheduler
    // entry point seeds the authoritative cursor into this transaction.
    let action = f.phase_action(4)?;
    check!(
        f.r.w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&action, &actual, token(24))
            .is_err()
    );
    check_eq!(f.r.w.engine().action_state().revision(), revision);
    check!(f.order(id)? == before);
    // The identical actual K ingress remains admissible after refusal.
    apply_scheduler_ingress(&mut f, &actual)?;
    check_ne!(f.order(id)?.progress, before.progress);
    Ok(())
}

#[inline(never)]
fn scheduler_fence_case(dependency: &str, phase: i32) -> Result {
    let mut f = FactoryRig::new(false)?;
    let id = f.accepted()?;
    let inv = f.advance_invocation(id)?;
    let before = f.order(id)?;
    let balance = f.balance(f.payer)?;
    let revision = f.r.w.engine().action_state().revision();
    let disk = f.r.boot.clone();
    let p = if phase > 0 {
        Some(f.prepare_inv(4, &inv, 24)?)
    } else {
        None
    };
    let p = if phase == 2 {
        f.stage_inv(4, &inv, 24, p.ok_or("scheduled prepared image")?)?;
        None
    } else {
        p
    };
    f.r.w.engine().registry.freeze_mechanics_owner(
        &OwnerId::new(dependency)?,
        mush_core::mechanics::IncidentId(1),
        mush_core::mechanics::RecoveryReason::IntegrityViolation {
            code: super::id("factory_schedule_fixture")?,
        },
    )?;
    match phase {
        0 => check!(
            f.prepare_inv(4, &inv, 24).is_err(),
            "frozen dependency prepared"
        ),
        1 => check!(
            f.stage_inv(4, &inv, 24, p.ok_or("scheduled prepared image")?)
                .is_err(),
            "frozen dependency staged"
        ),
        _ => check!(
            f.r.w.engine_mut().take_staged_action().is_err(),
            "frozen dependency reached custody"
        ),
    }
    check_eq!(f.r.w.engine().action_state().revision(), revision);
    check_eq!(f.balance(f.payer)?, balance);
    // Owner reads correctly refuse a frozen owner; the durable image is
    // compared directly without granting a post-freeze read capability.
    check!(
        factory_boot_image(&f.r.boot) == factory_boot_image(&disk),
        "refusal changed persistence"
    );
    check!(
        before.terminal.is_none(),
        "fixture must retain unfinished work"
    );
    Ok(())
}

#[test]
fn scheduled_progress_preserves_actual_ingress_and_all_dependency_custody_fences() -> Result {
    for wrong_request in [false, true] {
        scheduler_ingress_case(wrong_request)?;
    }
    for dependency in ["industry", "org", "economy", "inventory", "tacsim"] {
        for phase in 0..3 {
            scheduler_fence_case(dependency, phase)?;
        }
    }
    Ok(())
}

// Preserve all ModuleBootData fields and ordered document/ledger/stream tails.
// Equality failures report only the fixed message above.
fn factory_boot_image(boot: &Boot) -> std::collections::BTreeMap<&str, Value> {
    boot.iter()
        .map(|(namespace, data)| {
            (
                namespace.as_str(),
                json!({
                    "kv":data.kv, "ledger":data.ledger, "streams":data.streams,
                    "fetch_ms":data.fetch_ms, "omitted":data.omitted,
                }),
            )
        })
        .collect()
}

#[path = "refit_test_rules.rs"]
mod refit_rules;
#[path = "refit_integration_tests.rs"]
mod refit_tests;
