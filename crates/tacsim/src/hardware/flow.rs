#![cfg(test)]

//! Actual shipped content -> Origin -> paid integration -> delivery -> refit.
//! Commands create ordinary setup; every certified/money/physical transition is
//! the real registered provider and prepared ACK union. No synthetic receipt.
use super::*;
use crate::{TacsimConfig, TacsimRuleset, composition::mind_commerce::MindCommerce};
use mush_core::{
    CommitToken, DefinitionRef, GOD, GameModule, ModuleBootData, ModuleOp, ModuleWrite, ObjId,
    ObjKind, OwnerId,
};
use mush_economy::{
    AccountId, Amount, CatalogRef, EconomyModule, EconomyStore,
    core_integration::ScopedCoreAccount, registry_filing::InitialFilingPolicy,
};
use mush_industry::{
    IndustryModule,
    core_production::{CoreOriginMode, DieClass, OriginAgent},
};
use mush_inventory::{Grade, InventoryHandleCell, InventoryModule};
use mush_org::{OrgModule, OrgStore, PrincipalId, SystemPrincipalId};
use mush_roll::{
    RollModule,
    commerce::{filing::InitialFilingTerms, module::CommerceMode},
};
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use std::{collections::HashMap, sync::Arc};
#[path = "flow/driver.rs"]
mod driver;
#[path = "flow/launch.rs"]
mod launch;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Boot = HashMap<SmolStr, ModuleBootData>;
fn layer() -> TacsimRuleset {
    TacsimRuleset::with_config(TacsimConfig {
        enabled: true,
        ..Default::default()
    })
}
fn filing(deployment: &HardwareDeployment) -> Result<InitialFilingPolicy> {
    Ok(InitialFilingPolicy::new(
        deployment.issuer.clone(),
        deployment.integrator.account,
        Amount::from_minor(0),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?)
}
fn modules(
    deployment: &HardwareDeployment,
    mode: CoreOriginMode,
    registry: Option<&mush_roll::commerce::registry_inspection::RegistryInspectionPolicy>,
) -> Result<Vec<Box<dyn GameModule>>> {
    let native = filing(deployment)?;
    let hardware = GovernedHardware::prepare(&layer(), mode, deployment.clone(), &native)?;
    let handles: InventoryHandleCell = Arc::default();
    let mut owners = hardware.compose(
        layer(),
        EconomyModule::new().with_title_issuers(vec![deployment.issuer.clone()]),
        InventoryModule::new().with_quantity_delegates(["industry", "tacsim"]),
        IndustryModule::new(),
        handles,
    )?;
    let commerce = MindCommerce::prepare(
        &owners.tacsim,
        if mode == CoreOriginMode::Initialize {
            CommerceMode::Initialize
        } else {
            CommerceMode::Active
        },
        InitialFilingTerms::new(90 * 86400, 3 * 86400)?,
        native,
    )?;
    // Doc 17h cut M8d2: the Registry's inspection and seizure, when a test names the Registry
    // (Active only), with the minds facts `minds.seized` resolves through.
    let commerce = match (registry, mode) {
        (Some(policy), CoreOriginMode::Active) => commerce
            .with_registry_inspection(policy.clone())?
            .with_registry_seizure(SEIZURE_TERMS)?
            .with_minds_facts()?,
        _ => commerce,
    };
    owners.tacsim = commerce.configure_tacsim(owners.tacsim)?;
    owners.economy = commerce.configure_economy(owners.economy)?;
    Ok(vec![
        Box::new(OrgModule::new()),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(owners.economy),
        Box::new(owners.inventory),
        Box::new(owners.industry),
        Box::new(commerce.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(owners.tactics),
        Box::new(owners.tacsim),
    ])
}
fn command(w: &mut TestWorld, actor: ObjId, line: &str) -> Result {
    let out = w.run(actor, line);
    let refusals: Vec<String> = out
        .iter()
        .filter_map(|(_, e)| match e {
            mush_proto::OutputEvent::Sys(t)
                if t.spans().iter().any(|s| s.tag.as_deref() == Some("error")) =>
            {
                Some(t.to_plain_text())
            }
            _ => None,
        })
        .collect();
    if !refusals.is_empty() {
        let verb = line.split_whitespace().next().unwrap_or_default();
        return Err(format!(
            "actual hardware setup command refused: {verb}: {}",
            refusals.join(" | ")
        )
        .into());
    }
    Ok(())
}
struct Rig {
    w: Option<TestWorld>,
    boot: Boot,
    deployment: HardwareDeployment,
    buyer: ObjId,
    payer: AccountId,
    serial: u128,
    /// Doc 17h cut M8d2: the Registry's inspection policy, once a test names the Registry.
    registry: Option<mush_roll::commerce::registry_inspection::RegistryInspectionPolicy>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(w) = &self.w
            && let Some(r) = w
                .world()
                .module_state::<crate::arena_bridge::ArenaRegistry>()
        {
            r.tasks().signal_stop_all();
        }
    }
}
impl Rig {
    fn world(&self) -> &TestWorld {
        self.w.as_ref().expect("world owned")
    }
    fn world_mut(&mut self) -> &mut TestWorld {
        self.w.as_mut().expect("world owned")
    }
    fn flush(&mut self) {
        let ops = self.world_mut().take_module_ops();
        fold_module_ops_into(&mut self.boot, ops);
        self.world_mut().world_mut().take_dirty();
    }
    fn reboot(&mut self, mode: CoreOriginMode) -> Result {
        self.flush();
        if let Some(r) = self
            .world()
            .world()
            .module_state::<crate::arena_bridge::ArenaRegistry>()
        {
            r.tasks().signal_stop_all();
        }
        let modules = modules(&self.deployment, mode, self.registry.as_ref())?;
        self.restart(modules)?;
        self.flush();
        Ok(())
    }
    #[inline(never)]
    fn restart(&mut self, modules: Vec<Box<dyn GameModule>>) -> Result {
        let mut world = self.w.take().ok_or("world owned")?.into_engine().world;
        let connected: Vec<_> = world
            .iter()
            .map(|o| o.id)
            .filter(|id| world.is_connected(*id))
            .collect();
        for actor in connected {
            world.set_connected(actor, false);
        }
        let (_, registry) = mush_testkit::boot_world();
        let mut engine = mush_core::Engine::with_modules(world, registry, modules)?;
        let mut boot = self.boot.clone();
        let mut typed = Vec::new();
        // Match the production store loader: authoritative typed envelopes are
        // hydrated before independent owner documents reach their boot hooks.
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
        engine.hydrate_action_state(typed)?;
        engine.modules_loaded(
            boot,
            &mut mush_core::Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        self.w = Some(TestWorld::from_engine(engine));
        Ok(())
    }
    fn provision(extra: Vec<Box<dyn GameModule>>) -> Result<Self> {
        let handles: InventoryHandleCell = Arc::default();
        let mut modules: Vec<Box<dyn GameModule>> = vec![
            Box::new(OrgModule::new()),
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(EconomyModule::new()),
            Box::new(
                InventoryModule::new()
                    .with_quantity_delegates(["industry", "tacsim"])
                    .with_handle_cell(handles.clone()),
            ),
            Box::new(IndustryModule::new().with_inventory(handles.clone())),
            Box::new(layer().with_inventory(handles)),
        ];
        modules.extend(extra);
        let mut w = TestWorld::with_modules(modules);
        w.world_mut().set_now(1000);
        w.world_mut().set_game_clock(mush_core::clock::GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        });
        let buyer = w.spawn_player("Hardware buyer");
        let agent = w.spawn_player("Hardware stock agent");
        let factory = w
            .world_mut()
            .create(ObjKind::Thing, "Basin fitting shop", GOD, None)?;
        let output = w
            .world_mut()
            .create(ObjKind::Thing, "Basin completed work", GOD, None)?;
        let containers = [
            w.world_mut()
                .create(ObjKind::Thing, "Basin die depot", GOD, None)?,
            w.world_mut()
                .create(ObjKind::Thing, "Basin agent stock one", GOD, None)?,
            w.world_mut()
                .create(ObjKind::Thing, "Basin agent stock two", GOD, None)?,
        ];
        for line in [
            "@org/create Basin",
            "@org/scope create Basin=die-stock",
            "@org/scope create Basin=factory",
            "@instrument/register tacsim.credit=2",
        ] {
            command(&mut w, GOD, line)?;
        }
        let orgs = w
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org owner")?
            .state();
        let org = orgs
            .orgs()
            .find(|o| o.name.as_str() == "Basin")
            .ok_or("actual Basin organization")?
            .id;
        let scope = |name: &str| {
            orgs.scopes_in(org)
                .find(|s| s.name.as_str() == name)
                .map(|s| s.id)
                .ok_or("actual scope")
        };
        let stock_scope = scope("die-stock")?;
        let factory_scope = scope("factory")?;
        for (i, c) in containers.into_iter().enumerate() {
            command(
                &mut w,
                GOD,
                &format!(
                    "@vault/create HardwareStock{i}=container:#{} owner=org:Basin scope={}",
                    c.0, stock_scope.0
                ),
            )?;
        }
        for scope in [stock_scope, factory_scope] {
            command(
                &mut w,
                GOD,
                &format!("@account/open org:Basin/{}=tacsim.credit", scope.0),
            )?;
        }
        command(
            &mut w,
            GOD,
            &format!("@account/open #{}=tacsim.credit", buyer.0),
        )?;
        command(
            &mut w,
            GOD,
            "@account/system hardware.fixture=tacsim.credit",
        )?;
        let e = w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy owner")?;
        let snapshot = e.snapshot();
        let vault = |name: &str| {
            snapshot
                .vaults()
                .find(|v| v.name.as_str() == name)
                .map(|v| v.id)
                .ok_or("actual stock vault")
        };
        let depot = vault("HardwareStock0")?;
        let stock1 = vault("HardwareStock1")?;
        let stock2 = vault("HardwareStock2")?;
        let account = |scope| {
            snapshot
                .accounts()
                .find(|a| a.owner == PrincipalId::Organization(org) && a.scope == Some(scope))
                .map(|a| a.id)
                .ok_or("actual treasury")
        };
        let supplier = account(stock_scope)?;
        let integrator = account(factory_scope)?;
        check_eq!(
            snapshot
                .account(supplier)
                .ok_or("supplier")?
                .instrument
                .as_str(),
            crate::economy::CREDIT
        );
        check_eq!(
            snapshot
                .account(integrator)
                .ok_or("integrator")?
                .instrument
                .as_str(),
            crate::economy::CREDIT
        );
        let payer = snapshot
            .accounts()
            .find(|a| a.owner == PrincipalId::Object(buyer))
            .ok_or("actual payer")?
            .id;
        let faucet = snapshot
            .accounts()
            .find(|a| matches!(&a.owner,PrincipalId::System(s)if s.as_str()=="hardware.fixture"))
            .ok_or("actual funded system")?
            .id;
        command(
            &mut w,
            GOD,
            &format!("@econ/mint {}={} 1000000", faucet.0, payer.0),
        )?;
        command(
            &mut w,
            GOD,
            &format!("@facility/open {}=2 core_integration core_refit", factory.0),
        )?;
        command(
            &mut w,
            GOD,
            &format!("@facility/owner {}=org:Basin", factory.0),
        )?;
        command(
            &mut w,
            GOD,
            &format!(
                "@asset/title #{}=org:Basin scope={}",
                factory.0, factory_scope.0
            ),
        )?;
        for actor in [GOD, agent] {
            for cap in ["industry.core_die.issue", "vault.deposit", "vault.withdraw"] {
                command(
                    &mut w,
                    GOD,
                    &format!(
                        "@org/grant #{}=Basin {cap} scope=#{}",
                        actor.0, stock_scope.0
                    ),
                )?;
            }
            for cap in ["industry.core.integrate", "industry.core.refit"] {
                command(
                    &mut w,
                    GOD,
                    &format!(
                        "@org/grant #{}=Basin {cap} scope=#{}",
                        actor.0, factory_scope.0
                    ),
                )?;
            }
        }
        // Doc 17h cut M8b C2: an ungoverning shop beside the chartered one, its own
        // organization paid, holding `core_ungovern` and not `core_refit`.
        let shop = w
            .world_mut()
            .create(ObjKind::Thing, "Back-room shop", GOD, None)?;
        let shop_output =
            w.world_mut()
                .create(ObjKind::Thing, "Back-room completed work", GOD, None)?;
        for line in ["@org/create Tether", "@org/scope create Tether=backroom"] {
            command(&mut w, GOD, line)?;
        }
        let tether = w
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org owner")?
            .state()
            .orgs()
            .find(|o| o.name.as_str() == "Tether")
            .ok_or("actual Tether organization")?
            .id;
        let backroom = w
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org owner")?
            .state()
            .scopes_in(tether)
            .find(|s| s.name.as_str() == "backroom")
            .map(|s| s.id)
            .ok_or("actual back-room scope")?;
        command(
            &mut w,
            GOD,
            &format!("@account/open org:Tether/{}=tacsim.credit", backroom.0),
        )?;
        let shop_account = w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy owner")?
            .snapshot()
            .accounts()
            .find(|a| a.owner == PrincipalId::Organization(tether))
            .map(|a| a.id)
            .ok_or("actual back-room treasury")?;
        for line in [
            format!("@facility/open {}=2 core_ungovern", shop.0),
            format!("@facility/owner {}=org:Tether", shop.0),
            format!("@asset/title #{}=org:Tether scope={}", shop.0, backroom.0),
        ] {
            command(&mut w, GOD, &line)?;
        }
        for actor in [GOD, agent] {
            command(
                &mut w,
                GOD,
                &format!(
                    "@org/grant #{}=Tether industry.core.ungovern scope=#{}",
                    actor.0, backroom.0
                ),
            )?;
        }
        let deployment = HardwareDeployment {
            ungovern: Some(super::UngoverningSite {
                provider: ScopedCoreAccount {
                    account: shop_account,
                    organization: tether,
                    scope: backroom,
                },
                factory: shop,
                output_container: shop_output,
                // Doc 17h cut M8b G3: unequal to the fitting fee, so the settlement is
                // proved to credit the recipe's own fee.
                fee_minor: Some(SHOP_FEE),
            }),
            importer: org,
            stock_scope,
            depot,
            agents: [
                OriginAgent {
                    actor: GOD,
                    stock: stock1,
                },
                OriginAgent {
                    actor: agent,
                    stock: stock2,
                },
            ],
            grades: [Grade::new(10)?, Grade::new(20)?, Grade::new(30)?],
            supplier_account: supplier,
            integrator: ScopedCoreAccount {
                account: integrator,
                organization: org,
                scope: factory_scope,
            },
            issuer: SystemPrincipalId::new("hardware.factory").ok_or("issuer")?,
            factory,
            output_container: output,
        };
        let boot = fold_module_ops(w.take_module_ops());
        w.world_mut().take_dirty();
        Ok(Self {
            w: Some(w),
            boot,
            deployment,
            buyer,
            payer,
            serial: 100,
            registry: None,
        })
    }
    fn new() -> Result<Self> {
        Self::new_with(Vec::new(), |_| Ok(()))
    }
    /// [`Rig::new`] with `extra` modules composed for the provisioning boot only, and `setup`
    /// run while they are (doc 17h cut M8b-2: the Class III end-user entitlement fixture).
    fn new_with(extra: Vec<Box<dyn GameModule>>, setup: fn(&mut Self) -> Result) -> Result<Self> {
        let mut r = Self::provision(extra)?;
        setup(&mut r)?;
        r.flush();
        let agent = r.deployment.agents[1].actor;
        r.reboot(CoreOriginMode::Initialize)?;
        r.initialize_roll()?;
        for action in [
            "initialize_core_origin",
            "initialize_core_production",
            "initialize_core_refit",
        ] {
            r.perform("industry", action, GOD, json!({}))?;
        }
        check!(
            r.plan("industry", "issue_core_die", agent, json!({"class":"i"}))
                .is_err()
        );
        r.reboot(CoreOriginMode::Active)?;
        Ok(r)
    }
    fn money(&self, id: AccountId) -> Result<(i64, i64)> {
        let account = self
            .world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy")?
            .snapshot()
            .account(id)
            .ok_or("account")?;
        Ok((account.posted_balance.minor(), account.held_amount.minor()))
    }
    fn rows(&self, family: &str) -> Vec<Value> {
        self.boot
            .values()
            .flat_map(|d| d.kv.iter())
            .filter(|(k, v)| {
                mush_core::is_action_state_persistence_key(k)
                    && v["family"]["id"].as_str() == Some(family)
            })
            .filter_map(|(_, v)| v.get("value").cloned())
            .collect()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shipped_origin_manufacture_delivery_refit_uses_real_signed_acknowledgements() -> Result {
    let mut r = Rig::new()?;
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let money = [
        r.money(r.payer)?,
        r.money(r.deployment.supplier_account)?,
        r.money(r.deployment.integrator.account)?,
    ];
    let (_, die) = r.perform("industry", "issue_core_die", agent, json!({"class":"i"}))?;
    let lot = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
    let (offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":"i","supplier_lot":lot.to_string()}),
    )?;
    let (accept, order) = r.perform(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
    )?;
    let id =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(order["order"].clone())?
            .to_string();
    let params = json!({"order":id});
    check_eq!(r.money(r.payer)?, (money[0].0, money[0].1 + 300_000));
    check!(
        r.plan(
            "industry",
            "complete_core_integration",
            buyer,
            params.clone()
        )
        .is_err()
    );
    r.perform("industry", "reserve_core_output", buyer, params.clone())?;
    let accepted = r.world().world().world_ic_now();
    r.progress("advance_core_integration", 12)?;
    r.reboot(CoreOriginMode::Active)?;
    r.progress("advance_core_integration", 12)?;
    check_eq!(r.world().world().world_ic_now().at - accepted.at, 24 * 3600);
    let (_, completed) = r.perform(
        "industry",
        "complete_core_integration",
        buyer,
        params.clone(),
    )?;
    let output = serde_json::from_value::<ObjId>(completed["object"].clone())?;
    check_eq!(
        r.money(r.deployment.supplier_account)?.0,
        money[1].0 + 270_000
    );
    check_eq!(
        r.money(r.deployment.integrator.account)?.0,
        money[2].0 + 30_000
    );
    r.perform("industry", "deliver_core_output", buyer, params)?;
    check_eq!(
        r.world()
            .world()
            .get(output)
            .ok_or("delivered core")?
            .location,
        Some(buyer)
    );
    let native = mush_economy::core_integration::require_core_delivery(r.world().world(), accept)?;
    check_eq!(native.subject().requester, buyer);
    let factory = r.deployment.factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull = r.world_mut().world_mut().create(
        ObjKind::Thing,
        "Actual governed line hull",
        buyer,
        Some(factory),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@unit/create #{}=tacsim.unit.line_tank", hull.0),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, buyer.0),
    )?;
    // Shipped unit definitions have section cells, not a unit-level heat pool.
    // Damage an actual non-core cell and prove that refitting preserves it.
    let hull_internals = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull")?,
        cell: mush_tactics::CellId::new(crate::definitions::unit::INTERNALS_CELL)?,
    };
    let pristine = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("native line unit before damage")?;
    let intact = pristine
        .cell(&hull_internals)
        .ok_or("actual hull internals")?;
    check_eq!(intact.current, intact.max);
    let damaged_internals = intact.current.checked_sub(1).ok_or("bounded hull damage")?;
    check!(damaged_internals > 0 && damaged_internals >= intact.min);
    let damage = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: hull_internals.clone(),
            value: damaged_internals,
        }],
        Some(pristine.revision()),
        Some("retained actual hull internals damage".into()),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@unit/damage #{}={}",
            hull.0,
            serde_json::to_string(&damage)?
        ),
    )?;
    r.flush();
    let before = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("native line unit")?
        .clone();
    check_eq!(
        before.cell(&hull_internals).map(|cell| cell.current),
        Some(damaged_internals)
    );
    let (offer,_)=r.perform("industry","offer_core_refit",agent,json!({"requester":buyer.0,"hull":hull.0,"incoming":output.0,"manufacture":accept.to_hex(),"class":"i"}))?;
    let (_, order) = r.perform(
        "industry",
        "accept_core_refit",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
    )?;
    let id =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(order["order"].clone())?
            .to_string();
    check!(
        r.plan(
            "industry",
            "install_core_component",
            buyer,
            json!({"order":id})
        )
        .is_err()
    );
    r.progress("advance_core_refit", 1)?;
    r.perform(
        "industry",
        "install_core_component",
        buyer,
        json!({"order":id}),
    )?;
    let after = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("native fitted unit")?
        .clone();
    check_eq!(
        after.cell(&hull_internals).map(|cell| cell.current),
        Some(damaged_internals)
    );
    check_eq!(
        after.schema().key().id.as_str(),
        crate::definitions::core_replacement::LINE_I
    );
    check_eq!(after.object(), before.object());
    for (address, cell) in before.cells() {
        if !matches!(address, mush_tactics::CellAddr::Component { component, .. } if component.as_str() == "core")
        {
            check_eq!(after.cell(address), Some(cell));
        }
    }
    for (component, state) in before.components() {
        if component.as_str() != "core" {
            check_eq!(
                after
                    .components()
                    .find(|(id, _)| *id == component)
                    .map(|(_, value)| value),
                Some(state)
            );
        }
    }
    check_eq!(r.money(r.payer)?.0, money[0].0 - 310_000);
    check_eq!(
        r.money(r.deployment.integrator.account)?.0,
        money[2].0 + 40_000
    );
    check_eq!(r.money(r.payer)?.1, money[0].1);
    check!(!r.rows("core_component_exchanges").is_empty());
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(
        mush_tactics::module::unit_of(r.world().world(), hull),
        Some(&after)
    );
    check!(
        mush_economy::core_integration::require_core_delivery(r.world().world(), accept).is_ok()
    );
    // Doc 17h cut M8a (A6; the brief's test 5; the fix round's F6): the delivered core's title
    // was born of this integration, and the retained die certificate its funding named agrees
    // (its operation, lot and class), so the core's provenance is `Certified` by the
    // certificate's importer at its class (Class I: the flow's die) — read after delivery,
    // when the die lot is spent. The inspection's facts over the refitted hull read that core,
    // Class I, governed, certified, with no trip open, and the receipt says so in one line.
    // The hull is the title's object, not the core: unknown. (The rig has no port maps, so
    // the read is the gate-free `read_unit`.)
    {
        use crate::minds::registry_inspection::facts::{StoreRead, read_unit as read};
        use crate::minds::registry_inspection::{HullClass, Provenance, SealCheck};
        let engine = r.world().engine();
        let world = &engine.world;
        let mut reader = StoreRead(engine.action_state());
        let certified = Provenance::Certified {
            importer: r.deployment.importer,
            grade: mush_economy::core_integration::CoreClass::I,
        };
        check_eq!(
            crate::minds::provenance::provenance_of(world, &mut reader, output),
            certified
        );
        check_eq!(
            crate::minds::provenance::provenance_of(world, &mut reader, hull),
            Provenance::Unknown
        );
        // F6: the economy's birth of the output, read directly — this integration, the die
        // the funding named, Class I — and none for the hull.
        let economy = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or("the economy")?;
        let birth = economy
            .core_title_birth(output)
            .map_err(|e| format!("{e:?}"))?
            .ok_or("the output's birth")?;
        check_eq!(birth.object, output);
        check_eq!(birth.class, mush_economy::core_integration::CoreClass::I);
        check_eq!(
            economy
                .core_title_birth(hull)
                .map_err(|e| format!("{e:?}"))?,
            None
        );
        // F6: the certificate is the proof. The retained one agrees; one that names another
        // lot or class than the birth, or none at all, is `Unknown` — never agreement.
        let certificates = world
            .module_state::<mush_industry::core_production::DieCertificates>()
            .ok_or("the certificate read is granted")?;
        let certificate = engine
            .action_state()
            .get(certificates.handle(), &birth.die_certificate.to_hex())
            .map_err(|e| format!("{e:?}"))?
            .cloned()
            .ok_or("the retained certificate")?;
        check_eq!(
            crate::minds::provenance::from_certificate(&birth, Some(&certificate)),
            certified
        );
        let mut other_lot = birth.clone();
        other_lot.die_lot = other_lot.die_lot.wrapping_add(1);
        let mut other_class = birth.clone();
        other_class.class = mush_economy::core_integration::CoreClass::II;
        for disagreeing in [&other_lot, &other_class] {
            check_eq!(
                crate::minds::provenance::from_certificate(disagreeing, Some(&certificate)),
                Provenance::Unknown
            );
        }
        check_eq!(
            crate::minds::provenance::from_certificate(&birth, None),
            Provenance::Unknown
        );
        let found = read(world, &mut reader, hull).ok_or("the refitted hull reads")?;
        check_eq!(found.facts.core, Some(output));
        check_eq!(found.facts.class, HullClass::I);
        check_eq!(found.facts.governed, Some(true));
        check_eq!(found.facts.provenance, certified);
        check_eq!(found.facts.seal_check, SealCheck::NoSeal);
        check_eq!(found.seal, None);
        check_eq!(
            crate::minds::registry_inspection::facts::receipt(world, &found.facts, false),
            format!(
                "Hull #{}: Class I governed · provenance certified (Basin, I) · no trip open",
                hull.0
            )
        );
    }
    Ok(())
}

