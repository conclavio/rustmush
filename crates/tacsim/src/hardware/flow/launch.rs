//! Real launch configuration and ordinary operator setup. This suite exercises
//! initialization, cold activation and the actual private source import. It does
//! not turn a source hash into a certificate, filing, sale or Conduct result.
use super::*;
#[path = "launch/story.rs"]
mod story;
use crate::launch::{
    HardwareCoordinates, HouseDeployment, LaunchComposition, LaunchDeployment, LaunchMode,
    LaunchRecipe, LaunchVenue, NamedHouses,
};
use mush_core::{
    AccountIdentity,
    account::{AccountIdentityEpoch, PersonalAccountSnapshot},
};
use mush_economy::managed_entitlement::Recipient;
use mush_org::{OrgId, OrgScopeId};
use std::collections::BTreeMap;

struct Logins {
    values: BTreeMap<ObjId, mush_core::AccountId>,
    epoch: AccountIdentityEpoch,
}
impl AccountIdentity for Logins {
    fn account_of(&self, actor: ObjId) -> Option<mush_core::AccountId> {
        self.values.get(&actor).copied()
    }
    fn capture_personal_account(&self, actor: ObjId) -> Option<PersonalAccountSnapshot> {
        self.epoch.personal_account(actor, self.account_of(actor)?)
    }
    fn observe_distinct_accounts(
        &self,
        a: ObjId,
        b: ObjId,
    ) -> Option<mush_core::account::AccountPairSnapshot> {
        self.epoch
            .distinct_pair(a, self.account_of(a)?, b, self.account_of(b)?)
    }
}
fn logins(r: &mut Rig) -> Result {
    let accounts: Arc<dyn AccountIdentity> = Arc::new(Logins {
        values: BTreeMap::from([
            (GOD, mush_core::AccountId::mint("launch-publisher", 1)?),
            (r.buyer, mush_core::AccountId::mint("launch-buyer", 2)?),
        ]),
        epoch: AccountIdentityEpoch::default(),
    });
    r.world_mut()
        .engine_mut()
        .set_account_identity(accounts.clone());
    *r.world_mut()
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .ok_or("actual Sim account slot")? = mush_sim::AccountOracle::new(accounts);
    Ok(())
}
fn setup() -> Result<(Rig, LaunchDeployment)> {
    let mut r = Rig::provision(vec![Box::new(mush_faction::FactionModule::new())])?;
    let room =
        r.world_mut()
            .world_mut()
            .create(ObjKind::Room, "Named launch counter", GOD, None)?;
    for line in [
        "@faction/create Republic",
        "@account/system launch.registry=tacsim.credit",
        "@account/system launch.venue=tacsim.credit",
    ] {
        command(r.world_mut(), GOD, line)?;
    }
    let mut houses = Vec::new();
    for (org_name, source_name) in [
        ("Meridiani", "Meridiani Field Series"),
        ("Coprates", "Coprates Drill"),
        ("Tycho", "Safe Conduct"),
    ] {
        command(r.world_mut(), GOD, &format!("@org/create {org_name}"))?;
        command(
            r.world_mut(),
            GOD,
            &format!("@org/scope create {org_name}=publication"),
        )?;
        let state = r
            .world()
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state();
        let organization = state
            .orgs()
            .find(|o| o.name.as_str() == org_name)
            .ok_or("actual named house")?
            .id;
        let scope = state
            .scopes_in(organization)
            .find(|s| s.name.as_str() == "publication")
            .ok_or("actual scope")?
            .id;
        command(
            r.world_mut(),
            GOD,
            &format!("@account/open org:{org_name}/{}=tacsim.credit", scope.0),
        )?;
        for cap in [
            mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
            "asset.title.transfer",
            "economy.account.post",
        ] {
            command(
                r.world_mut(),
                GOD,
                &format!("@org/grant #{}={org_name} {cap} scope=#{}", GOD.0, scope.0),
            )?;
        }
        let economy = r
            .world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy")?;
        let snapshot = economy.snapshot();
        let treasury = snapshot
            .accounts()
            .find(|a| a.owner == PrincipalId::Organization(organization) && a.scope == Some(scope))
            .ok_or("treasury")?
            .id;
        let faucet = snapshot
            .accounts()
            .find(|a| matches!(&a.owner,PrincipalId::System(s)if s.as_str()=="hardware.fixture"))
            .ok_or("actual faucet")?
            .id;
        command(
            r.world_mut(),
            GOD,
            &format!("@econ/mint {}={} 1000", faucet.0, treasury.0),
        )?;
        houses.push(HouseDeployment {
            organization,
            scope,
            treasury,
            publisher: GOD,
            source_name: source_name.into(),
        });
    }
    let [meridiani, coprates, tycho]: [HouseDeployment; 3] =
        houses.try_into().map_err(|_| "three named houses")?;
    let recipient = |name: &str| -> Result<Recipient> {
        let owner = PrincipalId::System(SystemPrincipalId::new(name).ok_or("system")?);
        let account = r
            .world()
            .world()
            .module_state::<EconomyStore>()
            .ok_or("Economy")?
            .snapshot()
            .accounts()
            .find(|a| a.owner == owner)
            .ok_or("actual account")?
            .id;
        Ok(Recipient { owner, account })
    };
    let republic = r
        .world()
        .world()
        .module_state::<mush_faction::FactionState>()
        .ok_or("Faction")?
        .faction_by_name("Republic")
        .ok_or("actual Republic")?
        .id;
    let h = &r.deployment;
    let deployment = LaunchDeployment {
        format: 1,
        mode: LaunchMode::Initialize,
        registry_issuer: h.issuer.clone(),
        registry: recipient("launch.registry")?,
        venue: LaunchVenue {
            name: mush_market::MarketId::new("launch.counter").ok_or("market")?,
            room,
            recipient: recipient("launch.venue")?,
        },
        houses: NamedHouses {
            meridiani,
            coprates,
            tycho,
        },
        republic,
        hardware: HardwareCoordinates {
            importer: h.importer,
            stock_scope: h.stock_scope,
            depot: h.depot,
            agents: h.agents.clone(),
            grades: h.grades,
            supplier_account: h.supplier_account,
            integrator: h.integrator.clone(),
            factory: h.factory,
            output_container: h.output_container,
        },
    };
    r.flush();
    Ok((r, deployment))
}
fn modules(d: &LaunchDeployment) -> Result<Vec<Box<dyn GameModule>>> {
    // This explicit host budget admits three full, independently charged named
    // Registry trials in one real UTC day. No quota balance or receipt is seeded.
    let mut cfg = layer().config().clone();
    cfg.minds.proving_daily_unit_minutes = 100_000;
    let layer = TacsimRuleset::with_config(cfg).with_conduct_carrier()?;
    let launch = LaunchComposition::prepare(&layer, d.clone())?;
    let (mut modules, handles) = crate::composition::server_modules_with_launch(
        Default::default(),
        Default::default(),
        Default::default(),
        &Default::default(),
        &crate::economy::GameEconomyConfig {
            housing: false,
            market: false,
            ..Default::default()
        },
        Default::default(),
        &launch,
    );
    modules.push(Box::new(
        launch.configure_tacsim(layer.with_inventory(handles))?,
    ));
    Ok(modules)
}
fn reboot(r: &mut Rig, d: &LaunchDeployment) -> Result {
    r.flush();
    if let Some(a) = r
        .world()
        .world()
        .module_state::<crate::arena_bridge::ArenaRegistry>()
    {
        a.tasks().signal_stop_all();
    }
    r.restart(modules(d)?)?;
    logins(r)?;
    r.flush();
    Ok(())
}
fn names(r: &Rig) -> Result<Vec<String>> {
    Ok(r.world()
        .world()
        .module_state::<mush_roll::RollStore>()
        .ok_or("Roll")?
        .names(GOD)
        .into_iter()
        .map(str::to_owned)
        .collect())
}

