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
    owners.economy = commerce.configure_economy(owners.economy);
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
    if out.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {
        return Err("actual hardware setup command refused".into());
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
        if let Some(w) = &self.w {
            if let Some(r) = w
                .world()
                .module_state::<crate::arena_bridge::ArenaRegistry>()
            {
                r.tasks().signal_stop_all();
            }
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
        self.w = Some(
            self.w
                .take()
                .ok_or("world owned")?
                .reboot_into(modules, self.boot.clone())?,
        );
        self.flush();
        Ok(())
    }
    fn new() -> Result<Self> {
        let handles: InventoryHandleCell = Arc::default();
        let mut w = TestWorld::with_modules(vec![
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
        ]);
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
            "@instrument/register cr=2",
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
                &format!("@account/open org:Basin/{}=cr", scope.0),
            )?;
        }
        command(&mut w, GOD, &format!("@account/open #{}=cr", buyer.0))?;
        command(&mut w, GOD, "@account/system hardware.fixture=cr")?;
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
        let deployment = HardwareDeployment {
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
        let mut r = Self {
            w: Some(w),
            boot,
            deployment,
            buyer,
            payer,
            serial: 100,
        };
        r.reboot(CoreOriginMode::Initialize)?;
        r.initialize_roll()?;
        for action in [
            "initialize_core_origin",
            "initialize_core_production",
            "initialize_core_refit",
        ] {
            r.perform("industry", action, GOD, json!({}))?;
        }
        assert!(
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
    assert_eq!(r.money(r.payer)?, (money[0].0, money[0].1 + 300_000));
    assert!(
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
    assert_eq!(r.world().world().world_ic_now().at - accepted.at, 24 * 3600);
    let (_, completed) = r.perform(
        "industry",
        "complete_core_integration",
        buyer,
        params.clone(),
    )?;
    let output = serde_json::from_value::<ObjId>(completed["object"].clone())?;
    assert_eq!(
        r.money(r.deployment.supplier_account)?.0,
        money[1].0 + 270_000
    );
    assert_eq!(
        r.money(r.deployment.integrator.account)?.0,
        money[2].0 + 30_000
    );
    r.perform("industry", "deliver_core_output", buyer, params)?;
    assert_eq!(
        r.world()
            .world()
            .get(output)
            .ok_or("delivered core")?
            .location,
        Some(buyer)
    );
    let native = mush_economy::core_integration::require_core_delivery(r.world().world(), accept)?;
    assert_eq!(native.subject().requester, buyer);
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
    let heat = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: mush_tactics::CellAddr::Unit {
                cell: mush_tactics::CellId::new("heat")?,
            },
            value: 7,
        }],
        None,
        Some("retained actual hull heat".into()),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@unit/damage #{}={}", hull.0, serde_json::to_string(&heat)?),
    )?;
    r.flush();
    let before = mush_tactics::module::unit_of(r.world().world(), hull)
        .ok_or("native line unit")?
        .clone();
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
    assert!(
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
    assert_eq!(
        after.schema().key().id.as_str(),
        crate::definitions::core_replacement::LINE_I
    );
    assert_eq!(after.object(), before.object());
    for (address, cell) in before.cells() {
        if !matches!(address, mush_tactics::CellAddr::Component { component, .. } if component.as_str() == "core")
        {
            assert_eq!(after.cell(address), Some(cell));
        }
    }
    for (component, state) in before.components() {
        if component.as_str() != "core" {
            assert_eq!(
                after
                    .components()
                    .find(|(id, _)| *id == component)
                    .map(|(_, value)| value),
                Some(state)
            );
        }
    }
    assert_eq!(r.money(r.payer)?.0, money[0].0 - 310_000);
    assert_eq!(
        r.money(r.deployment.integrator.account)?.0,
        money[2].0 + 40_000
    );
    assert_eq!(r.money(r.payer)?.1, money[0].1);
    assert!(!r.rows("core_component_exchanges").is_empty());
    r.reboot(CoreOriginMode::Active)?;
    assert_eq!(
        mush_tactics::module::unit_of(r.world().world(), hull),
        Some(&after)
    );
    assert!(
        mush_economy::core_integration::require_core_delivery(r.world().world(), accept).is_ok()
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
    assert_eq!(hardware.origin_policy().monthly_quotas, [64, 16, 4]);
    assert_eq!(
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
        assert_eq!(
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
    let (_, die) = r.perform("industry", "issue_core_die", agent, json!({"class":"iii"}))?;
    let lot = serde_json::from_value::<mush_inventory::LotId>(die["lot"].clone())?;
    let (offer, _) = r.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester":buyer.0,"class":"iii","supplier_lot":lot.to_string()}),
    )?;
    let disk = r.boot.clone();
    assert!(
        r.plan(
            "industry",
            "accept_core_integration",
            buyer,
            json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()})
        )
        .is_err()
    );
    assert_eq!(r.money(r.payer)?, before);
    assert!(
        same_boot(&r.boot, &disk),
        "refused hardware acceptance changed retained images"
    );
    r.reboot(CoreOriginMode::Active)?;
    assert_eq!(r.money(r.payer)?, before);
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
            assert!(r.take_install().is_err());
        } else {
            assert!(
                r.stage(plan.take().ok_or("prepared hardware action")?)
                    .is_err()
            );
        }
        assert_eq!(r.money(r.payer)?, before);
        assert!(
            same_boot(&r.boot, &disk),
            "stale hardware acceptance changed retained images"
        );
    }
    Ok(())
}