/// The back-room shop's fee in the test rig (doc 17h cut M8b G3): not the fitting fee.
const SHOP_FEE: i64 = 25_000;

/// One Class I core, integrated from a certified die and delivered to the buyer: the
/// acceptance's commit and the core.
fn integrate_class_one(r: &mut Rig) -> Result<(CommitToken, ObjId)> {
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let (_, die) = r.perform("industry", "issue_core_die", agent, json!({"class":"i"}))?;
    let lot = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
    let (offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":"i","supplier_lot":lot.to_string()}),
    )?;
    let (accept, order) = r.perform(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
    )?;
    let id =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(order["order"].clone())?
            .to_string();
    let params = json!({"order":id});
    r.perform("industry", "reserve_core_output", buyer, params.clone())?;
    r.progress("advance_core_integration", 24)?;
    let (_, completed) = r.perform(
        "industry",
        "complete_core_integration",
        buyer,
        params.clone(),
    )?;
    let output = serde_json::from_value::<ObjId>(completed["object"].clone())?;
    r.perform("industry", "deliver_core_output", buyer, params)?;
    Ok((accept, output))
}

/// Refit `hull` with `incoming` (Class I) under `recipe`, the agent offering and the buyer
/// accepting and installing; the hull and the buyer stand in the recipe's facility.
fn refit_class_one(
    r: &mut Rig,
    hull: ObjId,
    incoming: ObjId,
    accept: CommitToken,
    recipe: Option<&str>,
) -> Result {
    refit_class(r, hull, incoming, accept, DieClass::I, recipe)
}

