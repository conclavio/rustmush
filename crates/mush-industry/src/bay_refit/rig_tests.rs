#![cfg(test)]

//! Doc 31 cut UD.4b, end to end: a supplier's offer, the requester's
//! escrowed acceptance, the scheduled work on the IC clock, and Install or
//! Cancel, through the real Org, Economy, Inventory, Industry, Sim and
//! Tactics owners and the kernel's `bay_refit` protocol. Nothing is seeded
//! but ordinary staff setup: no receipt, hold, part custody or unit image.
use super::*;
use crate::IndustryModule;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, PreparedAuthoritativeAction, PreparedCausalRef,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, Engine, GOD, GameModule, IdempotencyKey, IdempotencyScope,
    ModuleBootData, ModuleOp, ModuleWrite, ObjKind, Outbox, PerceivedRef, PrepareCtx, RequestUid,
    RulesValue, ScopedIdempotencyKey,
};
use mush_economy::bay_refit::{BayPaymentPolicy, BaySite};
use mush_economy::core_integration::ScopedCoreAccount;
use mush_economy::registry_filing::InitialFilingPolicy;
use mush_economy::{AccountId, Amount, CatalogRef, EconomyModule, EconomyStore, InstrumentId};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule, LotId};
use mush_org::{OrgModule, OrgStore, PrincipalId};
use mush_tactics::unit::replacement::ComponentState;
use mush_tactics::{
    ComponentId, ComponentSpec, ConditionId, InstallPoint, SectionId, SectionSpec, UnitSpec,
    UnitSpecBody,
};
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
type Boot = HashMap<SmolStr, ModuleBootData>;

fn cid(s: &str) -> Result<CanonicalId> {
    Ok(CanonicalId::new(s)?)
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn command(world: &mut TestWorld, line: &str) -> Result {
    let output = world.run(GOD, line);
    if output.iter().any(|(_, e)| matches!(e, mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s| s.tag.as_deref() == Some("error")))) {
        return Err(format!("setup command refused: {line}").into());
    }
    Ok(())
}

