//! Real launch configuration and ordinary operator setup. This suite exercises
//! initialization, cold activation and the actual private source import. It does
//! not turn a source hash into a certificate, filing, sale or Conduct result.
use super::*;
#[path = "launch/story.rs"]
mod story;
use crate::launch::{
    HardwareCoordinates, HouseDeployment, LaunchComposition, LaunchDeployment, LaunchMode,
    LaunchRecipe, LaunchVenue, MarketTerms, NamedHouses, PlayerMarket, RegistryAuthority,
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
/// The format-1 (House-only) named launch. C1c moves it to format 2.
fn setup() -> Result<(Rig, LaunchDeployment)> {
    setup_with(1)
}
/// The named launch's identities, made by ordinary operator commands. Format 2
/// (17g C1b) adds the player market's: a `Registry` org with a `review` scope,
/// a `Registry operator` and a `Market clerk` character, and the provisional
/// terms. It grants nothing; `grant_registry` runs the runbook's grants.
fn setup_with(format: u16) -> Result<(Rig, LaunchDeployment)> {
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
            // Exactly one launch filing fee (`launch::content::FILING_MINOR`, in
            // minor units); the story asserts the treasury pays exactly that.
            &format!("@econ/mint {}={} 10000", faucet.0, treasury.0),
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
    let market = match format {
        1 => None,
        2 => Some(registry_market(&mut r)?),
        _ => return Err("a known launch deployment format".into()),
    };
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
        format,
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
        market,
    };
    r.flush();
    Ok((r, deployment))
}
/// Format 2's market identities, by the runbook's commands: the `Registry`
/// org and its `review` scope, one operator and the clerk. Nothing is granted.
fn registry_market(r: &mut Rig) -> Result<PlayerMarket> {
    for line in ["@org/create Registry", "@org/scope create Registry=review"] {
        command(r.world_mut(), GOD, line)?;
    }
    let state = r
        .world()
        .world()
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state();
    let organization = state
        .orgs()
        .find(|o| o.name.as_str() == "Registry")
        .ok_or("actual Registry")?
        .id;
    let scope = state
        .scopes_in(organization)
        .find(|s| s.name.as_str() == "review")
        .ok_or("actual review scope")?
        .id;
    let operator = r.world_mut().spawn_player("Registry operator");
    let clerk = r.world_mut().spawn_player("Market clerk");
    Ok(PlayerMarket {
        clerk,
        registry_authority: RegistryAuthority {
            organization,
            scope,
            operators: vec![operator],
        },
        terms: MarketTerms::PROVISIONAL,
    })
}
/// The runbook's Registry grants: both capabilities to each configured
/// operator on the exact Registry scope, by the baseline `@org/grant` as GOD
/// (C1 plan §1 item 4), at Initialize. `setup_with` grants nothing.
fn grant_registry(r: &mut Rig, d: &LaunchDeployment) -> Result {
    let market = d.market.as_ref().ok_or("a format-2 deployment")?;
    let authority = &market.registry_authority;
    for operator in &authority.operators {
        for capability in ["roll.registry.recall", "roll.registry.revoke"] {
            command(
                r.world_mut(),
                GOD,
                &format!(
                    "@org/grant #{}=Registry {capability} scope=#{}",
                    operator.0, authority.scope.0
                ),
            )?;
        }
    }
    // Retain the grants before any authoritative action, as a commit would.
    r.flush();
    Ok(())
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
    )?;
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
    check!(crate::test_checks::refused(r.plan(
        "industry",
        "issue_core_die",
        GOD,
        json!({"class":"i"})
    ))?);
    check!(r.action("market", "purchase_house_mind").is_err());
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
    check!(!image.is_empty());
    check!(
        r.plan("industry", "issue_core_die", GOD, json!({"class":"i"}))
            .is_ok()
    );
    check!(r.action("market", "purchase_house_mind").is_ok());
    check!(r.action("roll", "conduct_trial_admit").is_ok());
    check!(names(&r)?.is_empty());
    let before = r.boot.clone();
    command(
        r.world_mut(),
        GOD,
        "@mind/launch meridiani=Meridiani Field Series",
    )?;
    command(r.world_mut(), GOD, "@mind/launch coprates=Coprates Drill")?;
    command(r.world_mut(), GOD, "@mind/launch safe_conduct=Safe Conduct")?;
    r.flush();
    check!(!same_boot(&r.boot, &before));
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
        check_eq!(mind.draft(), &recipe.draft()?);
        check!(mind.latest().is_none());
    }
    check!(r.rows("roll_mind_initial_filing").is_empty());
    check!(r.rows("roll_mind_house_term_economics").is_empty());
    let count = names(&r)?.len();
    r.world_mut()
        .run(GOD, "@mind/launch meridiani=Meridiani Field Series");
    check_eq!(
        names(&r)?.len(),
        count,
        "duplicate import cannot replace the authored draft"
    );
    reboot(&mut r, &d)?;
    check_eq!(names(&r)?.len(), 3);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn named_launch_exact_coprates_membership_and_strict_host_coordinates() -> Result {
    let (mut r, d) = setup()?;
    let bytes = serde_json::to_vec(&d)?;
    let mut invalid: Value = serde_json::from_slice(&bytes)?;
    invalid["eligible"] = json!(true);
    check!(LaunchDeployment::decode(&serde_json::to_vec(&invalid)?).is_err());
    invalid = serde_json::from_slice(&bytes)?;
    invalid["houses"]["coprates"]["source_name"] = json!("bad=name");
    check!(LaunchDeployment::decode(&serde_json::to_vec(&invalid)?).is_err());
    check!(LaunchDeployment::decode(&vec![b' '; crate::launch::MAX_DEPLOYMENT_BYTES + 1]).is_err());
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
    check!(
        service
            .0
            .check_buyer(r.world().world(), issuer, buyer)
            .is_err()
    );
    check!(
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
    check!(
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
    check!(
        service
            .0
            .check_buyer(r.world().world(), issuer, buyer)
            .is_err()
    );
    // Org/scope identity is exact. No current holder, staff bit or account can
    // substitute membership for this one configured issuer.
    check!(
        service
            .0
            .check_buyer(r.world().world(), issuer, GOD)
            .is_err()
    );
    let foreign = mush_roll::commerce::house_catalogue::HouseIssuer::new(
        OrgId(u64::MAX),
        OrgScopeId(u64::MAX),
    )?;
    check!(
        service
            .0
            .check_buyer(r.world().world(), foreign, buyer)
            .is_ok()
    );
    Ok(())
}

/// Decode `value` and require a refusal whose text contains `message`.
fn decode_refuses(value: &Value, message: &str, case: &str) -> Result {
    match LaunchDeployment::decode(&serde_json::to_vec(value)?) {
        Ok(_) => Err(format!("decode admitted {case}").into()),
        Err(error) => {
            check!(
                error.to_string().contains(message),
                "{case}: {error} lacks {message:?}"
            );
            Ok(())
        }
    }
}

/// 17g C1b: format 2 is strict at decode, and format 1's identity is unchanged.
/// A format-1 document still serializes to exactly its eight pre-C1 keys, so
/// the `launch_deployment` descriptor (the deployment minus `mode`) of a
/// format-1 world, and its digest, do not move.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_format_two_is_strict_and_format_one_identity_is_unchanged() -> Result {
    let (_, one) = setup_with(1)?;
    let value = serde_json::to_value(&one)?;
    let mut keys: Vec<&str> = value
        .as_object()
        .ok_or("a deployment object")?
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    check_eq!(
        keys,
        [
            "format",
            "hardware",
            "houses",
            "mode",
            "registry",
            "registry_issuer",
            "republic",
            "venue"
        ]
    );
    check!(
        LaunchDeployment::decode(&serde_json::to_vec(&value)?)?
            .market
            .is_none()
    );
    let identity = "launch deployment identity differs";
    let market = "launch deployment player market differs";
    let unknown = "invalid launch deployment document";
    let (_, two) = setup_with(2)?;
    let valid = serde_json::to_value(&two)?;
    // Format and market agree, or the identity differs.
    let mut case = value.clone();
    case["market"] = valid["market"].clone();
    decode_refuses(&case, identity, "format 1 with a market")?;
    let mut case = valid.clone();
    case.as_object_mut()
        .ok_or("a deployment object")?
        .remove("market");
    decode_refuses(&case, identity, "format 2 without a market")?;
    let mut case = valid.clone();
    case["format"] = json!(3);
    decode_refuses(&case, identity, "format 3")?;
    // Every market object is closed.
    for (pointer, case_name) in [
        ("/market", "an unknown key inside market"),
        (
            "/market/registry_authority",
            "an unknown key inside registry_authority",
        ),
        ("/market/terms", "an unknown key inside terms"),
    ] {
        let mut case = valid.clone();
        case.pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .ok_or(case_name)?
            .insert("eligible".into(), json!(true));
        decode_refuses(&case, unknown, case_name)?;
    }
    // The clerk and the operators.
    let operator = two
        .market
        .as_ref()
        .ok_or("a format-2 market")?
        .registry_authority
        .operators
        .first()
        .copied()
        .ok_or("one operator")?;
    let set = |pointer: &str, replacement: Value, case_name: &str| -> Result {
        let mut case = valid.clone();
        *case.pointer_mut(pointer).ok_or("a market field")? = replacement;
        decode_refuses(&case, market, case_name)
    };
    set("/market/clerk", json!(GOD.0), "clerk #1")?;
    set("/market/clerk", json!(0), "clerk #0")?;
    let operators = "/market/registry_authority/operators";
    set(operators, json!([]), "no operators")?;
    set(
        operators,
        json!((100..133).collect::<Vec<u32>>()),
        "33 operators",
    )?;
    set(
        operators,
        json!([operator.0 + 1, operator.0]),
        "unsorted operators",
    )?;
    set(
        operators,
        json!([operator.0, operator.0]),
        "duplicated operators",
    )?;
    set(operators, json!([0, operator.0]), "operator #0")?;
    // The Registry organization is its own.
    set(
        "/market/registry_authority/organization",
        json!(two.houses.coprates.organization.0),
        "the Registry org equal to a House org",
    )?;
    set(
        "/market/registry_authority/organization",
        json!(two.hardware.importer.0),
        "the Registry org equal to the importer",
    )?;
    // The personal levy's recipient is a system principal.
    set(
        "/registry/owner",
        serde_json::to_value(PrincipalId::Object(operator))?,
        "a non-system Registry recipient",
    )?;
    // Each term at 0 and one past its bound.
    for (term, bound) in [
        ("price_minor", 100_000_000),
        ("renewal_price_minor", 100_000_000),
        (
            "quote_seconds",
            mush_market::mind_trade::records::MAX_QUOTE_SECONDS,
        ),
        (
            "renewal_offer_ic_seconds",
            mush_economy::managed_renewal::MAX_RENEWAL_OFFER_IC_SECONDS,
        ),
        (
            "resale_offer_seconds",
            mush_economy::managed_paid_resale::MAX_OFFER_SECONDS,
        ),
        (
            "charter_settle_ic_seconds",
            mush_economy::managed_charter_transfer::MAX_CHARTER_SETTLEMENT_IC_SECONDS,
        ),
    ] {
        let pointer = format!("/market/terms/{term}");
        set(&pointer, json!(0), &format!("{term} at 0"))?;
        set(
            &pointer,
            json!(bound + 1),
            &format!("{term} one past its bound"),
        )?;
        // The bound itself decodes: the refusal is the bound, not the value.
        let mut case = valid.clone();
        *case.pointer_mut(&pointer).ok_or("a term")? = json!(bound);
        check!(
            LaunchDeployment::decode(&serde_json::to_vec(&case)?).is_ok(),
            "{term} at its bound"
        );
    }
    // A valid format 2 decodes and round-trips, byte for byte.
    let bytes = serde_json::to_vec(&two)?;
    let decoded = LaunchDeployment::decode(&bytes)?;
    check_eq!(serde_json::to_vec(&decoded)?, bytes);
    check_eq!(decoded.market, two.market);
    Ok(())
}

/// 17g C1b: boot checks the market's world identities, in the House check's
/// shape (existence, ownership, kind), and never their status. Each refusal
/// boots a fresh rig, because a refused restart consumes the rig's world;
/// every rig is made by the same commands, so its ids are the same.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_format_two_checks_market_identities_at_boot() -> Result {
    let disagree = "named launch market identities disagree";
    let refuses = |change: &dyn Fn(&mut Rig, &mut PlayerMarket) -> Result, case: &str| -> Result {
        let (mut r, mut d) = setup_with(2)?;
        change(&mut r, d.market.as_mut().ok_or("a format-2 market")?)?;
        match reboot(&mut r, &d) {
            Ok(()) => Err(format!("the launch booted with {case}").into()),
            Err(error) => {
                check!(
                    error.to_string().contains(disagree),
                    "{case}: {error} lacks {disagree:?}"
                );
                Ok(())
            }
        }
    };
    let thing = |r: &mut Rig, name: &str| -> Result<ObjId> {
        Ok(r.world_mut()
            .world_mut()
            .create(ObjKind::Thing, name, GOD, None)?)
    };
    refuses(
        &|r, m| {
            m.registry_authority.operators = vec![thing(r, "Registry operator stand-in")?];
            Ok(())
        },
        "an operator that is a Thing",
    )?;
    refuses(
        &|r, m| {
            m.clerk = thing(r, "Market clerk stand-in")?;
            Ok(())
        },
        "a clerk that is a Thing",
    )?;
    refuses(
        &|r, m| {
            // A real scope of another org (the Basin importer's die stock).
            m.registry_authority.scope = r.deployment.stock_scope;
            Ok(())
        },
        "a scope of another org",
    )?;
    refuses(
        &|_, m| {
            m.registry_authority.organization = OrgId(u64::MAX);
            Ok(())
        },
        "an org id that does not exist",
    )?;
    // The valid identities boot, and keep booting once the Registry's scope is
    // retired: scope status is a runtime state the providers check.
    let (mut r, d) = setup_with(2)?;
    reboot(&mut r, &d)?;
    let scope = d
        .market
        .as_ref()
        .ok_or("a format-2 market")?
        .registry_authority
        .scope;
    command(
        r.world_mut(),
        GOD,
        &format!("@org/scope retire Registry=#{}", scope.0),
    )?;
    let retired = |r: &Rig| -> Result<bool> {
        Ok(r.world()
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state()
            .scope(scope)
            .is_some_and(|s| s.status != mush_org::ScopeStatus::Active))
    };
    check!(retired(&r)?, "the Registry scope is retired");
    reboot(&mut r, &d)?;
    // The reboot hydrated the org store from the boot image: the launch it
    // booted saw the retired scope, not an Active one (the kill switch).
    check!(
        retired(&r)?,
        "the rebooted launch hydrated the retired Registry scope"
    );
    let service = r
        .world()
        .world()
        .module_state::<crate::launch::registration::LaunchService>()
        .ok_or("real launch slot")?;
    check_eq!(service.0.deployment().market, d.market);
    Ok(())
}