/// [`refit_class_one`] at any class.
fn refit_class(
    r: &mut Rig,
    hull: ObjId,
    incoming: ObjId,
    accept: CommitToken,
    class: DieClass,
    recipe: Option<&str>,
) -> Result {
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let mut params = json!({"requester":buyer.0,"hull":hull.0,"incoming":incoming.0,
        "manufacture":accept.to_hex(),"class":class});
    if let Some(recipe) = recipe {
        params["recipe"] = json!(recipe);
    }
    let (offer, _) = r.perform("industry", "offer_core_refit", agent, params)?;
    let (_, order) = r.perform(
        "industry",
        "accept_core_refit",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
    )?;
    let id =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(order["order"].clone())?
            .to_string();
    // The shipped fitting works 1, 2 and 4 hours by class.
    r.progress("advance_core_refit", [1, 2, 4][class.index()])?;
    r.perform(
        "industry",
        "install_core_component",
        buyer,
        json!({"order":id}),
    )?;
    Ok(())
}

/// The ROM seal an IC epoch would take over `hull` now, under the shipped minds defaults, the
/// impounded read taken from the rig's economy as the epoch and launch paths take it.
fn rom_seal(r: &Rig, hull: ObjId) -> Result<crate::arena::mind::MindSeal> {
    let unit = mush_tactics::module::unit_of(r.world().world(), hull).ok_or("the rig's unit")?;
    let impounded = crate::match_::seal::core_impounded(r.world().world(), unit);
    rom_seal_with(r, hull, impounded)
}

/// [`rom_seal`] with the impounded read given (doc 17h cut M8d2: the seal over a seized core,
/// until `@registry/seize` composes a real seizure in this rig).
fn rom_seal_with(r: &Rig, hull: ObjId, impounded: bool) -> Result<crate::arena::mind::MindSeal> {
    let unit = mush_tactics::module::unit_of(r.world().world(), hull).ok_or("the rig's unit")?;
    let minds = crate::config::MindsConfig::default();
    let reflex = crate::config::ReflexConfig::default();
    Ok(crate::match_::seal::seal_rom_for(
        hull,
        unit,
        &[],
        crate::arena::mind::SortieId::ic(hull, 1_000),
        &crate::match_::seal::IcSealConfig {
            minds: &minds,
            reflex: &reflex,
            principal: None,
            impounded,
        },
    )?)
}