/// Two registered variants of one hull: a cannon on the hull, or a laser in
/// the same place. Each component's definition is its own.
#[derive(Clone)]
struct Units {
    specs: Vec<UnitSpec>,
    descriptors: Vec<DefinitionDescriptor>,
    recipe: DefinitionDescriptor,
}
impl Units {
    fn new() -> Result<Self> {
        let owner = OwnerId::new("tacsim")?;
        let mut b = DefinitionRegistryBuilder::new();
        let schema = DefinitionDescriptor::new(
            DefinitionKind::new("schema")?,
            DefinitionId::new("bay_rules")?,
            1,
            json!({"format":1}),
        )?;
        let schema_ref = b.register(owner.clone(), schema.clone())?;
        let mut descriptors = vec![schema];
        let mut specs = vec![];
        for (unit, component) in [("bay_unit_a", "cannon"), ("bay_unit_b", "laser")] {
            let d = DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new(format!("bay_{component}"))?,
                1,
                json!({"weapon":component}),
            )?;
            let definition = b.register(owner.clone(), d.clone())?;
            descriptors.push(d);
            let body = UnitSpecBody {
                sections: vec![SectionSpec {
                    id: SectionId::new("hull")?,
                    condition: ConditionId::new("ok")?,
                    facets: vec![],
                    slots: vec![],
                    cells: vec![],
                }],
                unit_cells: vec![],
                components: vec![ComponentSpec {
                    id: ComponentId::new(component)?,
                    definition,
                    install: InstallPoint::Unit,
                    condition: ConditionId::new("ok")?,
                    cells: vec![],
                    asset: None,
                }],
                capabilities: vec![],
                mobility: None,
                rules: RulesValue::new(schema_ref.clone(), json!({}))?,
            };
            let d = body
                .descriptor(DefinitionKind::new("unit")?, DefinitionId::new(unit)?, 1)?
                .as_current();
            let reference = b.register(owner.clone(), d.clone())?;
            descriptors.push(d);
            specs.push(body.with_definition(reference));
        }
        let recipe = DefinitionDescriptor::new(
            DefinitionKind::new("recipe")?,
            DefinitionId::new("bay_refit")?,
            1,
            json!({"format":1,"fixture":"bay"}),
        )?;
        Ok(Self {
            specs,
            descriptors,
            recipe,
        })
    }
    fn recipe_ref(&self) -> Result<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new()
            .register(OwnerId::new("tacsim")?, self.recipe.clone())?)
    }
    fn roots(&self) -> BTreeSet<DefinitionRef> {
        self.specs.iter().map(|s| s.definition.clone()).collect()
    }
}
impl GameModule for Units {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let mut grouped = BTreeMap::<DefinitionKind, Vec<DefinitionDescriptor>>::new();
        for d in self.descriptors.iter().chain([&self.recipe]) {
            grouped.entry(d.kind().clone()).or_default().push(d.clone());
        }
        for (kind, expected) in grouped {
            reg.definition_validator(
                kind,
                move |ctx: &mush_core::DefinitionValidationContext<'_>| {
                    let actual = ctx.definition();
                    if expected.iter().any(|d| {
                        let mut refs = d.references().to_vec();
                        refs.sort();
                        d.id() == &actual.reference().key().id
                            && d.version() == actual.reference().version()
                            && d.payload() == actual.payload()
                            && refs == actual.references()
                            && d.is_current() == actual.is_current()
                    }) {
                        Ok(())
                    } else {
                        Err("bay fixture definition changed".into())
                    }
                },
            )?;
        }
        for d in self.descriptors.iter().chain([&self.recipe]) {
            reg.definition(d.clone())?;
        }
        Ok(())
    }
}
/// The ruleset's part catalogue: a component's commodity is its definition id.
struct Parts;
impl BayPartCatalog for Parts {
    fn commodity(&self, component: &DefinitionRef) -> Option<CanonicalId> {
        CanonicalId::new(format!("part.{}", component.key().id.as_str())).ok()
    }
    fn grade(&self, _: &ComponentState) -> Option<Grade> {
        Grade::new(5).ok()
    }
}
/// The Roll filing the Economy participant requires, registered only as a
/// refusing prerequisite.
struct ClosedRoll;
struct Refuses;
fn refusal() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "fixture filing unavailable",
    )
    .expect("bounded error")
}
impl ActionProvider for Refuses {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(refusal())
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        _: &mut ActionTxn<'_>,
        _: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        Err(refusal())
    }
}
fn filing_descriptor() -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action")?,
        DefinitionId::new("initial_filing")?,
        1,
        json!({"fixture":"unavailable_initial_filing"}),
    )?)
}
fn filing_ref() -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, filing_descriptor()?)?)
}
impl GameModule for ClosedRoll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let d = filing_descriptor().map_err(|e| ModError::msg(e.to_string()))?;
        crate::core_production::provider::exact(reg, "action", std::slice::from_ref(&d))?;
        let a = reg.definition(d)?;
        reg.action_provider(a, Refuses)
    }
}

struct Composition {
    units: Units,
    filing: InitialFilingPolicy,
    config: BayRefitConfig,
}
impl Composition {
    fn modules(&self) -> Result<Vec<Box<dyn GameModule>>> {
        let cell: InventoryHandleCell = Arc::default();
        let mut catalog = mush_tactics::UnitCatalogBuilder::new();
        let vocabulary = mush_tactics::MobilityVocabulary::base().freeze();
        for spec in &self.units.specs {
            catalog.add(spec.clone(), &vocabulary)?;
        }
        let config = self.config.fresh();
        let tactics = mush_tactics::TacticsModule::with_catalog(catalog.freeze())
            .with_component_refit(
                config.tactics_policy(),
                config.action(INSTALL)?,
                config.roots().clone(),
                config.derived().to_vec(),
            );
        Ok(vec![
            Box::new(OrgModule::new()),
            Box::new(
                EconomyModule::new()
                    .with_title_issuers(vec![self.filing.issuer().clone()])
                    .with_initial_filing(self.filing.clone(), filing_ref()?)
                    .with_bay_refit(config.economy_config()?),
            ),
            Box::new(
                InventoryModule::new()
                    .with_quantity_delegates(["industry"])
                    .with_handle_cell(cell.clone())
                    .with_bay_parts(config.part_binding()?),
            ),
            Box::new(self.units.clone()),
            Box::new(ClosedRoll),
            Box::new(
                IndustryModule::new()
                    .with_inventory(cell)
                    .with_bay_refit(config),
            ),
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(tactics),
        ])
    }
}

