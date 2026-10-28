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
    core_production::{CoreOriginMode, OriginAgent},
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
        let modules = modules(&self.deployment, mode)?;
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
        })
    }
    fn new() -> Result<Self> {
        let mut r = Self::provision(Vec::new())?;
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
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    let mut params = json!({"requester":buyer.0,"hull":hull.0,"incoming":incoming.0,
        "manufacture":accept.to_hex(),"class":"i"});
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
    r.progress("advance_core_refit", 1)?;
    r.perform(
        "industry",
        "install_core_component",
        buyer,
        json!({"order":id}),
    )?;
    Ok(())
}

/// The ROM seal an IC epoch would take over `hull` now, under the shipped minds defaults.
fn rom_seal(r: &Rig, hull: ObjId) -> Result<crate::arena::mind::MindSeal> {
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
        Ok((
            crate::surface::act::rig_rates(w, hull, None).ok_or("the rig's drive rates")?,
            logistics.clone(),
        ))
    };
    let stock = surface(&r)?;
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