/// **Doc 17h cut M8b, the gate's tests 1 and 2** (C1, C2, G2): a stock, coreless terrain
/// vehicle is fitted at the chartered shop (Stickney's "Basin") with a certified Class I core
/// — `core_refit` fits it into the empty mount, its root becomes the rig family's Class I
/// with no core displaced, it seals Class I governed, and
/// M8a's inspection reads the core `certified`. Moved to the back-room shop (a facility
/// holding `core_ungovern` and not `core_refit`, its own organization paid the
/// `ungovern_minor` fee), the ungoverning refit fits a second certified core as the ungoverned
/// twin: `read_core` reads ungoverned, the economy carries the provenance note on that core's
/// title (the shop, the completion's IC second), the inspection record's `ungoverned_note`
/// says so, its provenance is still `certified` (the birth is the provenance; the note is the
/// crime), and the next seal's line is `governed: false`. A facility declaring both recipes is
/// refused at open.
#[tokio::test(flavor = "current_thread")]
async fn m8b_a_rig_is_fitted_at_stickney_then_ungoverned_in_the_back_room() -> Result {
    use crate::definitions::core_replacement::{RIG_I, RIG_I_UNGOVERNED};
    use crate::minds::registry_inspection::facts::{StoreRead, read_unit as read};
    use crate::minds::registry_inspection::{HullClass, Provenance, UngovernedNote};
    let mut r = Rig::new()?;
    let buyer = r.buyer;
    let (first_accept, first) = integrate_class_one(&mut r)?;
    let (second_accept, second) = integrate_class_one(&mut r)?;
    let factory = r.deployment.factory;
    let shop = r
        .deployment
        .ungovern
        .as_ref()
        .ok_or("the back-room shop")?
        .clone();
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull =
        r.world_mut()
            .world_mut()
            .create(ObjKind::Thing, "Prospector rig", buyer, Some(factory))?;
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@unit/create #{}={}",
            hull.0,
            crate::definitions::content::TV_PROSPECTOR
        ),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, buyer.0),
    )?;
    r.flush();
    // The shipped rig is coreless (Class 0, the ROM).
    check_eq!(
        mush_tactics::module::unit_of(r.world().world(), hull)
            .and_then(crate::arena::core::read_core),
        None
    );
    // What the surface reads of the stock rig: its drive rates and its hopper (G1).
    let surface = |r: &Rig| -> Result<_> {
        let w = r.world().world();
        let unit = crate::port::hull::live_unit_id(w, hull).ok_or("the rig's chassis")?;
        check_eq!(unit.as_str(), crate::definitions::content::TV_PROSPECTOR);
        let store = w.module_state::<crate::TacsimStore>().ok_or("the store")?;
        let (_, logistics) =
            crate::port::hull::spec_for(store, &unit).ok_or("the rig's interior and hopper")?;
        // The act layer's own entry points (doc 17h cut M8b G1, option (b)): the drive's
        // rates, the hopper's compartments the dump and the mine fill, and the payload read
        // a carrier's bay counts the rig by (`carried_rigs`' per-rig read).
        let carried = crate::port::vehicle::carried_rig(
            w,
            store,
            hull,
            None,
            None,
            &crate::definitions::goods::goods_table()?,
        )?;
        Ok((
            crate::surface::act::rig_rates(w, hull, None).ok_or("the rig's drive rates")?,
            logistics.clone(),
            crate::port::tv::hopper_compartments(w, hull, None).ok_or("the rig's hopper")?,
            (carried.dry_kg, carried.goods),
        ))
    };
    let stock = surface(&r)?;
    check_eq!(stock.3.0, 6_000, "the stock rig's dry mass");
    check!(!stock.3.1.is_empty(), "the stock rig's hopper takes ore");
    // The chartered fitting into the coreless base (doc 17h cut M8b G2, the lead's ruling):
    // the rig's Class I root, governed, certified, nothing displaced.
    refit_class_one(&mut r, hull, first, first_accept, None)
        .map_err(|e| format!("the chartered fitting: {e}"))?;
    let fitted = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("the fitted rig")?
        .clone();
    check_eq!(fitted.schema().key().id.as_str(), RIG_I);
    check_eq!(
        crate::arena::core::read_core(&fitted).map(|hw| (hw.class, hw.governed)),
        Some((crate::arena::core::CoreClass::I, true))
    );
    check_eq!(
        surface(&r)?,
        stock,
        "a fitted rig drives and mines as the stock one"
    );
    let seal = rom_seal(&r, hull)?;
    check_eq!(
        (seal.core_class, seal.class, seal.governed),
        (
            crate::arena::core::CoreClass::I,
            crate::arena::core::CoreClass::I,
            true
        )
    );
    // Doc 17h cut M8d2: a governed rig no seizure holds reads unimpounded.
    check!(!crate::match_::seal::core_impounded(
        r.world().world(),
        &fitted
    ));
    check!(
        seal.admissions(crate::minds::licence_state::SealLicenceState::Doctrine)
            .is_empty()
    );
    let certified = Provenance::Certified {
        importer: r.deployment.importer,
        grade: mush_economy::core_integration::CoreClass::I,
    };
    {
        let engine = r.world().engine();
        let mut reader = StoreRead(engine.action_state());
        let found = read(&engine.world, &mut reader, hull).ok_or("the fitted rig reads")?;
        check_eq!(found.facts.core, Some(first));
        check_eq!(found.facts.class, HullClass::I);
        check_eq!(found.facts.governed, Some(true));
        check_eq!(found.facts.provenance, certified.clone());
        check_eq!(found.facts.ungoverned_note, None);
    }
    // A facility declaring both recipes is refused at open.
    let both = r
        .world_mut()
        .world_mut()
        .create(ObjKind::Thing, "Two-faced shop", GOD, None)?;
    check!(
        command(
            r.world_mut(),
            GOD,
            &format!("@facility/open {}=2 core_refit core_ungovern", both.0),
        )
        .is_err_and(|refused| refused
            .to_string()
            .contains("a facility runs core_refit or core_ungovern, never both"))
    );
    // The ungoverning refit, in the back room.
    let money = (r.money(r.payer)?, r.money(shop.provider.account)?);
    r.world_mut().world_mut().move_to(buyer, shop.factory)?;
    r.world_mut().world_mut().move_to(hull, shop.factory)?;
    r.flush();
    refit_class_one(&mut r, hull, second, second_accept, Some("core_ungovern"))
        .map_err(|e| format!("the ungoverning refit: {e}"))?;
    let ungoverned = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("the ungoverned rig")?
        .clone();
    check_eq!(ungoverned.schema().key().id.as_str(), RIG_I_UNGOVERNED);
    check_eq!(
        surface(&r)?,
        stock,
        "an ungoverned rig drives and mines as the stock one"
    );
    check_eq!(
        crate::arena::core::read_core(&ungoverned).map(|hw| (hw.class, hw.governed)),
        Some((crate::arena::core::CoreClass::I, false))
    );
    // C7: the catalogue's `ungovern_minor` is provisionally the fitting fee; this shop
    // charges its own (G3), and exactly that moves.
    check_eq!(
        PublishedHardware::shipped()?.catalogue().ungovern_minor(),
        mush_economy::core_refit::FITTING_MINOR
    );
    let fee = SHOP_FEE;
    check!(fee != mush_economy::core_refit::FITTING_MINOR);
    check_eq!(r.money(r.payer)?.0, money.0.0 - fee);
    check_eq!(r.money(shop.provider.account)?.0, money.1.0 + fee);
    let note = r
        .world()
        .world()
        .module_state::<EconomyStore>()
        .ok_or("the economy")?
        .core_provenance_note(second)?
        .ok_or("the provenance note")?;
    let mush_economy::CoreProvenanceNote::Ungoverned { facility, at_ic } = note;
    check_eq!(facility, shop.factory);
    check!(at_ic > 0);
    check_eq!(
        r.world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("the economy")?
            .core_provenance_note(first)?,
        None
    );
    {
        let engine = r.world().engine();
        let mut reader = StoreRead(engine.action_state());
        let found = read(&engine.world, &mut reader, hull).ok_or("the ungoverned rig reads")?;
        check_eq!(found.facts.core, Some(second));
        check_eq!(found.facts.governed, Some(false));
        check_eq!(found.facts.provenance, certified);
        check_eq!(
            found.facts.ungoverned_note,
            Some(UngovernedNote {
                facility: shop.factory,
                at_ic
            })
        );
    }
    let seal = rom_seal(&r, hull)?;
    check!(!seal.governed && seal.lifts_floor());
    // Doc 17h cut M8d2 ("impounded in place"): the same fitted ungoverned rig, its core
    // impounded (held by a seizure, or the Registry's after a forfeit), seals the ROM: no
    // core, Class 0, governed, the no-core fallback, no floor lifted. The core stays fitted.
    check!(!crate::match_::seal::core_impounded(
        r.world().world(),
        &ungoverned
    ));
    let held = rom_seal_with(&r, hull, true)?;
    check_eq!(
        (
            held.core.clone(),
            held.core_class,
            held.class,
            held.governed
        ),
        (
            None,
            crate::arena::core::CoreClass::Zero,
            crate::arena::core::CoreClass::Zero,
            true
        )
    );
    check!(!held.lifts_floor());
    check_eq!(
        held.fallback.as_deref(),
        Some(crate::match_::seal::NO_CORE_FALLBACK)
    );
    check_eq!(held.engine, crate::arena::mind::SealEngine::Native);
    check!(crate::arena::core::read_core(&ungoverned).is_some_and(|hw| !hw.governed));
    match seal.sealed_line(crate::minds::licence_state::SealLicenceState::Doctrine) {
        crate::arena::ledger::EventKind::MindSealed {
            governed,
            admissions,
            ..
        } => {
            check!(!governed);
            check!(admissions.is_empty(), "Standard Doctrine admits nothing");
        }
        other => return Err(format!("a sealed line: {other:?}").into()),
    }
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(
        mush_tactics::module::unit_of(r.world().world(), hull),
        Some(&ungoverned)
    );
    check_eq!(
        r.world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("the economy")?
            .core_provenance_note(second)?,
        Some(note)
    );
    Ok(())
}