struct Rig {
    w: Box<TestWorld>,
    composition: Composition,
    boot: Boot,
    requester: ObjId,
    operator: ObjId,
    hull: ObjId,
    bay: ObjId,
    stores: EndpointRef,
    payer: AccountId,
    provider: AccountId,
    lot: LotId,
}
impl Rig {
    #[inline(never)]
    fn new() -> Result<Box<Self>> {
        let units = Units::new()?;
        let mut w = TestWorld::with_modules(vec![
            Box::new(OrgModule::new()),
            Box::new(EconomyModule::new()),
        ]);
        w.world_mut().set_now(1000);
        w.world_mut().set_game_clock(mush_core::clock::GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        });
        let requester = w.spawn_player("Bay customer");
        let operator = w.spawn_player("Bay chief");
        let bay = w
            .world_mut()
            .create(ObjKind::Thing, "Repair bay", GOD, None)?;
        let stores_object = w
            .world_mut()
            .create(ObjKind::Thing, "Bay stores", GOD, Some(bay))?;
        let hull = w
            .world_mut()
            .create(ObjKind::Thing, "Bay hull", requester, Some(bay))?;
        w.world_mut().move_to(requester, bay)?;
        command(&mut w, "@org/create Yard")?;
        command(&mut w, "@org/scope create Yard=bay")?;
        let orgs = w.world().module_state::<OrgStore>().ok_or("org")?.state();
        let yard = orgs.orgs().next().ok_or("yard")?.id;
        let scope = orgs
            .scopes_in(yard)
            .find(|s| s.name.as_str() == "bay")
            .ok_or("scope")?
            .id;
        command(&mut w, "@instrument/register cr=2")?;
        command(&mut w, &format!("@account/open org:Yard/{}=cr", scope.0))?;
        command(&mut w, &format!("@account/open #{}=cr", requester.0))?;
        command(&mut w, "@account/system bay.fixture=cr")?;
        command(
            &mut w,
            &format!(
                "@vault/create Stores=container:#{} owner=org:Yard scope={}",
                stores_object.0, scope.0
            ),
        )?;
        let snapshot = w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot();
        let provider = snapshot
            .accounts()
            .find(|a| a.scope == Some(scope))
            .ok_or("provider")?
            .id;
        let payer = snapshot
            .accounts()
            .find(|a| a.owner == PrincipalId::Object(requester))
            .ok_or("payer")?
            .id;
        let faucet = snapshot
            .accounts()
            .find(|a| matches!(&a.owner, PrincipalId::System(s) if s.as_str() == "bay.fixture"))
            .ok_or("faucet")?
            .id;
        let stores = EndpointRef(
            snapshot
                .vaults()
                .find(|v| v.name.as_str() == "Stores")
                .ok_or("stores")?
                .id,
        );
        command(
            &mut w,
            &format!("@econ/mint {}={} 1000000", faucet.0, payer.0),
        )?;
        let issuer = mush_org::SystemPrincipalId::new("bay.yard").ok_or("issuer")?;
        let filing = InitialFilingPolicy::new(
            issuer.clone(),
            provider,
            Amount::from_minor(0),
            CatalogRef {
                ruleset: "fixture".into(),
                code: "unused".into(),
            },
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
        let payment = BayPaymentPolicy {
            issuer,
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            recipe: units.recipe_ref()?,
            bays: vec![BaySite {
                facility: bay,
                provider: ScopedCoreAccount {
                    account: provider,
                    organization: yard,
                    scope,
                },
            }],
            max_fee_minor: 50_000,
        };
        let config = BayRefitConfig::new(
            payment,
            2,
            BTreeMap::from([(bay, stores)]),
            units.roots(),
            vec![],
            Arc::new(Parts),
            economy_owner,
        )?;
        let composition = Composition {
            units,
            filing,
            config,
        };
        let mut boot = fold_module_ops(w.take_module_ops());
        let mut w = w.reboot_into(composition.modules()?, boot.clone())?;
        fold_module_ops_into(&mut boot, w.take_module_ops());
        command(
            &mut w,
            &format!("@facility/open {}=2 {BAY_CAPABILITY}", bay.0),
        )?;
        command(&mut w, &format!("@facility/owner {}=org:Yard", bay.0))?;
        command(
            &mut w,
            &format!("@asset/title #{}=org:Yard scope={}", bay.0, scope.0),
        )?;
        command(&mut w, &format!("@unit/create #{}=bay_unit_a", hull.0))?;
        command(
            &mut w,
            &format!("@asset/title #{}=#{}", hull.0, requester.0),
        )?;
        command(
            &mut w,
            &format!(
                "@org/grant #{}=Yard {} scope=#{}",
                operator.0,
                mush_economy::bay_refit::OPERATOR_CAPABILITY,
                scope.0
            ),
        )?;
        fold_module_ops_into(&mut boot, w.take_module_ops());
        let mut r = Box::new(Self {
            w: Box::new(w),
            composition,
            boot,
            requester,
            operator,
            hull,
            bay,
            stores,
            payer,
            provider,
            lot: LotId(7),
        });
        r.mint_part()?;
        r.restart()?;
        r.commit(0, GOD, 10, json!({}))?;
        Ok(r)
    }
    /// A laser part in the bay's stores, minted as ordinary Origin matter.
    fn mint_part(&mut self) -> Result {
        let f = self.inventory()?;
        let lot = self.lot;
        let stores = self.stores;
        let commodity = cid("part.bay_laser")?;
        let p = {
            let mut txn = self.w.engine_mut().action_transaction(&f.lots)?;
            f.mint_origin(
                &mut txn,
                stores,
                &commodity,
                1,
                Grade::new(9)?,
                mush_economy::ExternalRef {
                    provider: "fixture".into(),
                    kind: "bay_part".into(),
                    id: "7".into(),
                    revision: Some(1),
                },
                lot,
            )?;
            txn.prepare()?
        };
        let (writes, install) = p.into_parts();
        self.w.engine_mut().install_prepared_action(install)?;
        fold_module_ops_into(&mut self.boot, writes);
        Ok(())
    }
    fn inventory(&self) -> Result<mush_inventory::InventoryFamilies> {
        Ok(self
            .w
            .world()
            .module_state::<crate::IndustryHandles>()
            .ok_or("industry handles")?
            .0
            .inventory
            .clone())
    }
    fn runtime(&self) -> Result<Arc<runtime::BayRuntime>> {
        Ok(self
            .w
            .world()
            .module_state::<runtime::BayHandle>()
            .ok_or("bay runtime")?
            .0
            .clone())
    }
    fn restart(&mut self) -> Result {
        let (_, registry) = mush_testkit::boot_world();
        let mut e = Engine::with_modules(
            self.w.world().clone(),
            registry,
            self.composition.modules()?,
        )?;
        let mut boot = self.boot.clone();
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
            });
        }
        e.hydrate_action_state(typed)?;
        e.modules_loaded(
            boot,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        *self.w = TestWorld::from_engine(e);
        fold_module_ops_into(&mut self.boot, self.w.take_module_ops());
        Ok(())
    }
    fn invocation(&self, actor: ObjId, n: u8, parameters: Value) -> Result<ActionInvocation> {
        let request = RequestUid::from_bytes([n; 16]);
        let principal = mush_core::PrincipalId::Object { id: actor };
        Ok(ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("industry")?,
                    namespace: cid("bay_fixture")?,
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })?)
    }
    fn prepare_inv(&mut self, phase: usize, inv: &ActionInvocation, n: u8) -> Result<Prepared> {
        let action = self.runtime()?.actions[phase].clone();
        if phase == 3 {
            let page = self.w.engine().discover_scheduled_actions(
                mush_core::ScheduleDriveCause::Live,
                None,
                256,
            )?;
            let candidate = page
                .candidates()
                .iter()
                .find(|c| c.action() == &action)
                .ok_or("a due bay schedule")?
                .clone();
            return Ok(self
                .w
                .engine_mut()
                .prepare_registered_composed_scheduled_authoritative_action(
                    &candidate,
                    inv,
                    token(n),
                )?);
        }
        Ok(self
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&action, inv, token(n))?)
    }
    fn stage(&mut self, phase: usize, inv: &ActionInvocation, n: u8, p: Prepared) -> Result {
        let action = self.runtime()?.actions[phase].clone();
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
            .w
            .engine()
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())?
            .finish()?;
        self.w.world_mut().take_dirty();
        self.w
            .engine_mut()
            .stage_authoritative_action(p.0, guards, bundle)?;
        let held = self.w.engine_mut().take_staged_action()?.ok_or("staged")?;
        let (bundle, install) = held.into_parts();
        self.w.engine_mut().install_composed_action(install)?;
        let mut ops = vec![];
        for write in bundle.writes() {
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
                ReplayWrite::ObjectDelete { .. } => return Err("the bay deleted an object".into()),
            }
        }
        fold_module_ops_into(&mut self.boot, ops);
        fold_module_ops_into(&mut self.boot, self.w.take_module_ops());
        Ok(())
    }
    fn commit(&mut self, phase: usize, actor: ObjId, n: u8, parameters: Value) -> Result {
        let inv = self.invocation(actor, n, parameters)?;
        let p = self.prepare_inv(phase, &inv, n)?;
        self.stage(phase, &inv, n, p)
    }
    /// Advance the order's schedule at its next due time, through the real
    /// scheduler ingress.
    fn advance(&mut self, order: crate::workorder::WorkOrderId, n: u8) -> Result {
        let runtime = self.runtime()?;
        let before = self.order(order)?;
        let due = before.progress.cursor().next_due().at;
        self.w.world_mut().set_now(due);
        let action = runtime.actions[3].clone();
        let page = self.w.engine().discover_scheduled_actions(
            mush_core::ScheduleDriveCause::Live,
            None,
            256,
        )?;
        let candidate = page
            .candidates()
            .iter()
            .find(|c| c.action() == &action)
            .ok_or("a due bay schedule")?;
        let ingress = mush_core::scheduled_action_ingress(&action, candidate.advance(), 1)?;
        let (_, inv, _) = self
            .w
            .engine()
            .plan_scheduled_action(candidate, &ingress)?
            .into_parts();
        let p = self.prepare_inv(3, &inv, n)?;
        self.stage(3, &inv, n, p)
    }
    fn order(&self, id: crate::workorder::WorkOrderId) -> Result<records::BayOrder> {
        Ok(self
            .w
            .engine()
            .action_state()
            .get(&self.runtime()?.families.orders, &id)?
            .ok_or("order")?
            .clone())
    }
    fn balance(&self, id: AccountId) -> Result<(i64, i64)> {
        let a = self
            .w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .account(id)
            .ok_or("account")?
            .clone();
        Ok((a.posted_balance.minor(), a.held_amount.minor()))
    }
    fn unit(&self) -> Result<mush_tactics::TacticalUnit> {
        Ok(self
            .w
            .world()
            .module_state::<mush_tactics::UnitStore>()
            .ok_or("units")?
            .units
            .get(&self.hull)
            .ok_or("the hull's unit")?
            .clone())
    }
    /// Offer the laser refit, then accept it: the order it opens.
    fn accepted(&mut self) -> Result<crate::workorder::WorkOrderId> {
        let offer = json!({"requester":self.requester,"hull":self.hull,"facility":self.bay,
            "target":"bay_unit_b","part":self.lot,"fee":2500});
        self.commit(1, self.operator, 20, offer)?;
        self.commit(
            2,
            self.requester,
            21,
            json!({"offer":token(20),"payer":self.payer.0.to_string()}),
        )?;
        Ok(crate::workorder::derive_work_order_id(
            RequestUid::from_bytes([21; 16]),
            0,
        )?)
    }
}