#[tokio::test(flavor = "current_thread")]
async fn named_launch_real_initialization_cold_activation_and_private_import() -> Result {
    let (mut r, mut d) = setup()?;
    reboot(&mut r, &d)?;
    assert!(
        r.plan("industry", "issue_core_die", GOD, json!({"class":"i"}))
            .is_err()
    );
    assert!(r.action("market", "purchase_house_mind").is_err());
    r.initialize_roll()?;
    for action in [
        "initialize_core_origin",
        "initialize_core_production",
        "initialize_core_refit",
    ] {
        r.perform("industry", action, GOD, json!({}))?;
    }
    let image = r.boot.clone();
    d.mode = LaunchMode::Active;
    reboot(&mut r, &d)?;
    assert!(!image.is_empty());
    assert!(
        r.plan("industry", "issue_core_die", GOD, json!({"class":"i"}))
            .is_ok()
    );
    assert!(r.action("market", "purchase_house_mind").is_ok());
    assert!(r.action("roll", "conduct_trial_admit").is_ok());
    assert!(names(&r)?.is_empty());
    let before = r.boot.clone();
    command(
        r.world_mut(),
        GOD,
        "@mind/launch meridiani=Meridiani Field Series",
    )?;
    command(r.world_mut(), GOD, "@mind/launch coprates=Coprates Drill")?;
    command(r.world_mut(), GOD, "@mind/launch safe_conduct=Safe Conduct")?;
    r.flush();
    assert!(!same_boot(&r.boot, &before));
    for (recipe, house) in [
        (LaunchRecipe::Meridiani, &d.houses.meridiani),
        (LaunchRecipe::Coprates, &d.houses.coprates),
        (LaunchRecipe::SafeConduct, &d.houses.tycho),
    ] {
        let roll = r
            .world()
            .world()
            .module_state::<mush_roll::RollStore>()
            .ok_or("Roll")?;
        let mind = roll.mind(GOD, &house.source_name)?;
        assert_eq!(mind.draft(), &recipe.draft()?);
        assert!(mind.latest().is_none());
    }
    assert!(r.rows("roll_mind_initial_filing").is_empty());
    assert!(r.rows("roll_mind_house_term_economics").is_empty());
    let count = names(&r)?.len();
    r.world_mut()
        .run(GOD, "@mind/launch meridiani=Meridiani Field Series");
    assert_eq!(
        names(&r)?.len(),
        count,
        "duplicate import cannot replace the authored draft"
    );
    reboot(&mut r, &d)?;
    assert_eq!(names(&r)?.len(), 3);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn named_launch_exact_coprates_membership_and_strict_host_coordinates() -> Result {
    let (mut r, d) = setup()?;
    let bytes = serde_json::to_vec(&d)?;
    let mut invalid: Value = serde_json::from_slice(&bytes)?;
    invalid["eligible"] = json!(true);
    assert!(LaunchDeployment::decode(&serde_json::to_vec(&invalid)?).is_err());
    invalid = serde_json::from_slice(&bytes)?;
    invalid["houses"]["coprates"]["source_name"] = json!("bad=name");
    assert!(LaunchDeployment::decode(&serde_json::to_vec(&invalid)?).is_err());
    assert!(
        LaunchDeployment::decode(&vec![b' '; crate::launch::MAX_DEPLOYMENT_BYTES + 1]).is_err()
    );
    reboot(&mut r, &d)?;
    let service = r
        .world()
        .world()
        .module_state::<crate::launch::registration::LaunchService>()
        .ok_or("real launch slot")?
        .clone();
    let issuer = mush_roll::commerce::house_catalogue::HouseIssuer::new(
        d.houses.coprates.organization,
        d.houses.coprates.scope,
    )?;
    let other = mush_roll::commerce::house_catalogue::HouseIssuer::new(
        d.houses.meridiani.organization,
        d.houses.meridiani.scope,
    )?;
    let buyer = r.buyer;
    assert!(
        service
            .0
            .check_buyer(r.world().world(), issuer, buyer)
            .is_err()
    );
    assert!(
        service
            .0
            .check_buyer(r.world().world(), other, buyer)
            .is_ok()
    );
    command(
        r.world_mut(),
        GOD,
        "@faction/add Republic=Hardware buyer/level:1",
    )?;
    assert!(
        service
            .0
            .check_buyer(r.world().world(), issuer, buyer)
            .is_ok()
    );
    command(
        r.world_mut(),
        GOD,
        "@faction/remove Republic=Hardware buyer",
    )?;
    assert!(
        service
            .0
            .check_buyer(r.world().world(), issuer, buyer)
            .is_err()
    );
    // Org/scope identity is exact. No current holder, staff bit or account can
    // substitute membership for this one configured issuer.
    assert!(
        service
            .0
            .check_buyer(r.world().world(), issuer, GOD)
            .is_err()
    );
    let foreign = mush_roll::commerce::house_catalogue::HouseIssuer::new(
        OrgId(u64::MAX),
        OrgScopeId(u64::MAX),
    )?;
    assert!(
        service
            .0
            .check_buyer(r.world().world(), foreign, buyer)
            .is_ok()
    );
    Ok(())
}