/// The Class III end-user entitlement (`core3_end_user`), which no shipped composition
/// provisions (doc 17h §2.2): a test-only staff command registering it, composed for the
/// rig's provisioning boot only (`mush-industry`'s delivery fixture's pattern).
const CORE3: &str = "tacsim.licence.core3_end_user";
struct Core3Fixture;
impl GameModule for Core3Fixture {
    fn name(&self) -> &'static str {
        "m8b2core3fixture"
    }
    fn register(
        &mut self,
        reg: &mut mush_core::Registrar<'_>,
    ) -> std::result::Result<(), mush_core::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            mush_core::CommandSpec {
                name: "@core3-fixture".into(),
                shape: mush_core::ArgShape::Raw,
                ..mush_core::CommandSpec::default()
            },
            Arc::new(|ctx: &mut mush_core::Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(mush_core::CmdError::msg("fixture operator"));
                }
                mush_economy::register_instrument(
                    ctx,
                    mush_economy::InstrumentDef {
                        id: mush_economy::InstrumentId::new(CORE3)
                            .ok_or_else(|| mush_core::CmdError::msg("instrument"))?,
                        ruleset: "tacsim".into(),
                        code: "licence.core3_end_user".into(),
                        kind: mush_economy::InstrumentKind::Entitlement,
                        scale: 0,
                        transferable: false,
                        negative_policy: mush_economy::NegativePolicy::SystemOnly,
                        status: mush_economy::InstrumentStatus::Active,
                        display: None,
                        revision: 1,
                    },
                )
                .map_err(|e| mush_core::CmdError::msg(e.to_string()))
            }),
        )?;
        Ok(())
    }
}
/// The buyer holds one Class III end-user entitlement.
fn entitle(r: &mut Rig) -> Result {
    let buyer = r.buyer;
    for line in [
        "@core3-fixture".to_owned(),
        format!("@account/open #{}={CORE3}", buyer.0),
        format!("@account/system core.licence.fixture={CORE3}"),
    ] {
        command(r.world_mut(), GOD, &line)?;
    }
    let (account, faucet) = {
        let e = r
            .world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy owner")?;
        let s = e.snapshot();
        let of = |owner: &dyn Fn(&PrincipalId) -> bool| {
            s.accounts()
                .find(|a| owner(&a.owner) && a.instrument.as_str() == CORE3)
                .map(|a| a.id)
                .ok_or("an entitlement account")
        };
        (
            of(&|o| *o == PrincipalId::Object(buyer))?,
            of(&|o| matches!(o, PrincipalId::System(n) if n.as_str() == "core.licence.fixture"))?,
        )
    };
    command(
        r.world_mut(),
        GOD,
        &format!("@econ/mint {}={} 1", faucet.0, account.0),
    )
}
/// A rig whose buyer holds the Class III entitlement.
fn entitled_rig() -> Result<Rig> {
    Rig::new_with(vec![Box::new(Core3Fixture)], entitle)
}

/// The buyer's own vault, a container they carry: where a staff issue puts a heirloom or grey
/// die for the buyer to bring to the back room.
fn stash(r: &mut Rig) -> Result<mush_economy::VaultId> {
    let buyer = r.buyer;
    let bag =
        r.world_mut()
            .world_mut()
            .create(ObjKind::Thing, "Buyer's satchel", buyer, Some(buyer))?;
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@vault/create BuyerStash=container:#{} owner=#{}",
            bag.0, buyer.0
        ),
    )?;
    let vault = r
        .world()
        .world()
        .module_state::<EconomyStore>()
        .ok_or("Economy owner")?
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == "BuyerStash")
        .map(|v| v.id)
        .ok_or("the buyer's stash")?;
    r.flush();
    Ok(vault)
}

/// GOD's issue of one die under `action` (`issue_heirloom_die`, `issue_grey_die`) into
/// `vault`: the lot.
fn issue(r: &mut Rig, action: &str, params: Value) -> Result<mush_inventory::LotId> {
    let (_, die) = r.perform("industry", action, GOD, params)?;
    Ok(serde_json::from_value::<mush_inventory::LotId>(
        die["lot"].clone(),
    )?)
}

/// One core of `class`, integrated in the back room from the buyer's uncertified `lot` and
/// delivered to the buyer: the acceptance's commit and the core.
fn integrate_uncertified(
    r: &mut Rig,
    lot: mush_inventory::LotId,
    class: DieClass,
) -> Result<(CommitToken, ObjId)> {
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let (offer, _) = r
        .perform(
            "industry",
            "offer_core_integration",
            agent,
            json!({"requester":buyer.0,"class":class,"site":"core_ungovern"}),
        )
        .map_err(|e| format!("the back room's offer: {e}"))?;
    let (accept, order) = r.perform(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string(),"input_lot":lot.to_string()}),
    )
    .map_err(|e| format!("the buyer's acceptance: {e}"))?;
    let id =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(order["order"].clone())?
            .to_string();
    let params = json!({"order":id});
    r.perform("industry", "reserve_core_output", buyer, params.clone())
        .map_err(|e| format!("the output's reservation: {e}"))?;
    // The shipped `core_integrate` works 24, 48 and 96 hours by class.
    r.progress("advance_core_integration", [24, 48, 96][class.index()])
        .map_err(|e| format!("the work: {e}"))?;
    let (_, completed) = r
        .perform(
            "industry",
            "complete_core_integration",
            buyer,
            params.clone(),
        )
        .map_err(|e| format!("the completion: {e}"))?;
    let output = serde_json::from_value::<ObjId>(completed["object"].clone())?;
    // Fix round 1 (L2, D-UM-42): the back-room core is born in the shop's own container,
    // never the chartered factory's.
    let shop = r
        .deployment
        .ungovern
        .as_ref()
        .ok_or("the back-room shop")?
        .output_container;
    let born = r.world().world().get(output).ok_or("the core")?.location;
    check_eq!(born, Some(shop));
    check!(born != Some(r.deployment.output_container));
    r.perform("industry", "deliver_core_output", buyer, params)
        .map_err(|e| format!("the delivery: {e}"))?;
    Ok((accept, output))
}

/// The integration's uniform refusal (doc 17h cut M8b-2 fix round 1, L3).
const MANUFACTURE_REFUSAL: &str = "Core manufacture is unavailable or changed.";
/// The refit's uniform refusal.
const REFIT_REFUSAL: &str = "Core refit is unavailable or changed.";

/// The text of a step that must refuse; an acceptance is the test's failure.
fn refusal_text<T>(result: Result<T>) -> Result<String> {
    match result {
        Ok(_) => Err("accepted where a refusal was expected".into()),
        Err(refused) => Ok(refused.to_string()),
    }
}

/// The economy's birth of `core`: its source and class.
fn birth_of(
    r: &Rig,
    core: ObjId,
) -> Result<(
    mush_economy::core_integration::DieSource,
    mush_economy::core_integration::CoreClass,
)> {
    let birth = r
        .world()
        .world()
        .module_state::<EconomyStore>()
        .ok_or("the economy")?
        .core_title_birth(core)
        .map_err(|e| format!("{e:?}"))?
        .ok_or("the core's birth")?;
    Ok((birth.source, birth.class))
}

/// `core`'s provenance word, read as M8a's inspection reads it.
fn provenance(r: &Rig, core: ObjId) -> crate::minds::registry_inspection::Provenance {
    let engine = r.world().engine();
    let mut reader = crate::minds::registry_inspection::facts::StoreRead(engine.action_state());
    crate::minds::provenance::provenance_of(&engine.world, &mut reader, core)
}

/// A coreless prospector rig titled to the buyer, standing in `at`.
fn rig_at(r: &mut Rig, at: ObjId) -> Result<ObjId> {
    let buyer = r.buyer;
    let hull =
        r.world_mut()
            .world_mut()
            .create(ObjKind::Thing, "Prospector rig", buyer, Some(at))?;
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@unit/create #{}={}",
            hull.0,
            crate::definitions::content::TV_PROSPECTOR
        ),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, buyer.0),
    )?;
    r.flush();
    Ok(hull)
}