/// **The laser refit, end to end**: the fee is held at Accept, the hull is
/// held in the bay and the laser lot reserved; two IC hours of work later
/// Install captures exactly the fee, refits the hull to the laser variant,
/// consumes the laser lot and returns the cannon to stores; and a cold boot
/// reconstructs every owner's image.
#[test]
fn ud4b_a_bay_refit_holds_works_installs_and_survives_a_cold_boot() -> Result {
    let mut r = Rig::new()?;
    let before = r.unit()?;
    let id = r.accepted()?;
    check_eq!(r.balance(r.payer)?, (1_000_000, 2500), "the fee is held");
    let hulls = r.runtime()?.families.hulls().clone();
    let head =
        r.w.engine()
            .action_state()
            .get(&hulls, &r.hull)?
            .ok_or("hull head")?
            .clone();
    check_eq!(head.pending(), Some(id), "the hull is held in the bay");
    check!(
        r.commit(4, r.requester, 22, json!({"order":id.to_string()}))
            .is_err(),
        "no install before the work is done"
    );
    r.advance(id, 30)?;
    r.advance(id, 31)?;
    check!(r.order(id)?.progress.production().is_complete());
    r.commit(4, r.requester, 32, json!({"order":id.to_string()}))?;
    let after = r.unit()?;
    check_eq!(after.schema().key().id.as_str(), "bay_unit_b");
    check_eq!(after.revision(), before.revision() + 1);
    check!(after.components().any(|(c, _)| c.as_str() == "laser"));
    check_eq!(
        r.balance(r.payer)?,
        (1_000_000 - 2500, 0),
        "the fee is captured"
    );
    check_eq!(r.balance(r.provider)?.0, 2500, "and paid to the bay");
    let f = r.inventory()?;
    let lots = r.w.engine().action_state();
    check!(
        lots.get(&f.lots, &r.lot)?.is_none(),
        "the laser lot is consumed"
    );
    let order = r.order(id)?;
    let returned = order.intent.offer.returned.ok_or("a returned lot")?;
    let cannon = lots
        .get(&f.lots, &returned)?
        .ok_or("the cannon in stores")?;
    check_eq!(cannon.commodity().as_str(), "part.bay_cannon");
    check_eq!(cannon.endpoint(), r.stores);
    r.restart()?;
    check_eq!(r.unit()?.schema().key().id.as_str(), "bay_unit_b");
    check!(r.order(id)?.installation.is_some());
    Ok(())
}

/// **Cancel releases everything it held**: the fee, the slot, the hull and
/// the laser lot, which is free for the next order.
#[test]
fn ud4b_a_cancelled_bay_order_releases_the_fee_the_hull_and_the_part() -> Result {
    let mut r = Rig::new()?;
    let id = r.accepted()?;
    r.commit(5, r.requester, 22, json!({"order":id.to_string()}))?;
    check_eq!(r.balance(r.payer)?, (1_000_000, 0), "the fee is released");
    check_eq!(r.unit()?.schema().key().id.as_str(), "bay_unit_a");
    let f = r.inventory()?;
    check!(
        r.w.engine().action_state().get(&f.lots, &r.lot)?.is_some(),
        "the laser lot stays in stores"
    );
    let hulls = r.runtime()?.families.hulls().clone();
    let head =
        r.w.engine()
            .action_state()
            .get(&hulls, &r.hull)?
            .ok_or("hull head")?
            .clone();
    check_eq!(head.pending(), None, "the hull is out of the bay");
    r.restart()?;
    check!(r.order(id)?.terminal.is_some());
    Ok(())
}