/// 17g C1b, INTERIM: C1c deletes this test. A format-2 deployment initializes
/// and activates with its market identities checked, but composes only the
/// House chain until C1c composes the market from `launch::market::policies`.
/// This pins that interim state, so that no player-market owner is reachable
/// through the launch before C1c's census and privacy proofs land with it.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_format_two_composes_no_market_owner_yet() -> Result {
    let (mut r, mut d) = setup_with(2)?;
    reboot(&mut r, &d)?;
    // The runbook's grants, made at Initialize by the baseline command.
    grant_registry(&mut r, &d)?;
    r.initialize_roll()?;
    for action in [
        "initialize_core_origin",
        "initialize_core_production",
        "initialize_core_refit",
    ] {
        r.perform("industry", action, GOD, json!({}))?;
    }
    d.mode = LaunchMode::Active;
    reboot(&mut r, &d)?;
    check!(r.action("market", "purchase_house_mind").is_ok());
    check!(r.action("roll", "conduct_trial_admit").is_ok());
    check!(r.action("roll", "registry_revision_recall").is_err());
    check!(r.action("roll", "registry_term_revocation").is_err());
    // One v3 entitlement action and one personal renewal action.
    check!(r.action("market", "list_derived_mind").is_err());
    check!(r.action("market", "draft_mind_renewal").is_err());
    reboot(&mut r, &d)?;
    Ok(())
}

/// 17g C1b, a stop-rule fix older than C1: every capability the named launch's
/// operator grants with `@org/grant` (the House publishers', the Basin
/// importer's stock and factory agents', and the Registry operators') is in
/// the production org vocabulary. This suite's rig composes an org module
/// with no vocabulary, so it never saw that the production `@org/grant`
/// refused the three hardware ones (`industry.core_die.issue`,
/// `industry.core.integrate`, `industry.core.refit`); the first named launch
/// through `game_modules` (mush-server's `player_market.rs`) did.
#[test]
fn the_org_vocabulary_admits_every_named_launch_grant() -> Result {
    let vocabulary = crate::composition::org_capability_vocabulary();
    for capability in [
        mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
        "asset.title.transfer",
        "economy.account.post",
        "industry.core_die.issue",
        "vault.deposit",
        "vault.withdraw",
        "industry.core.integrate",
        "industry.core.refit",
        "roll.registry.recall",
        "roll.registry.revoke",
    ] {
        check!(
            vocabulary.contains(&mush_org::CapabilityId::new(capability).ok_or("an id")?),
            "{capability}"
        );
    }
    Ok(())
}