/// **Doc 17h cut M8b-2, a heirloom die** (H1–H4): only GOD issues one (the agent and the
/// buyer are refused); the governed facility refuses it with its uniform refusal; in the back
/// room (a facility holding `core_ungovern`) it integrates as Class III paid to the shop alone,
/// its title birth's source is `heirloom` and its provenance `Heirloom`; the governed refit
/// refuses the core, and the ungoverning refit fits it (displacing a certified Class I fitted
/// at Stickney first: `core_ungovern` never fits a coreless chassis, M8b G2) as the rig's
/// ungoverned Class III, which `read_core` reads and M8a's inspection prints. After a restart the birth's source, the
/// provenance and the fitted unit are unchanged (the integration's receipt audit replays).
#[tokio::test(flavor = "current_thread")]
async fn m8b2_a_heirloom_integrates_in_the_back_room_as_an_ungoverned_three() -> Result {
    use crate::definitions::core_replacement::RIG_III_UNGOVERNED;
    use crate::minds::registry_inspection::facts::{StoreRead, read_unit as read};
    use crate::minds::registry_inspection::{HullClass, Provenance};
    use mush_economy::core_integration::{CoreClass, DieSource};
    let mut r = entitled_rig()?;
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let shop = r
        .deployment
        .ungovern
        .as_ref()
        .ok_or("the back-room shop")?
        .clone();
    let vault = stash(&mut r)?;
    // H3: GOD-only. The stock agent (who may issue certified dies) and the buyer are refused.
    for actor in [agent, buyer] {
        check!(crate::test_checks::refused(r.plan(
            "industry",
            "issue_heirloom_die",
            actor,
            json!({"vault":vault.0})
        ))?);
    }
    r.flush();
    let quotas = r.rows("industry_core_die_quota");
    let lot = issue(&mut r, "issue_heirloom_die", json!({"vault":vault.0}))?;
    r.flush();
    check_eq!(
        r.rows("industry_core_die_quota"),
        quotas,
        "a heirloom counts against no quota"
    );
    let record = r
        .rows("industry_core_die_uncertified")
        .into_iter()
        .find(|row| row["lot"] == json!(lot))
        .ok_or("the heirloom's issue")?;
    check_eq!(record["source"], json!("heirloom"));
    check_eq!(record["label"], json!("die.h"));
    check_eq!(record["class"], json!(DieClass::III));
    // H2: the chartered site refuses it (its uniform refusal: the lot has no certificate).
    let (governed_offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":DieClass::III}),
    )?;
    let before = r.money(r.payer)?;
    let refused = refusal_text(r.plan(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":governed_offer.to_hex(),"payer":r.payer.0.to_string(),
                "input_lot":lot.to_string()}),
    ))?;
    check!(
        refused.contains("accept_core_integration") && refused.contains(MANUFACTURE_REFUSAL),
        "{refused}"
    );
    check_eq!(r.money(r.payer)?, before);
    // H2: the back room integrates it, the integration fee paid to the shop alone.
    let money = (
        r.money(r.payer)?,
        r.money(shop.provider.account)?,
        r.money(r.deployment.integrator.account)?,
        r.money(r.deployment.supplier_account)?,
    );
    let (accept, core) = integrate_uncertified(&mut r, lot, DieClass::III)?;
    let fee = CoreClass::III.integration_minor();
    check_eq!(r.money(r.payer)?.0, money.0.0 - fee);
    check_eq!(r.money(shop.provider.account)?.0, money.1.0 + fee);
    check_eq!(r.money(r.deployment.integrator.account)?, money.2);
    check_eq!(r.money(r.deployment.supplier_account)?, money.3);
    check_eq!(birth_of(&r, core)?, (DieSource::Heirloom, CoreClass::III));
    check_eq!(provenance(&r, core), Provenance::Heirloom);
    // The governed refit never fits it (it would be fitted governed); the ungoverning refit
    // fits it as the rig's ungoverned twin.
    let factory = r.deployment.factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull = rig_at(&mut r, factory)?;
    // The positive control (fix round 1, L3): through the same `refit_class` call, the
    // chartered refit fits a certified core to this rig. The ungoverning refit never fits a
    // coreless chassis (doc 17h cut M8b G2), so the heirloom will displace it.
    let (certified_accept, certified) = integrate_class_one(&mut r)?;
    refit_class(&mut r, hull, certified, certified_accept, DieClass::I, None)
        .map_err(|e| format!("the chartered fitting: {e}"))?;
    let refused = refusal_text(refit_class(&mut r, hull, core, accept, DieClass::III, None))?;
    check!(
        refused.contains("offer_core_refit") && refused.contains(REFIT_REFUSAL),
        "the chartered refit refuses the heirloom at its offer: {refused}"
    );
    r.world_mut().world_mut().move_to(buyer, shop.factory)?;
    r.world_mut().world_mut().move_to(hull, shop.factory)?;
    r.flush();
    refit_class(
        &mut r,
        hull,
        core,
        accept,
        DieClass::III,
        Some("core_ungovern"),
    )
    .map_err(|e| format!("the ungoverning refit: {e}"))?;
    let fitted = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("the fitted rig")?
        .clone();
    check_eq!(fitted.schema().key().id.as_str(), RIG_III_UNGOVERNED);
    check_eq!(
        crate::arena::core::read_core(&fitted).map(|hw| (hw.class, hw.governed)),
        Some((crate::arena::core::CoreClass::III, false))
    );
    {
        let engine = r.world().engine();
        let mut reader = StoreRead(engine.action_state());
        let found = read(&engine.world, &mut reader, hull).ok_or("the fitted rig reads")?;
        check_eq!(found.facts.core, Some(core));
        check_eq!(found.facts.class, HullClass::III);
        check_eq!(found.facts.governed, Some(false));
        check_eq!(found.facts.provenance, Provenance::Heirloom);
        let receipt =
            crate::minds::registry_inspection::facts::receipt(&engine.world, &found.facts, false);
        check!(
            receipt.contains("provenance heirloom"),
            "the inspection prints it: {receipt}"
        );
    }
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(birth_of(&r, core)?, (DieSource::Heirloom, CoreClass::III));
    check_eq!(provenance(&r, core), Provenance::Heirloom);
    check_eq!(
        mush_tactics::module::unit_of(r.world().world(), hull),
        Some(&fitted)
    );
    Ok(())
}

/// **Doc 17h cut M8b-2, grey dies** (H1–H4): a grey die of each class, issued by GOD with no
/// origin recorded, integrates in the back room as a core of that class whose birth's source is
/// `grey` and provenance `Grey`; the Class I one is fitted only ungoverned. The chartered site
/// refuses a grey lot, the back room refuses a certified one, and after a restart every
/// birth's source is unchanged.
#[tokio::test(flavor = "current_thread")]
async fn m8b2_grey_dies_integrate_as_ungoverned_cores_of_their_class() -> Result {
    use crate::definitions::core_replacement::RIG_I_UNGOVERNED;
    use crate::minds::registry_inspection::Provenance;
    use mush_economy::core_integration::{CoreClass, DieSource};
    let mut r = entitled_rig()?;
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let shop = r
        .deployment
        .ungovern
        .as_ref()
        .ok_or("the back-room shop")?
        .clone();
    let vault = stash(&mut r)?;
    check!(crate::test_checks::refused(r.plan(
        "industry",
        "issue_grey_die",
        agent,
        json!({"class":"i","vault":vault.0})
    ))?);
    let mut cores = Vec::new();
    for (die, class) in [
        (DieClass::I, CoreClass::I),
        (DieClass::II, CoreClass::II),
        (DieClass::III, CoreClass::III),
    ] {
        let lot = issue(
            &mut r,
            "issue_grey_die",
            json!({"class":die,"vault":vault.0}),
        )?;
        let shop_before = r.money(shop.provider.account)?.0;
        let (accept, core) = integrate_uncertified(&mut r, lot, die)?;
        check_eq!(
            r.money(shop.provider.account)?.0,
            shop_before + class.integration_minor()
        );
        check_eq!(birth_of(&r, core)?, (DieSource::Grey, class));
        check_eq!(provenance(&r, core), Provenance::Grey);
        cores.push((accept, core, class));
    }
    // The chartered site refuses a grey lot; the back room refuses a certified one.
    let grey = issue(
        &mut r,
        "issue_grey_die",
        json!({"class":"i","vault":vault.0}),
    )?;
    let (governed_offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":"i"}),
    )?;
    let refused = refusal_text(r.plan(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":governed_offer.to_hex(),"payer":r.payer.0.to_string(),
                "input_lot":grey.to_string()}),
    ))?;
    check!(
        refused.contains("accept_core_integration") && refused.contains(MANUFACTURE_REFUSAL),
        "{refused}"
    );
    let (_, die) = r.perform("industry", "issue_core_die", agent, json!({"class":"i"}))?;
    let certified = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
    let (back_room_offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":"i","site":"core_ungovern"}),
    )?;
    let refused = refusal_text(r.plan(
        "industry",
        "accept_core_integration",
        buyer,
        json!({"offer":back_room_offer.to_hex(),"payer":r.payer.0.to_string(),
                "input_lot":certified.to_string()}),
    ))?;
    check!(
        refused.contains("accept_core_integration") && refused.contains(MANUFACTURE_REFUSAL),
        "{refused}"
    );
    // The Class I grey core is fitted only ungoverned.
    let (accept, core, _) = cores[0];
    let factory = r.deployment.factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull = rig_at(&mut r, factory)?;
    // As for the heirloom: the positive control first, a certified core fitted at Stickney
    // through the same call (M8b G2), then the grey core refused at the refit's offer.
    let (certified_accept, certified) = integrate_class_one(&mut r)?;
    refit_class_one(&mut r, hull, certified, certified_accept, None)
        .map_err(|e| format!("the chartered fitting: {e}"))?;
    let refused = refusal_text(refit_class_one(&mut r, hull, core, accept, None))?;
    check!(
        refused.contains("offer_core_refit") && refused.contains(REFIT_REFUSAL),
        "the chartered refit refuses the grey core at its offer: {refused}"
    );
    r.world_mut().world_mut().move_to(buyer, shop.factory)?;
    r.world_mut().world_mut().move_to(hull, shop.factory)?;
    r.flush();
    refit_class_one(&mut r, hull, core, accept, Some("core_ungovern"))
        .map_err(|e| format!("the ungoverning refit: {e}"))?;
    let fitted = mush_tactics::module::unit_of(r.world().world(), hull).ok_or("the fitted rig")?;
    check_eq!(fitted.schema().key().id.as_str(), RIG_I_UNGOVERNED);
    check_eq!(
        crate::arena::core::read_core(fitted).map(|hw| (hw.class, hw.governed)),
        Some((crate::arena::core::CoreClass::I, false))
    );
    r.reboot(CoreOriginMode::Active)?;
    for (_, core, class) in cores {
        check_eq!(birth_of(&r, core)?, (DieSource::Grey, class));
        check_eq!(provenance(&r, core), Provenance::Grey);
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn shipped_class_three_requires_actual_entitlement_and_never_charges_on_refusal() -> Result {
    let mut r = Rig::new()?;
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let native = filing(&r.deployment)?;
    let hardware = GovernedHardware::prepare(
        &layer(),
        CoreOriginMode::Active,
        r.deployment.clone(),
        &native,
    )?;
    check_eq!(hardware.origin_policy().monthly_quotas, [64, 16, 4]);
    check_eq!(
        hardware.payment_policy().delivery,
        Some(mush_economy::core_integration::CoreDeliveryPromise::PersonalCustodyV1)
    );
    for (class, id) in [
        (
            crate::arena::core::CoreClass::I,
            crate::definitions::core_replacement::LINE_I,
        ),
        (
            crate::arena::core::CoreClass::II,
            crate::definitions::core_replacement::LINE_II,
        ),
        (
            crate::arena::core::CoreClass::III,
            crate::definitions::core_replacement::LINE_III,
        ),
    ] {
        check_eq!(
            hardware
                .family()
                .definition(class)
                .ok_or("finite unit")?
                .key()
                .id
                .as_str(),
            id
        );
    }
    let before = r.money(r.payer)?;
    let (_, die) = r.perform(
        "industry",
        "issue_core_die",
        agent,
        json!({"class":mush_industry::core_production::DieClass::III}),
    )?;
    let lot = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
    let (offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":mush_industry::core_production::DieClass::III,"supplier_lot":lot.to_string()}),
    )?;
    let disk = r.boot.clone();
    check!(
        r.plan(
            "industry",
            "accept_core_integration",
            buyer,
            json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()})
        )
        .is_err()
    );
    check_eq!(r.money(r.payer)?, before);
    check!(
        same_boot(&r.boot, &disk),
        "refused hardware acceptance changed retained images"
    );
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(r.money(r.payer)?, before);
    Ok(())
}

// ModuleBootData is not PartialEq. Compare every retained field without printing
// source/owner payloads on assertion failure.
fn same_boot(a: &Boot, b: &Boot) -> bool {
    a.len() == b.len()
        && a.iter().all(|(key, a)| {
            b.get(key).is_some_and(|b| {
                a.kv == b.kv
                    && a.ledger == b.ledger
                    && a.streams == b.streams
                    && a.fetch_ms == b.fetch_ms
                    && a.omitted == b.omitted
            })
        })
}

#[tokio::test(flavor = "current_thread")]
async fn shipped_signed_acceptance_rechecks_facility_before_stage_and_consuming_take() -> Result {
    for at_take in [false, true] {
        let mut r = Rig::new()?;
        let buyer = r.buyer;
        let agent = r.deployment.agents[1].actor;
        let (_, die) = r.perform("industry", "issue_core_die", agent, json!({"class":"i"}))?;
        let lot = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
        let (offer, _) = r.perform(
            "industry",
            "offer_core_integration",
            agent,
            json!({"requester":buyer.0,"class":"i","supplier_lot":lot.to_string()}),
        )?;
        let before = r.money(r.payer)?;
        let disk = r.boot.clone();
        let plan = r.plan(
            "industry",
            "accept_core_integration",
            buyer,
            json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
        )?;
        let mut plan = Some(plan);
        if at_take {
            r.stage(plan.take().ok_or("prepared hardware action")?)?;
        }
        let factory = r.deployment.factory;
        r.world_mut()
            .world_mut()
            .get_mut(factory)
            .ok_or("actual facility")?
            .flags
            .insert(mush_core::Flags::GOING);
        r.world_mut().world_mut().take_dirty();
        if at_take {
            check!(r.take_install().is_err());
        } else {
            check!(
                r.stage(plan.take().ok_or("prepared hardware action")?)
                    .is_err()
            );
        }
        check_eq!(r.money(r.payer)?, before);
        check!(
            same_boot(&r.boot, &disk),
            "stale hardware acceptance changed retained images"
        );
    }
    Ok(())
}

/// Doc 17h cut M8d2: the shipped seizure terms (`[tacsim.minds]`' defaults).
const SEIZURE_TERMS: crate::minds::registry_inspection::seizure::SeizureTerms =
    crate::minds::registry_inspection::seizure::SeizureTerms {
        max_age_ic_days: 3,
        hold_ic_days: 3,
    };

/// **Name the Registry** (doc 17h cut M8d2): a `Registry` org with a `review` scope, one
/// operator granted `roll.registry.inspect`, `.seize` and `.forfeit` there by GOD (the
/// runbook's grants), and a reboot that composes the inspection and the seizure over them.
fn name_registry(
    r: &mut Rig,
) -> Result<(
    ObjId,
    mush_roll::commerce::registry_inspection::RegistryInspectionPolicy,
)> {
    let operator = r.world_mut().spawn_player("Registry operator");
    for line in ["@org/create Registry", "@org/scope create Registry=review"] {
        command(r.world_mut(), GOD, line)?;
    }
    let (org, scope) = {
        let orgs = r
            .world()
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org owner")?
            .state();
        let org = orgs
            .orgs()
            .find(|o| o.name.as_str() == "Registry")
            .ok_or("the Registry")?
            .id;
        let scope = orgs
            .scopes_in(org)
            .find(|s| s.name.as_str() == "review")
            .ok_or("the review scope")?
            .id;
        (org, scope)
    };
    for capability in [
        "roll.registry.inspect",
        "roll.registry.seize",
        "roll.registry.forfeit",
    ] {
        command(
            r.world_mut(),
            GOD,
            &format!(
                "@org/grant #{}=Registry {capability} scope=#{}",
                operator.0, scope.0
            ),
        )?;
    }
    let policy = mush_roll::commerce::registry_inspection::RegistryInspectionPolicy::new(
        r.deployment.issuer.clone(),
        org,
        scope,
        vec![operator],
    )?;
    r.registry = Some(policy.clone());
    r.reboot(CoreOriginMode::Active)?;
    Ok((operator, policy))
}

/// A rig fitted at Stickney with a certified Class I core, then ungoverned in the back room
/// with a second (M8b's flow): the hull and its ungoverned core.
fn ungoverned_rig(r: &mut Rig) -> Result<(ObjId, ObjId)> {
    let (first_accept, first) = integrate_class_one(r)?;
    let (second_accept, second) = integrate_class_one(r)?;
    let buyer = r.buyer;
    let factory = r.deployment.factory;
    let shop = r
        .deployment
        .ungovern
        .as_ref()
        .ok_or("the back-room shop")?
        .factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull = rig_at(r, factory)?;
    refit_class_one(r, hull, first, first_accept, None)?;
    r.world_mut().world_mut().move_to(buyer, shop)?;
    r.world_mut().world_mut().move_to(hull, shop)?;
    r.flush();
    refit_class_one(r, hull, second, second_accept, Some("core_ungovern"))?;
    Ok((hull, second))
}

/// **Land an inspection of `hull` as `operator`** (doc 17h cut M8d2; the deviation M8d's
/// `seed_ungoverned` set the precedent for): this rig's hulls belong to no port or yard, so the
/// inspection action's known-hull gate (`facts::known`) refuses them. The record is the real
/// read of the real hull (`facts::read_unit`: the fitted core, its governor, its provenance, the
/// holder), at the world's IC now and the ring's next `seq`, decoded through the record's own
/// strict wire (its census runs) and written in one action-state transaction.
fn seed_inspection(
    r: &mut Rig,
    hull: ObjId,
    operator: ObjId,
    policy: &mush_roll::commerce::registry_inspection::RegistryInspectionPolicy,
) -> Result<u64> {
    use crate::minds::registry_inspection::facts::{StoreRead, read_unit};
    use mush_roll::commerce::registry_inspection::{InspectionRecord, inspection_slots};
    let host = crate::minds::registry_inspection::InspectionHost::of(r.world().world())
        .ok_or("the inspection is composed")?;
    let family = host.writer().family()?;
    let (facts, seq, at_ic) = {
        let engine = r.world().engine();
        let mut reader = StoreRead(engine.action_state());
        let found = read_unit(&engine.world, &mut reader, hull).ok_or("the hull reads")?;
        let newest = inspection_slots(hull)
            .filter_map(|slot| engine.action_state().get(&family, &slot).ok().flatten())
            .map(|record| record.seq)
            .max()
            .unwrap_or(0);
        (found.facts, newest + 1, engine.world.world_ic_now())
    };
    r.serial += 1;
    let mut token = [0x5e; 16];
    token[..8].copy_from_slice(&u64::try_from(r.serial)?.to_be_bytes());
    let record: InspectionRecord = serde_json::from_value(json!({
        "format": 1,
        "inspection": CommitToken::from_bytes(token),
        "action": policy.action()?,
        "authority": policy,
        "operator": operator,
        "authorization": {"at": at_ic.at, "organization_revision": 1, "scope_revision": 1,
            "sources": [{"tag": "grant", "revision": 1}]},
        "hull": hull,
        "seq": seq,
        "core": facts.core,
        "class": facts.class,
        "governed": facts.governed,
        "provenance": facts.provenance,
        "seal_check": facts.seal_check,
        "at_ic": at_ic,
        "ungoverned_note": facts.ungoverned_note,
        "holder": facts.holder,
    }))?;
    let slot = mush_roll::commerce::registry_inspection::inspection_key(hull, seq);
    {
        let engine = r.world_mut().engine_mut();
        let mut txn = engine.action_transaction(&family)?;
        if txn.get(&family, &slot)?.is_some() {
            txn.replace(&family, &slot, record)?;
        } else {
            txn.insert(&family, &slot, record)?;
        }
        let (_, installation) = txn.prepare()?.into_parts();
        engine.install_prepared_action(installation)?;
    }
    r.flush();
    Ok(seq)
}

/// The core's title, read from the economy.
fn title(r: &Rig, core: ObjId) -> Result<mush_economy::EconomicAsset> {
    Ok(r.world()
        .world()
        .module_state::<EconomyStore>()
        .ok_or("the economy")?
        .snapshot()
        .asset(core)
        .ok_or("the core's title")?
        .clone())
}

/// What `actor` is told for `line`, every line joined (the world flushed after).
fn said(r: &mut Rig, actor: ObjId, line: &str) -> String {
    let text = r
        .world_mut()
        .run(actor, line)
        .iter()
        .filter_map(|(_, e)| match e {
            mush_proto::OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    r.flush();
    text
}

/// The seizure records, decoded.
fn seizures(r: &Rig) -> Result<Vec<mush_roll::commerce::registry_inspection::SeizureRecord>> {
    Ok(
        r.rows(mush_roll::commerce::registry_inspection::SEIZURE_FAMILY)
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?,
    )
}

/// **Doc 17h cut M8d2, the seizure half of M8d's gate** (S1–S7; the lead's "impounded in
/// place"). In the hardware flow rig, with the Registry named:
/// - before any inspection the hull cannot be seized; a stale ungoverned inspection (one IC
///   second past `seizure_inspection_max_age_ic_days`) cannot be either, by the lead's sentence;
/// - a fresh inspection of the ungoverned rig is seized: the core title `Disputed { the Registry
///   org, hull/<8 hex>/inspection/2 }` and still the holder's, one seizure record, `minds.seized`
///   resolving at the seizure's IC second, and the hull's ROM seal sealed as no core (Class 0,
///   governed, no floor lifted) while `read_core` still reads the ungoverned core fitted;
/// - a second seizure is refused; the seized core cannot be refitted (the governed shop refuses
///   the hull's held core) nor transferred, by its holder or by staff;
/// - forfeit before the hold refuses ("That seizure is still within its hold.");
/// - a restart replays it all (the economy's receipt audit and the managed-title index, the
///   record, the seal);
/// - after the hold the forfeit moves the title to the Registry org with the claim released,
///   the hull still sealing the ROM; a second forfeit refuses; a restart replays that too.
#[tokio::test(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn m8d2_an_ungoverned_rig_is_seized_impounded_and_forfeited_after_its_hold() -> Result {
    use crate::minds::registry_inspection::seizure::{
        FORFEIT_ACTION, Findings, SEIZE_ACTION, fitted_core,
    };
    use mush_roll::commerce::facts::RegistryFindings as _;
    let mut r = Rig::new()?;
    let (hull, core) = ungoverned_rig(&mut r)?;
    let (operator, policy) = name_registry(&mut r)?;
    let registry = PrincipalId::Organization(policy.organization());
    let n = hull.0;
    check_eq!(fitted_core(r.world().world(), hull), Some(core));

    // No inspection: nothing to seize.
    check!(
        r.plan("tacsim", SEIZE_ACTION, operator, json!({"hull": n}))
            .is_err()
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/seize #{n}")),
        "No such inspection."
    );
    // A stale inspection: one IC second past the age.
    check_eq!(seed_inspection(&mut r, hull, operator, &policy)?, 1);
    let now = r.world().world().now();
    r.world_mut().world_mut().set_now(now + 3 * 86_400 + 1);
    r.flush();
    check!(
        r.plan("tacsim", SEIZE_ACTION, operator, json!({"hull": n}))
            .is_err(),
        "a stale inspection grounds nothing"
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/seize #{n}")),
        "The latest inspection of that hull found nothing to seize."
    );

    // A fresh inspection (seq 2) grounds the seizure.
    check_eq!(seed_inspection(&mut r, hull, operator, &policy)?, 2);
    let before = title(&r, core)?;
    let (_, outcome) = r.perform("tacsim", SEIZE_ACTION, operator, json!({"hull": n}))?;
    check_eq!(
        outcome["summary"].as_str(),
        Some(
            format!(
                "Seized the core in hull #{n}. It stays fitted and flies nothing; it may be \
                 forfeited after 3 IC days."
            )
            .as_str()
        )
    );
    let held = title(&r, core)?;
    let key = mush_economy::registry_filing::seizure::inspection_key(hull, 2);
    check_eq!(
        held.encumbrances,
        vec![mush_economy::Encumbrance::Disputed {
            claimant: registry.clone(),
            reason: key.clone(),
        }]
    );
    check_eq!(
        held.title_owner,
        before.title_owner,
        "seizure keeps the title"
    );
    let records = seizures(&r)?;
    check_eq!(records.len(), 1);
    let record = records[0].clone();
    check_eq!(
        (record.hull, record.seq, record.core, record.seized_by),
        (hull, 2, core, operator)
    );
    check_eq!(record.forfeit_after_ic, record.at_ic + 3 * 86_400);
    // `minds.seized`, the public finding, at the seizure's IC second.
    check_eq!(
        Findings.seized(r.world().world(), hull, 2),
        Some(record.at_ic)
    );
    check_eq!(Findings.seized(r.world().world(), hull, 1), None);
    // Impounded in place: the ROM seal as no core, the core still fitted and ungoverned.
    let unit = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("the rig")?
        .clone();
    check!(crate::match_::seal::core_impounded(
        r.world().world(),
        &unit
    ));
    let rom = rom_seal(&r, hull)?;
    check_eq!(
        (rom.core.clone(), rom.class, rom.governed),
        (None, crate::arena::core::CoreClass::Zero, true)
    );
    check!(!rom.lifts_floor());
    check!(crate::arena::core::read_core(&unit).is_some_and(|hw| !hw.governed));

    // Twice: refused.
    check!(
        r.plan("tacsim", SEIZE_ACTION, operator, json!({"hull": n}))
            .is_err()
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/seize #{n}")),
        "That inspection's core is already seized."
    );
    // S6: the seized core cannot be sold or transferred, by its holder or by staff.
    let other = r.world_mut().spawn_player("A buyer of cores");
    for actor in [r.buyer, GOD] {
        check!(
            command(
                r.world_mut(),
                actor,
                &format!("@asset/transfer #{}=#{}", core.0, other.0)
            )
            .is_err(),
            "a seized core cannot be transferred"
        );
    }
    r.flush();
    check_eq!(title(&r, core)?, held);
    // S6: nor refitted: the chartered shop refuses the hull whose fitted core is held.
    let (third_accept, third) = integrate_class_one(&mut r)?;
    let buyer = r.buyer;
    let factory = r.deployment.factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    r.world_mut().world_mut().move_to(hull, factory)?;
    r.flush();
    check!(
        refit_class_one(&mut r, hull, third, third_accept, None).is_err(),
        "a seized core cannot be refitted out"
    );
    check_eq!(fitted_core(r.world().world(), hull), Some(core));

    // Forfeit before the hold: refused, by the lead's sentence.
    check!(
        r.plan(
            "tacsim",
            FORFEIT_ACTION,
            operator,
            json!({"hull": n, "seq": 2})
        )
        .is_err()
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/forfeit #{n}")),
        "That seizure is still within its hold."
    );

    // A restart replays the seizure.
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(title(&r, core)?, held);
    check_eq!(seizures(&r)?, vec![record.clone()]);
    check_eq!(
        Findings.seized(r.world().world(), hull, 2),
        Some(record.at_ic)
    );
    check_eq!(
        rom_seal(&r, hull)?.class,
        crate::arena::core::CoreClass::Zero
    );

    // After the hold: the title to the Registry org, the claim released.
    let now = r.world().world().now();
    r.world_mut().world_mut().set_now(now + 3 * 86_400);
    r.flush();
    let (_, outcome) = r.perform(
        "tacsim",
        FORFEIT_ACTION,
        operator,
        json!({"hull": n, "seq": 2}),
    )?;
    check_eq!(
        outcome["summary"].as_str(),
        Some(format!("The core seized from hull #{n} is forfeit to the Registry.").as_str())
    );
    let forfeit = title(&r, core)?;
    check_eq!(forfeit.title_owner, registry);
    check!(forfeit.encumbrances.is_empty());
    let records = seizures(&r)?;
    check!(records.len() == 1 && records[0].forfeited());
    check_eq!(records[0].forfeited_by, Some(operator));
    // Still impounded: the Registry's core flies nothing.
    check_eq!(
        rom_seal(&r, hull)?.class,
        crate::arena::core::CoreClass::Zero
    );
    // Twice: the uniform refusal.
    check!(
        r.plan(
            "tacsim",
            FORFEIT_ACTION,
            operator,
            json!({"hull": n, "seq": 2})
        )
        .is_err()
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/forfeit #{n}/2")),
        "That did not go through. Nothing changed."
    );
    // A restart replays the forfeit.
    r.reboot(CoreOriginMode::Active)?;
    check_eq!(title(&r, core)?, forfeit);
    check_eq!(
        rom_seal(&r, hull)?.class,
        crate::arena::core::CoreClass::Zero
    );
    Ok(())
}

/// **Doc 17h cut M8d2: a governed rig cannot be seized** (S4, the record's word): its newest
/// inspection read a governed certified core, so the provider refuses and the command says the
/// lead's sentence; its title carries no claim and it seals governed Class I as before.
#[tokio::test(flavor = "current_thread")]
async fn m8d2_a_governed_rig_cannot_be_seized() -> Result {
    use crate::minds::registry_inspection::seizure::SEIZE_ACTION;
    let mut r = Rig::new()?;
    let (accept, core) = integrate_class_one(&mut r)?;
    let buyer = r.buyer;
    let factory = r.deployment.factory;
    r.world_mut().world_mut().move_to(buyer, factory)?;
    let hull = rig_at(&mut r, factory)?;
    refit_class_one(&mut r, hull, core, accept, None)?;
    let (operator, policy) = name_registry(&mut r)?;
    seed_inspection(&mut r, hull, operator, &policy)?;
    check!(
        r.plan("tacsim", SEIZE_ACTION, operator, json!({"hull": hull.0}))
            .is_err()
    );
    check_eq!(
        said(&mut r, operator, &format!("@registry/seize #{}", hull.0)),
        "The latest inspection of that hull found nothing to seize."
    );
    check!(title(&r, core)?.encumbrances.is_empty());
    check!(seizures(&r)?.is_empty());
    let seal = rom_seal(&r, hull)?;
    check_eq!(
        (seal.class, seal.governed),
        (crate::arena::core::CoreClass::I, true)
    );
    // A non-operator is told the driver's one sentence, before any hull is read.
    check_eq!(
        said(&mut r, buyer, &format!("@registry/seize #{}", hull.0)),
        "That did not go through. Nothing changed."
    );
    Ok(())
}
