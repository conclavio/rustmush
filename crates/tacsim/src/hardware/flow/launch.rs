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
/// The login accounts: God's, the buyer's and, on format 2, one staff login
/// holding the clerk and every Registry operator (17g C1c, as the runbook and
/// mush-server's `launch_world` place them). A paid-resale Offer names its
/// buyer, and the writer requires seller and buyer on distinct logins.
fn logins(r: &mut Rig, d: &LaunchDeployment) -> Result {
    let mut values = BTreeMap::from([
        (GOD, mush_core::AccountId::mint("launch-publisher", 1)?),
        (r.buyer, mush_core::AccountId::mint("launch-buyer", 2)?),
    ]);
    if let Some(market) = &d.market {
        let staff = mush_core::AccountId::mint("launch-staff-two", 3)?;
        for actor in
            std::iter::once(market.clerk).chain(market.registry_authority.operators.clone())
        {
            values.insert(actor, staff);
        }
    }
    let accounts: Arc<dyn AccountIdentity> = Arc::new(Logins {
        values,
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
/// The named launch on the production format: format 2, with the player
/// market's identities (17g C1c; C1b's interim pinned format 1 here).
/// `setup_with(1)` is the House-only format.
fn setup() -> Result<(Rig, LaunchDeployment)> {
    setup_with(2)
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
            attestation_agent: None,
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
        procurement: Vec::new(),
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
        for capability in [
            "roll.registry.recall",
            "roll.registry.revoke",
            "roll.registry.inspect",
            // Doc 17h cut M8d D3: the runbook's fourth grant.
            "roll.registry.fine",
            // Doc 17h cut M8g G3: the fifth, the claims read (no projected verb).
            "roll.registry.claims",
        ] {
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
    let (mut modules, handles) = crate::composition::domain_modules_with_launch(
        Default::default(),
        Default::default(),
        &crate::economy::GameEconomyConfig {
            housing: false,
            market: false,
            ..Default::default()
        },
        &launch,
    )?;
    modules.push(Box::new(launch.configure_tacsim(
        layer.with_inventory(handles).with_unit_authority(),
    )?));
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
    logins(r, d)?;
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

/// 17g C1c: one action of each player-market owner the launch's Active branch
/// composes on format 2 (C1a's SURFACE), in the builder order:
/// - the v2 source-form purchase (`with_original_source_purchases`), an
///   authored fork, a dependent filing (which needs the marked Registry; that
///   step is a flag with no action of its own), a v3 entitlement step, the
///   compatible and derived line updates, the immediate charter Accept;
/// - Registry revocation, the pending-charter Fund and Settle
///   (`offer_mind_charter_transfer` is shared with the immediate charter
///   transfer, so it proves nothing alone), paid resale's Offer, Registry
///   recall;
/// - a personal renewal step and the personal renewal service's close
///   (`close_mind_renewal_service`, not the House one), author withdrawal,
///   owner attestation and parent re-qualification.
const MARKET_ACTIONS: [(&str, &str); 17] = [
    ("market", "purchase_original_mind"),
    ("roll", "fork_paid_mind"),
    ("roll", "dependent_initial_filing"),
    ("market", "list_derived_mind"),
    ("roll", "update_original_mind"),
    ("roll", "update_derived_mind"),
    ("roll", "accept_mind_charter_transfer"),
    ("roll", "registry_term_revocation"),
    ("roll", "fund_mind_charter_transfer"),
    ("roll", "settle_mind_charter_transfer"),
    ("market", mush_roll::commerce::paid_resale::protocol::OFFER),
    ("roll", "registry_revision_recall"),
    ("market", "draft_mind_renewal"),
    (
        "market",
        mush_economy::managed_renewal::SERVICE_CLOSE_ACTION,
    ),
    ("roll", "withdraw_authored_mind"),
    ("roll", "attest_owner_mind"),
    ("roll", "replace_owner_parent_qualification"),
];
/// 17g C1c: the player market's class-E verbs (C1 plan §7), offered to every
/// live player wherever it stands once their owners are composed: so one
/// player's projection shows whether any of these owners is composed.
/// Registry recall and revocation (class A) and the service verbs (class B)
/// are not here; `MARKET_ACTIONS` covers their owners.
///
/// 17g C2a (C2 plan §2.3) split the shared verbs one per kind, so this list
/// went from 22 to 26: `market-original-mind-offer` and
/// `market-original-mind-purchase` gave way to their generic kinds
/// (`-retire-listing`, `-retire-quote`, `-refund`; List, Quote, Fund and
/// Grant need the counter and Expire is God's), `paid-mind-resale` to its
/// three kinds, and the version-1 retirement gained its own verb beside
/// version 2's `roll-retire-authored-draft`.
const GENERIC_MARKET_VERBS: [&str; 26] = [
    "original-refund-v3",
    "derived-refund-v1",
    "market-original-mind-retire-listing",
    "market-original-mind-retire-quote",
    "market-original-mind-refund",
    "roll-file-dependent-mind",
    "roll-update-original-mind",
    "roll-update-derived-mind",
    "roll-fork-paid-mind",
    "roll-retire-authored-draft",
    "roll-retire-authored-draft-v1",
    "roll-offer-charter-transfer",
    "roll-accept-charter-transfer",
    "roll-offer-pending-charter",
    "roll-fund-pending-charter",
    "paid-mind-resale-offer",
    "paid-mind-resale-accept",
    "paid-mind-resale-fund",
    "mind-renewal-draft",
    "mind-renewal-fund",
    "mind-renewal-grant",
    "mind-renewal-refund",
    "mind-renewal-close",
    "roll-withdraw-authored-mind",
    "roll-attest-owner-mind",
    "roll-replace-owner-parent",
];
/// The verbs of `GENERIC_MARKET_VERBS` projected to `actor`, sorted.
fn generic_market_verbs(r: &mut Rig, actor: ObjId) -> Result<Vec<String>> {
    let mut verbs: Vec<String> = projected(r, actor)?
        .iter()
        .map(|offer| offer.presentation().verb.clone())
        .filter(|verb| GENERIC_MARKET_VERBS.contains(&verb.as_str()))
        .collect();
    verbs.sort();
    verbs.dedup();
    Ok(verbs)
}
/// The launch's three industry initialize actions, beside Roll's.
const INDUSTRY_INITIALIZE: [&str; 3] = [
    "initialize_core_origin",
    "initialize_core_production",
    "initialize_core_refit",
];
/// The runbook's bring-up on a rig: an Initialize boot, the Registry grants
/// when `grant` is set (format 2), the commerce and industry initialize
/// actions, then a cold Active boot.
fn activate(r: &mut Rig, d: &mut LaunchDeployment, grant: bool) -> Result {
    d.mode = LaunchMode::Initialize;
    reboot(r, d)?;
    r.initialize_roll()?;
    for action in INDUSTRY_INITIALIZE {
        r.perform("industry", action, GOD, json!({}))?;
    }
    if grant {
        grant_registry(r, d)?;
    }
    d.mode = LaunchMode::Active;
    reboot(r, d)
}
/// The offers `+actions` lists to `actor`: `Engine::legal_action_query` for the
/// actor as its own principal and subject, then `Engine::project_legal_actions`.
fn projected(r: &mut Rig, actor: ObjId) -> Result<Vec<mush_core::ProjectedLegalAction>> {
    let e = r.world_mut().engine_mut();
    e.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x46; 32])?,
        [],
    )?);
    let query = e.legal_action_query(
        mush_core::PrincipalId::Object { id: actor },
        actor,
        mush_core::PerceivedRef::SelfActor,
    )?;
    Ok(e.project_legal_actions(&query)?.projected())
}
/// The Registry verbs (`roll-registry-recall`, `roll-registry-revocation` and,
/// doc 17h cut M8a, `roll-registry-inspection`) projected to `actor`.
fn registry_verbs(r: &mut Rig, actor: ObjId) -> Result<Vec<String>> {
    Ok(projected(r, actor)?
        .iter()
        .map(|offer| offer.presentation().verb.clone())
        .filter(|verb| {
            matches!(
                verb.as_str(),
                "roll-registry-recall"
                    | "roll-registry-revocation"
                    | "roll-registry-inspection"
                    | "roll-registry-fine"
            )
        })
        .collect())
}
/// Every line bare `@mind/launch` sends `actor`, in order.
fn launch_lines(r: &mut Rig, actor: ObjId) -> Vec<String> {
    r.world_mut()
        .run(actor, "@mind/launch")
        .iter()
        .filter(|(audience, _)| *audience == mush_core::Audience::Actor(actor))
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}
/// God's readiness line for one operator.
fn readiness(operator: ObjId, state: &str) -> String {
    format!("Registry operators: #{} {state}.", operator.0)
}
/// 17g C1c: the market line with `MarketTerms::PROVISIONAL`, byte for byte.
const PROVISIONAL_MARKET_LINE: &str = "Player market: personal filing costs 100cr. A personal Pin listed at the counter sells for 400cr for 90 IC days plus 3 days Grace; the Registry takes 5%, any parent royalty is paid next, and the charter owner gets the rest. It renews for 400cr on its own accepted offer, open 3 IC days; a Pin bought from a market-original-mind-list listing does not renew. A licence resale offer stays open 1 day and pays the seller the rest; buying a resold House Pin needs the same standing as buying it new. A pending charter transfer settles within 3 IC days. Registry recall and revocation belong to the configured Registry operators. Read the listings here with +minds and your licences with +minds/holdings; help personal-minds lists every market command.";
/// The deployment's one Registry operator.
fn operator(d: &LaunchDeployment) -> Result<ObjId> {
    Ok(d.market
        .as_ref()
        .ok_or("a format-2 market")?
        .registry_authority
        .operators
        .first()
        .copied()
        .ok_or("one operator")?)
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
    // 17g C1c: Initialize composes no market owner; every one is Active-only.
    for (owner, id) in MARKET_ACTIONS {
        check!(r.action(owner, id).is_err(), "{owner}/{id} at Initialize");
    }
    r.initialize_roll()?;
    for action in INDUSTRY_INITIALIZE {
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
    // Doc 17h cut M8b-2 (H3): the launch composes the staff issues of heirloom and grey dies
    // beside the certified one; no vendor sells either (doc 17h §4).
    for action in [
        mush_industry::core_production::ISSUE_HEIRLOOM_ACTION,
        mush_industry::core_production::ISSUE_GREY_ACTION,
    ] {
        check!(r.action("industry", action).is_ok(), "{action} is composed");
    }
    check!(r.action("market", "purchase_house_mind").is_ok());
    check!(r.action("roll", "conduct_trial_admit").is_ok());
    // 17g C1c: the production format composes every player-market owner
    // after the House chain.
    for (owner, id) in MARKET_ACTIONS {
        check!(r.action(owner, id).is_ok(), "{owner}/{id} is composed");
    }
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

/// Doc 17h §3 M8f (P2; the fix round's H8): each procurement policy's
/// identities are checked at boot, one refusal per arm: the agent must be a
/// live Player-kind character and not GOING, the scope must be the House
/// organisation's ROOT scope (where its treasury pays a buy from), and a
/// named backing faction must exist. The valid policy boots, and the launch
/// hands its policies to the procurement tick.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_checks_procurement_identities_at_boot() -> Result {
    use crate::launch::{ProcurementPolicy, ProcurementRole};
    let disagree = "named launch procurement identities disagree";
    let policy = |r: &mut Rig, d: &LaunchDeployment| -> Result<ProcurementPolicy> {
        let org = d.houses.meridiani.organization;
        let root = r
            .world()
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state()
            .org(org)
            .ok_or("the House org")?
            .root_scope;
        let agent = r.world_mut().world_mut().create(
            ObjKind::Player,
            "Procurement agent",
            GOD,
            Some(d.venue.room),
        )?;
        Ok(ProcurementPolicy {
            org,
            scope: root,
            agent,
            budget_minor_per_ic_month: None,
            standing: Some(d.republic),
            cap_permille: None,
            roles: ProcurementRole::all(),
        })
    };
    let refuses =
        |change: &dyn Fn(&mut Rig, &mut ProcurementPolicy) -> Result, case: &str| -> Result {
            let (mut r, mut d) = setup_with(2)?;
            let mut p = policy(&mut r, &d)?;
            change(&mut r, &mut p)?;
            d.procurement = vec![p];
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
    refuses(
        &|r, p| {
            p.agent =
                r.world_mut()
                    .world_mut()
                    .create(ObjKind::Thing, "Agent stand-in", GOD, None)?;
            Ok(())
        },
        "an agent that is a Thing",
    )?;
    refuses(
        &|r, p| {
            r.world_mut()
                .world_mut()
                .get_mut(p.agent)
                .ok_or("the agent")?
                .flags
                .insert(mush_core::Flags::GOING);
            Ok(())
        },
        "an agent that is GOING",
    )?;
    refuses(
        &|_, p| {
            p.agent = ObjId(u32::MAX);
            Ok(())
        },
        "an agent that does not exist",
    )?;
    refuses(
        &|r, p| {
            // A real scope of another org (the Basin importer's die stock).
            p.scope = r.deployment.stock_scope;
            Ok(())
        },
        "a scope that is not the House's root",
    )?;
    refuses(
        &|_, p| {
            p.standing = Some(mush_faction::FactionId(u64::MAX));
            Ok(())
        },
        "a backing faction that does not exist",
    )?;
    let (mut r, mut d) = setup_with(2)?;
    let p = policy(&mut r, &d)?;
    d.procurement = vec![p.clone()];
    reboot(&mut r, &d)?;
    let state = r
        .world()
        .world()
        .module_state::<crate::procurement::tick::ProcurementState>()
        .ok_or("the procurement slot")?;
    check!(
        state.is_agent(p.org, p.agent),
        "the launch hands its policies to the tick"
    );
    Ok(())
}

/// Doc 17h §3 M8h (A1): each House's attestation agent is checked at boot, one
/// refusal per arm: a named `attestation_agent` must be a live Player-kind
/// character and not GOING, and a House with a procurement policy names no
/// other one (the policy's agent is its attesting character). The valid
/// deployment boots and hands the tick each House's agent: the policy's agent
/// for the House with a policy, the named one for a House without, none for
/// the House with neither.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_checks_house_attestation_agents_at_boot() -> Result {
    use crate::launch::{ProcurementPolicy, ProcurementRole};
    let disagree = "named launch House attestation identities disagree";
    let agent = |r: &mut Rig, d: &LaunchDeployment, name: &str| -> Result<ObjId> {
        Ok(r.world_mut()
            .world_mut()
            .create(ObjKind::Player, name, GOD, Some(d.venue.room))?)
    };
    let refuses = |change: &dyn Fn(&mut Rig, &mut LaunchDeployment) -> Result,
                   case: &str,
                   expected: &str|
     -> Result {
        let (mut r, mut d) = setup_with(2)?;
        change(&mut r, &mut d)?;
        match reboot(&mut r, &d) {
            Ok(()) => Err(format!("the launch booted with {case}").into()),
            Err(error) => {
                check!(
                    error.to_string().contains(expected),
                    "{case}: {error} lacks {expected:?}"
                );
                Ok(())
            }
        }
    };
    refuses(
        &|r, d| {
            d.houses.coprates.attestation_agent = Some(r.world_mut().world_mut().create(
                ObjKind::Thing,
                "Agent stand-in",
                GOD,
                None,
            )?);
            Ok(())
        },
        "an attestation agent that is a Thing",
        disagree,
    )?;
    refuses(
        &|r, d| {
            let named = agent(r, d, "Attestation agent")?;
            r.world_mut()
                .world_mut()
                .get_mut(named)
                .ok_or("the agent")?
                .flags
                .insert(mush_core::Flags::GOING);
            d.houses.coprates.attestation_agent = Some(named);
            Ok(())
        },
        "an attestation agent that is GOING",
        disagree,
    )?;
    refuses(
        &|_, d| {
            d.houses.coprates.attestation_agent = Some(ObjId(u32::MAX));
            Ok(())
        },
        "an attestation agent that does not exist",
        disagree,
    )?;
    let policy = |r: &mut Rig, d: &LaunchDeployment| -> Result<ProcurementPolicy> {
        let org = d.houses.meridiani.organization;
        let root = r
            .world()
            .world()
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state()
            .org(org)
            .ok_or("the House org")?
            .root_scope;
        Ok(ProcurementPolicy {
            org,
            scope: root,
            agent: agent(r, d, "Procurement agent")?,
            budget_minor_per_ic_month: None,
            standing: None,
            cap_permille: None,
            roles: ProcurementRole::all(),
        })
    };
    refuses(
        &|r, d| {
            d.procurement = vec![policy(r, d)?];
            d.houses.meridiani.attestation_agent = Some(agent(r, d, "Second agent")?);
            Ok(())
        },
        "a House naming an agent beside its procurement agent",
        "launch deployment House attestation differs",
    )?;
    let (mut r, mut d) = setup_with(2)?;
    let p = policy(&mut r, &d)?;
    let named = agent(&mut r, &d, "Coprates attestation agent")?;
    d.procurement = vec![p.clone()];
    // The policy's own agent named again is the same agent, not a second one.
    d.houses.meridiani.attestation_agent = Some(p.agent);
    d.houses.coprates.attestation_agent = Some(named);
    reboot(&mut r, &d)?;
    let state = r
        .world()
        .world()
        .module_state::<crate::house_attestation::HouseAttestationState>()
        .ok_or("the House attestation slot")?;
    check_eq!(
        state.agents(),
        &[
            (d.houses.meridiani.organization, p.agent),
            (d.houses.coprates.organization, named),
        ][..],
        "the policy's agent, the named one, none for Tycho"
    );
    Ok(())
}

/// 17g C1c: C1b's interim pin (format 2 composed no market owner) moves to
/// format 1, where it is permanent. A format-1 deployment initializes and
/// activates the House chain exactly as before C1: the House actions are
/// registered and no player-market action is, and bare `@mind/launch` prints
/// `HELP` and nothing else, for God and for a player.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_format_one_stays_house_only() -> Result {
    let (mut r, mut d) = setup_with(1)?;
    activate(&mut r, &mut d, false)?;
    check!(r.action("market", "purchase_house_mind").is_ok());
    check!(r.action("roll", "conduct_trial_admit").is_ok());
    // M8c F10 (the lead's ruling): the owner attestations are composed with
    // the House family, market or not, so format 1 registers them too.
    check!(r.action("roll", "attest_owner_mind").is_ok());
    check!(r.action("roll", "attest_house_mind").is_ok());
    for (owner, id) in MARKET_ACTIONS {
        if id == "attest_owner_mind" {
            continue;
        }
        check!(r.action(owner, id).is_err(), "{owner}/{id} on format 1");
    }
    let buyer = r.buyer;
    // No market verb is projected to anyone: a class-E verb would reach both,
    // so this covers every generic owner, and the Registry verbs besides.
    // `named_launch_market_help_states_the_deployment_terms` is the positive
    // control on format 2.
    for actor in [GOD, buyer] {
        // M8c F10: the owner attestation is composed with the House family,
        // so its verb may be offered on format 1; no other market verb is.
        let mut verbs = generic_market_verbs(&mut r, actor)?;
        verbs.retain(|verb| verb != "roll-attest-owner-mind");
        check_eq!(
            verbs,
            Vec::<String>::new(),
            "format 1 offers #{} no market verb",
            actor.0
        );
        check!(registry_verbs(&mut r, actor)?.is_empty());
    }
    for actor in [GOD, buyer] {
        check_eq!(
            launch_lines(&mut r, actor),
            vec![crate::launch::HELP.to_owned()],
            "format 1 prints HELP alone for #{}",
            actor.0
        );
    }
    reboot(&mut r, &d)?;
    Ok(())
}

/// 17g C1c (C1 plan §1 item 4, §7 class A): Registry recall and revocation —
/// and, doc 17h cut M8a, inspection — are offered only to a configured
/// operator whose `@org/grant`s are effective now, read through the projection
/// `+actions` lists. God's readiness line follows the same authorizer.
/// Retiring the Registry scope is the kill switch: all three offers vanish and
/// the world still reboots, and none returns after it.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_registry_offers_reach_only_the_granted_operators() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let market = d.market.clone().ok_or("a format-2 market")?;
    let operator = operator(&d)?;
    let principals = [GOD, operator, market.clerk, r.buyer];
    // Sorted: the four verbs `grant_registry`'s four grants open (M8d: the fine).
    let both = vec![
        "roll-registry-fine".to_owned(),
        "roll-registry-inspection".to_owned(),
        "roll-registry-recall".to_owned(),
        "roll-registry-revocation".to_owned(),
    ];
    let lacks = readiness(
        operator,
        "lacks recall, revocation, inspection, fine and claims",
    );
    // Before the grants: no one, and God reads what the operator lacks.
    for actor in principals {
        check!(
            registry_verbs(&mut r, actor)?.is_empty(),
            "#{} before the grants",
            actor.0
        );
    }
    check_eq!(launch_lines(&mut r, GOD).last(), Some(&lacks));
    // The runbook's grants: the operator alone, and God reads it ready.
    grant_registry(&mut r, &d)?;
    for actor in principals {
        let mut verbs = registry_verbs(&mut r, actor)?;
        verbs.sort();
        let expected = if actor == operator {
            both.clone()
        } else {
            Vec::new()
        };
        check_eq!(verbs, expected, "#{} after the grants", actor.0);
    }
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&readiness(operator, "ready"))
    );
    // Doc 17h cut M8a (F10): granted but not listed — the buyer holds the inspection's grant
    // on the Registry's scope, yet is no configured operator, so it is offered nothing.
    let buyer = r.buyer;
    let unlisted = format!(
        "@org/grant #{}=Registry roll.registry.inspect scope=#{}",
        buyer.0, market.registry_authority.scope.0
    );
    command(r.world_mut(), GOD, &unlisted)?;
    r.flush();
    check!(
        registry_verbs(&mut r, buyer)?.is_empty(),
        "a granted principal the deployment does not list"
    );
    // The kill switch: retire the Registry's scope (named by id).
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@org/scope retire Registry=#{}",
            market.registry_authority.scope.0
        ),
    )?;
    r.flush();
    for actor in principals {
        check!(
            registry_verbs(&mut r, actor)?.is_empty(),
            "#{} once the scope is retired",
            actor.0
        );
    }
    check_eq!(launch_lines(&mut r, GOD).last(), Some(&lacks));
    // A retired scope is a runtime state, so the world reboots, and still
    // no one is offered either action.
    reboot(&mut r, &d)?;
    for actor in principals {
        check!(
            registry_verbs(&mut r, actor)?.is_empty(),
            "#{} after the reboot",
            actor.0
        );
    }
    // The positive control: the operator's projection is read, not empty.
    // 17g C2a: any of the three split resale verbs (Offer, Accept, Fund).
    check!(
        projected(&mut r, operator)?.iter().any(|offer| matches!(
            offer.presentation().verb.as_str(),
            "paid-mind-resale-offer" | "paid-mind-resale-accept" | "paid-mind-resale-fund"
        )),
        "the operator is still offered the generic market actions"
    );
    check_eq!(launch_lines(&mut r, GOD).last(), Some(&lacks));
    Ok(())
}

/// 17g C1c (C1 plan §1 item 4): God's readiness line names, for each operator
/// in ascending order and in one line joined by ", ", exactly the capabilities
/// it lacks, so the runbook's reader re-runs the right `@org/grant`. Two
/// operators each hold one of the three grants, and each is projected only the
/// verb its grant opens; once both hold all three (doc 17h cut M8a's
/// inspection the third), both read ready and both are offered all three
/// verbs.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_readiness_names_what_each_operator_lacks() -> Result {
    let (mut r, mut d) = setup()?;
    // A second operator, spawned after the clerk, so the configured list
    // stays ascending as the document requires.
    let second = r.world_mut().spawn_player("Second operator");
    let first = operator(&d)?;
    check!(first.0 < second.0, "the operators are configured ascending");
    let market = d.market.as_mut().ok_or("a format-2 market")?;
    market.registry_authority.operators.push(second);
    let scope = market.registry_authority.scope;
    activate(&mut r, &mut d, false)?;
    let line = |items: [(ObjId, &str); 2]| {
        let items: Vec<String> = items
            .iter()
            .map(|(op, state)| format!("#{} {state}", op.0))
            .collect();
        format!("Registry operators: {}.", items.join(", "))
    };
    let grant = |r: &mut Rig, op: ObjId, capability: &str| -> Result {
        command(
            r.world_mut(),
            GOD,
            &format!(
                "@org/grant #{}=Registry {capability} scope=#{}",
                op.0, scope.0
            ),
        )?;
        r.flush();
        Ok(())
    };
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([
            (
                first,
                "lacks recall, revocation, inspection, fine and claims"
            ),
            (
                second,
                "lacks recall, revocation, inspection, fine and claims"
            ),
        ]))
    );
    // One grant each, a different one: each reads what it still lacks.
    grant(&mut r, first, "roll.registry.recall")?;
    grant(&mut r, second, "roll.registry.revoke")?;
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([
            (first, "lacks revocation, inspection, fine and claims"),
            (second, "lacks recall, inspection, fine and claims")
        ]))
    );
    check_eq!(
        registry_verbs(&mut r, first)?,
        vec!["roll-registry-recall".to_owned()]
    );
    check_eq!(
        registry_verbs(&mut r, second)?,
        vec!["roll-registry-revocation".to_owned()]
    );
    // The missing grants: both ready (M8a: the inspection is the third).
    grant(&mut r, first, "roll.registry.revoke")?;
    grant(&mut r, second, "roll.registry.recall")?;
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([
            (first, "lacks inspection, fine and claims"),
            (second, "lacks inspection, fine and claims")
        ]))
    );
    grant(&mut r, first, "roll.registry.inspect")?;
    grant(&mut r, second, "roll.registry.inspect")?;
    // Doc 17h cut M8d D3: the fine is the fourth, a grant of its own.
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([
            (first, "lacks fine and claims"),
            (second, "lacks fine and claims")
        ]))
    );
    grant(&mut r, first, "roll.registry.fine")?;
    grant(&mut r, second, "roll.registry.fine")?;
    // Doc 17h cut M8g G3: the claims read is the fifth, with no projected verb.
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([(first, "lacks claims"), (second, "lacks claims")]))
    );
    grant(&mut r, first, "roll.registry.claims")?;
    grant(&mut r, second, "roll.registry.claims")?;
    check_eq!(
        launch_lines(&mut r, GOD).last(),
        Some(&line([(first, "ready"), (second, "ready")]))
    );
    for op in [first, second] {
        let mut verbs = registry_verbs(&mut r, op)?;
        verbs.sort();
        check_eq!(
            verbs,
            vec![
                "roll-registry-fine".to_owned(),
                "roll-registry-inspection".to_owned(),
                "roll-registry-recall".to_owned(),
                "roll-registry-revocation".to_owned(),
            ],
            "#{} holds all four",
            op.0
        );
    }
    Ok(())
}

/// 17g C1c (C1 plan §4.2): bare `@mind/launch` on format 2 prints `HELP`, then
/// the market line rendered from the deployment's terms, to any actor, and
/// then God's readiness line to God alone.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_market_help_states_the_deployment_terms() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let buyer = r.buyer;
    // The positive control of `named_launch_format_one_stays_house_only`:
    // on format 2 the buyer is projected every class-E market verb.
    let mut every: Vec<String> = GENERIC_MARKET_VERBS.map(str::to_owned).to_vec();
    every.sort();
    check_eq!(generic_market_verbs(&mut r, buyer)?, every);
    let help = crate::launch::HELP.to_owned();
    let line = PROVISIONAL_MARKET_LINE.to_owned();
    check_eq!(
        launch_lines(&mut r, buyer),
        vec![help.clone(), line.clone()]
    );
    check_eq!(
        launch_lines(&mut r, GOD),
        vec![
            help,
            line,
            readiness(
                operator(&d)?,
                "lacks recall, revocation, inspection, fine and claims"
            )
        ]
    );
    // Terms that do not divide into whole credits or days: the price renders
    // with two decimals, an IC window in IC hours, the resale window in hours.
    let mut tuned = d.clone();
    let market = tuned.market.as_mut().ok_or("a format-2 market")?;
    market.terms.price_minor = 12_350;
    market.terms.renewal_offer_ic_seconds = 7_200;
    market.terms.resale_offer_seconds = 3_600;
    let tuned = LaunchDeployment::decode(&serde_json::to_vec(&tuned)?)?;
    check_eq!(
        crate::launch::market_help(tuned.market.as_ref().ok_or("a format-2 market")?),
        PROVISIONAL_MARKET_LINE
            .replacen("sells for 400cr", "sells for 123.50cr", 1)
            .replacen("open 3 IC days;", "open 2 IC hours;", 1)
            .replacen("stays open 1 day ", "stays open 1 hour ", 1)
    );
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
    let vocabulary = crate::composition::domain_org_capability_vocabulary();
    for capability in [
        mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
        "asset.title.transfer",
        "economy.account.post",
        "industry.core_die.issue",
        "vault.deposit",
        "vault.withdraw",
        "industry.core.integrate",
        "industry.core.refit",
        // Doc 17h cut M8b G8.
        "industry.core.ungovern",
        "roll.registry.recall",
        "roll.registry.revoke",
        "roll.registry.inspect",
        "roll.holder.org",
        "roll.registry.seal_mark",
    ] {
        check!(
            vocabulary.contains(&mush_org::CapabilityId::new(capability).ok_or("an id")?),
            "{capability}"
        );
    }
    Ok(())
}

/// 17g C2a test 4 (C2 plan §2.1; placed on the launch rig, which boots the
/// full player market Active through Initialize, because the providers
/// project nothing in an unbooted composition). Each C2a command's built
/// parameter object, from fixture values of the right types, resolves against
/// a live player's own projected offer for its registered Roll action through
/// `Engine::legal_action_query`, `project_legal_actions` and
/// `resolve_legal_action` (the path `install_submit` uses), and passes
/// `ActionProviderRegistry::validate_invocation` for that action. The format-2
/// Active boot's owner-read service and grant counts are printed.
#[tokio::test(flavor = "current_thread")]
async fn author_parameters_satisfy_the_registered_providers() -> Result {
    use crate::minds::market_commands::author;
    use mush_core::action::{
        IdempotencyKey, IdempotencyKeyText, IdempotencyScope, PerceivedRef, RequestUid,
        ScopedIdempotencyKey,
    };
    use mush_core::mechanics::{CanonicalId, DefinitionDigest};
    use mush_core::{OfferExecutionContext, OfferSubmission};
    use mush_roll::commerce::command_actions::RollCommandActions;
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let buyer = r.buyer;
    let payer = r.payer;
    let e = r.world_mut().engine_mut();
    let counts = e.registry.owner_read_counts().ok_or("frozen reads")?;
    eprintln!(
        "NAMED_LAUNCH_OWNER_READS services={} grants={} caps=({}, {})",
        counts.0,
        counts.1,
        mush_core::read_service::MAX_OWNER_READ_SERVICES,
        mush_core::read_service::MAX_OWNER_READ_GRANTS
    );
    check!(
        e.world
            .module_state::<crate::minds::market_commands::MindCommandService>()
            .is_some(),
        "the format-2 Active launch composes the command slot"
    );
    let actions = RollCommandActions::capture(e.registry.definitions().ok_or("definitions")?);
    let digest = |name: &str| DefinitionDigest::for_canonical_value("c2a.fixture.v1", &json!(name));
    let revision = mush_roll::RevisionRef {
        key: mush_roll::RevisionKey {
            character: buyer,
            mind: 1,
            number: 1,
        },
        digest: digest("revision")?,
    };
    let rating: mush_roll::commerce::rating::RatingRef = serde_json::from_value(json!({
        "job": mush_core::CommitToken::from_bytes([9; 16]),
        "id": digest("rating id")?,
        "schema": 1,
        "receipt_digest": digest("receipt")?,
    }))?;
    let shell = RequestUid::from_bytes([7; 16]);
    let cases = [
        ("reserve", actions.reserve(), author::reserve_parameters()),
        (
            "file original",
            actions.file(false),
            author::file_parameters(shell, &revision, &rating, payer, "Fixture Charter"),
        ),
        (
            "file dependent",
            actions.file(true),
            author::file_parameters(shell, &revision, &rating, payer, "Fixture Charter"),
        ),
        (
            "update original",
            actions.update(false),
            author::update_parameters(1, &revision, &rating, payer),
        ),
        (
            "update derived",
            actions.update(true),
            author::update_parameters(1, &revision, &rating, payer),
        ),
        (
            "withdraw",
            actions.withdraw(),
            author::withdraw_parameters(1),
        ),
        ("attest", actions.attest(), author::attest_parameters(1)),
        (
            "fork",
            actions.fork(),
            author::fork_parameters(1, "fixture fork"),
        ),
    ];
    e.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x47; 32])?,
        [],
    )?);
    for (serial, (name, action, parameters)) in cases.into_iter().enumerate() {
        let action = action.ok_or_else(|| format!("{name} is composed"))?;
        let principal = mush_core::PrincipalId::Object { id: buyer };
        let query = e.legal_action_query(principal.clone(), buyer, PerceivedRef::SelfActor)?;
        let offers = e.project_legal_actions(&query)?;
        let offer = offers
            .projected_for_action(&action)
            .ok_or_else(|| format!("{name} is offered to a live player"))?;
        let resolved = e
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: RequestUid::from_bytes([u8::try_from(serial)?.wrapping_add(40); 16]),
                    principal: principal.clone(),
                    actor: buyer,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: action.key().owner.clone(),
                            namespace: CanonicalId::new(action.key().id.as_str())?,
                        },
                        1,
                        IdempotencyKey::Caller(IdempotencyKeyText::parse_canonical(format!(
                            "mind.fixture.{serial}"
                        ))?),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: offer.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: parameters.clone(),
                },
            )
            .map_err(|error| format!("{name} resolves: {error}"))?;
        let (resolved_action, resolved, _) = resolved.into_authoritative_parts();
        check_eq!(&resolved_action, &action, "{name}");
        check_eq!(resolved.invocation().parameters(), &parameters, "{name}");
        e.registry
            .action_providers()
            .ok_or("providers")?
            .validate_invocation(&action, resolved.invocation())
            .map_err(|error| format!("{name} validates: {error}"))?;
    }
    Ok(())
}

/// 17g C2c test 6 (C2 plan §2.1, the C2c brief; on the launch rig beside C2a's
/// test 4, for the same reason: the providers project nothing unbooted). The
/// format-2 Active launch registers the market's command read
/// (`mind_market_commands` v2 since doc 17h M8f) and, since 17g C2e, grants it to TacSim (33
/// services and 35 grants with the scoped passenger facts; the counts and the
/// mechanics fingerprint are printed, the counts pinned). With the
/// buyer at the counter, the parameter objects `@mind/publish` builds for
/// the List offer, with and without the transfer mark, and `+minds/buy`'s
/// Quote, and the Fund, Grant and Refund wires `+minds/buy/pay`,
/// `/complete` and `/refund` submit (17g C2g: `market::fund_parameters` and
/// `terminal_parameters`, `{"quote"}` and `{"operation"}`,
/// `mind_entitlement::provider`'s own),
/// each resolve against the player's own projected offer for the action
/// `MarketCommandActions` captured, in both protocols, and pass
/// `validate_invocation`.
#[tokio::test(flavor = "current_thread")]
async fn market_parameters_satisfy_the_registered_providers() -> Result {
    use crate::minds::market_commands::market;
    use mush_core::action::{
        IdempotencyKey, IdempotencyKeyText, IdempotencyScope, PerceivedRef, RequestUid,
        ScopedIdempotencyKey,
    };
    use mush_core::mechanics::CanonicalId;
    use mush_core::{OfferExecutionContext, OfferSubmission};
    use mush_market::mind_commands::{MarketCommandActions, Protocol, Step};
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let buyer = r.buyer;
    let room = d.venue.room;
    r.world_mut().world_mut().move_to(buyer, room)?;
    let e = r.world_mut().engine_mut();
    let counts = e.registry.owner_read_counts().ok_or("frozen reads")?;
    eprintln!(
        "NAMED_LAUNCH_OWNER_READS services={} grants={} caps=({}, {})",
        counts.0,
        counts.1,
        mush_core::read_service::MAX_OWNER_READ_SERVICES,
        mush_core::read_service::MAX_OWNER_READ_GRANTS
    );
    eprintln!(
        "NAMED_LAUNCH_MECHANICS fingerprint={}",
        e.registry
            .mechanics_fingerprint()
            .map(|f| f.to_hex())
            .unwrap_or_default()
    );
    check_eq!(
        counts,
        (35, 38),
        "35 services and 38 grants: the DT.2b1 planning reader and its own requester, plus \
         Tacsim's unit-authority request, join DT.2a's 34 services and 36 grants; \
         the caps are 64/64"
    );
    let definitions = e.registry.definitions().ok_or("definitions")?;
    check!(
        definitions.iter().any(|entry| {
            let key = entry.reference().key();
            key.owner.as_str() == "market"
                && key.kind.as_str() == "owner_read"
                && key.id.as_str() == "mind_market_commands"
                && entry.reference().version() == 2
        }),
        "the market's command read is registered"
    );
    check!(
        e.world
            .module_state::<crate::minds::market_commands::MindCommandService>()
            .is_some_and(|service| service.market.is_some()),
        "TacSim holds the market read"
    );
    let actions = MarketCommandActions::capture(definitions);
    let licence = mush_roll::commerce::LicenceId::new(1)?;
    let token = mush_core::CommitToken::from_bytes([0x5c; 16]);
    let mut cases = Vec::new();
    for protocol in [Protocol::Original, Protocol::Derived] {
        cases.extend([
            (
                protocol,
                Step::List,
                market::list_parameters(licence, "sealed", 0, true),
            ),
            (
                protocol,
                Step::List,
                market::list_parameters(licence, "open_resale", 300, false),
            ),
            (protocol, Step::Quote, market::quote_parameters(token)),
            (protocol, Step::Fund, market::fund_parameters(token)),
            (protocol, Step::Grant, market::terminal_parameters(token)),
            (protocol, Step::Refund, market::terminal_parameters(token)),
        ]);
    }
    e.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x48; 32])?,
        [],
    )?);
    for (serial, (protocol, step, parameters)) in cases.into_iter().enumerate() {
        let name = format!("{protocol:?} {step:?} {parameters}");
        let action = actions
            .entitlement(protocol, step)
            .ok_or_else(|| format!("{name} is composed"))?;
        let principal = mush_core::PrincipalId::Object { id: buyer };
        let query = e.legal_action_query(principal.clone(), buyer, PerceivedRef::SelfActor)?;
        let offers = e.project_legal_actions(&query)?;
        let offer = offers
            .projected_for_action(&action)
            .ok_or_else(|| format!("{name} is offered at the counter"))?;
        let resolved = e
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: RequestUid::from_bytes([u8::try_from(serial)?.wrapping_add(80); 16]),
                    principal: principal.clone(),
                    actor: buyer,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: action.key().owner.clone(),
                            namespace: CanonicalId::new(action.key().id.as_str())?,
                        },
                        1,
                        IdempotencyKey::Caller(IdempotencyKeyText::parse_canonical(format!(
                            "mind.market.{serial}"
                        ))?),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: offer.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: parameters.clone(),
                },
            )
            .map_err(|error| format!("{name} resolves: {error}"))?;
        let (resolved_action, resolved, _) = resolved.into_authoritative_parts();
        check_eq!(&resolved_action, &action, "{name}");
        check_eq!(resolved.invocation().parameters(), &parameters, "{name}");
        e.registry
            .action_providers()
            .ok_or("providers")?
            .validate_invocation(&action, resolved.invocation())
            .map_err(|error| format!("{name} validates: {error}"))?;
    }
    Ok(())
}

/// 17g C2h (the lead's ruling R3, the C2e plan §5.1): on the format-2 Active
/// launch both renewal services register their `front` switch and delegate
/// its continuation to TacSim, and nothing else is delegated: exactly two
/// delegations, `@housemindrenewal/front` and `@mindrenewal/front`, each to
/// `tacsim` with the market's reason, and each service's player front is
/// live once the launch is activated: the handle `+minds/renew` is handed
/// (`player_front`, read here from the world as `player_front_in`: the
/// service composed and its runtime enabled) is the delegated one. On
/// format 1 (House-only) the House renewal service composes with its
/// `front` switch but no grantee (the player market, and so the `+minds`
/// slot, is not composed: `MindCommerce::configure_market` delegates only
/// where `configure_tacsim` sets `mind_commands`), so there is no
/// delegation at all, the House front's handle has no delegate, and
/// `+minds/renew` stays `NOT_OPEN` there.
///
/// The delegation enters no digest: the command table is not one of the
/// registries `Registry::mechanics_fingerprint` folds (mush-core's
/// `continuation::tests` proves a composition fingerprints identically with
/// and without one). The fingerprint is printed here; 17g C2h's builder
/// compared it before and after the change on this rig.
#[tokio::test(flavor = "current_thread")]
async fn named_launch_delegates_the_renewal_fronts_to_tacsim_on_format_two_only() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let e = r.world_mut().engine_mut();
    eprintln!(
        "NAMED_LAUNCH_MECHANICS fingerprint={}",
        e.registry
            .mechanics_fingerprint()
            .map(|f| f.to_hex())
            .unwrap_or_default()
    );
    let mut delegations: Vec<(String, String, String)> = e
        .registry
        .continuation_delegations()
        .into_iter()
        .map(|(command, grantee, reason)| (command, grantee.as_str().to_owned(), reason.to_owned()))
        .collect();
    delegations.sort();
    let reason = "17g C2h: +minds/renew hands the actor's own renewal step to the service";
    check_eq!(
        delegations,
        vec![
            (
                "@housemindrenewal/front".to_owned(),
                "tacsim".to_owned(),
                reason.to_owned()
            ),
            (
                "@mindrenewal/front".to_owned(),
                "tacsim".to_owned(),
                reason.to_owned()
            ),
        ],
        "exactly the two renewal fronts, both to TacSim"
    );
    for command in ["@mindrenewal", "@housemindrenewal"] {
        check!(
            e.registry
                .spec_of(command)
                .is_some_and(|spec| spec.switches.iter().any(|s| s == "front")),
            "{command} registers its front"
        );
    }
    let tacsim = mush_core::mechanics::OwnerId::new(crate::NS).map_err(|_| "TacSim's owner")?;
    for (command, front) in [
        (
            "@mindrenewal",
            mush_market::mind_renewal::player_front_in(&e.world),
        ),
        (
            "@housemindrenewal",
            mush_market::mind_house_renewal::player_front_in(&e.world),
        ),
    ] {
        let front = front.ok_or_else(|| format!("{command}'s player front is live"))?;
        check_eq!(
            e.registry.continuation_delegate(&front),
            Some(&tacsim),
            "{command}'s live front is the one delegated to TacSim"
        );
    }

    let (mut r, mut d) = setup_with(1)?;
    activate(&mut r, &mut d, false)?;
    let e = r.world_mut().engine_mut();
    check!(
        e.registry.continuation_delegations().is_empty(),
        "format 1 delegates nothing: {:?}",
        e.registry.continuation_delegations()
    );
    check!(
        e.registry
            .spec_of("@housemindrenewal")
            .is_some_and(|spec| spec.switches.iter().any(|s| s == "front")),
        "format 1 composes the House renewal service, its front undelegated"
    );
    let house = mush_market::mind_house_renewal::player_front_in(&e.world)
        .ok_or("format 1's House renewal service is live")?;
    check_eq!(e.registry.continuation_delegate(&house), None);
    check!(e.registry.spec_of("@mindrenewal").is_none());
    check!(mush_market::mind_renewal::player_front_in(&e.world).is_none());
    Ok(())
}

/// 17g C2d test 9 (C2 plan §2.1; on the launch rig beside C2a's test 4 and
/// C2c's test 6, for the same reason: the providers project nothing
/// unbooted). After the runbook's Registry grants, each C2d parameter
/// object resolves against its principal's own projected offer for the
/// action the commands capture, through `legal_action_query`,
/// `project_legal_actions` and `resolve_legal_action`, and passes
/// `ActionProviderRegistry::validate_invocation`: as the buyer, the
/// paid-resale Offer (`trade::resale_offer_parameters`), Accept and Fund
/// (`resale_accept_parameters`) and the pending charter's Offer
/// (`charter_offer_parameters`, decimal strings, 86,400 seconds) and Fund
/// (`charter_fund_parameters`); as the deployment's granted operator,
/// `roll-registry-recall` and `roll-registry-revocation`
/// (`registry::recall_parameters`, `revocation_parameters`), whose fixed
/// `reason` the projection adds. The buyer is offered neither Registry
/// action.
#[tokio::test(flavor = "current_thread")]
async fn trade_parameters_satisfy_the_registered_providers() -> Result {
    use crate::minds::market_commands::{registry, trade};
    use mush_core::action::{
        IdempotencyKey, IdempotencyKeyText, IdempotencyScope, PerceivedRef, RequestUid,
        ScopedIdempotencyKey,
    };
    use mush_core::mechanics::CanonicalId;
    use mush_core::{OfferExecutionContext, OfferSubmission};
    use mush_market::mind_commands::{MarketCommandActions, ResaleKind};
    use mush_roll::commerce::command_actions::RollCommandActions;
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    grant_registry(&mut r, &d)?;
    let operator = operator(&d)?;
    let buyer = r.buyer;
    let payer = r.payer;
    let e = r.world_mut().engine_mut();
    let definitions = e.registry.definitions().ok_or("definitions")?;
    let market = MarketCommandActions::capture(definitions);
    let roll = RollCommandActions::capture(definitions);
    let licence = mush_roll::commerce::LicenceId::new(1)?;
    let charter = mush_roll::commerce::CharterId::new(1)?;
    let token = mush_core::CommitToken::from_bytes([0x5d; 16]);
    let reason = json!({"reason": "registry_review"});
    let with_reason = |mut parameters: serde_json::Value| -> serde_json::Value {
        if let (Some(into), Some(from)) = (parameters.as_object_mut(), reason.as_object()) {
            into.extend(from.clone());
        }
        parameters
    };
    let cases = [
        (
            "resale offer",
            buyer,
            market.resale(ResaleKind::Offer),
            trade::resale_offer_parameters(licence, operator, 21, payer),
            None,
        ),
        (
            "resale accept",
            buyer,
            market.resale(ResaleKind::Accept),
            trade::resale_accept_parameters(token, payer),
            None,
        ),
        (
            "resale fund",
            buyer,
            market.resale(ResaleKind::Fund),
            trade::resale_accept_parameters(token, payer),
            None,
        ),
        (
            "pending charter offer",
            buyer,
            roll.pending_charter_offer(),
            trade::charter_offer_parameters(charter, operator, payer, 100),
            None,
        ),
        (
            "pending charter fund",
            buyer,
            roll.pending_charter_fund(),
            trade::charter_fund_parameters(token, payer),
            None,
        ),
        (
            "recall",
            operator,
            roll.recall(),
            registry::recall_parameters(1),
            Some(with_reason(registry::recall_parameters(1))),
        ),
        (
            "revocation",
            operator,
            roll.revocation(),
            registry::revocation_parameters(1),
            Some(with_reason(registry::revocation_parameters(1))),
        ),
    ];
    check_eq!(
        trade::charter_offer_parameters(charter, operator, payer, 100)
            .get("valid_for_seconds")
            .and_then(serde_json::Value::as_str),
        Some("86400")
    );
    e.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x49; 32])?,
        [],
    )?);
    for (serial, (name, actor, action, parameters, resolved_as)) in cases.into_iter().enumerate() {
        let action = action.ok_or_else(|| format!("{name} is composed"))?;
        let principal = mush_core::PrincipalId::Object { id: actor };
        let query = e.legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
        let offers = e.project_legal_actions(&query)?;
        let offer = offers
            .projected_for_action(&action)
            .ok_or_else(|| format!("{name} is offered to #{}", actor.0))?;
        let resolved = e
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: RequestUid::from_bytes([u8::try_from(serial)?.wrapping_add(120); 16]),
                    principal: principal.clone(),
                    actor,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: action.key().owner.clone(),
                            namespace: CanonicalId::new(action.key().id.as_str())?,
                        },
                        1,
                        IdempotencyKey::Caller(IdempotencyKeyText::parse_canonical(format!(
                            "mind.trade.{serial}"
                        ))?),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: offer.offer(),
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: parameters.clone(),
                },
            )
            .map_err(|error| format!("{name} resolves: {error}"))?;
        let (resolved_action, resolved, _) = resolved.into_authoritative_parts();
        check_eq!(&resolved_action, &action, "{name}");
        check_eq!(
            resolved.invocation().parameters(),
            &resolved_as.unwrap_or(parameters),
            "{name}"
        );
        e.registry
            .action_providers()
            .ok_or("providers")?
            .validate_invocation(&action, resolved.invocation())
            .map_err(|error| format!("{name} validates: {error}"))?;
    }
    // The Registry actions reach the granted operator alone.
    let query = e.legal_action_query(
        mush_core::PrincipalId::Object { id: buyer },
        buyer,
        PerceivedRef::SelfActor,
    )?;
    let offers = e.project_legal_actions(&query)?;
    for action in [roll.recall(), roll.revocation()] {
        let action = action.ok_or("composed")?;
        check!(
            offers.projected_for_action(&action).is_none(),
            "the buyer is not offered {action:?}"
        );
    }
    Ok(())
}

/// **Doc 17h cut M8d D7 (Q-M8-5).** The format-2 launch, which composes the Registry
/// inspection, makes the Red Dies front on the Republic's board at world build: one front, one
/// six-segment clock advancing on `minds.unwarranted`, none filled. A `minds.unwarranted` line
/// — which names nobody — moves it one step (the event emitted here, as the faction suite
/// does; the line's producer is the chronicle's tick); a restart makes no second front.
#[tokio::test(flavor = "current_thread")]
async fn m8d_the_launch_makes_the_red_dies_front_on_the_republic() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let red_dies = |r: &Rig| -> Result<(mush_faction::FrontId, mush_faction::Clock)> {
        let sim = r
            .world()
            .world()
            .module_state::<mush_faction::SimState>()
            .ok_or("fronts")?;
        let fronts: Vec<_> = sim
            .fronts
            .values()
            .filter(|f| f.title == mush_faction::RED_DIES_TITLE)
            .collect();
        check_eq!(fronts.len(), 1, "one Red Dies front");
        let front = fronts[0];
        check_eq!(front.faction, d.republic, "on the Republic's board");
        let clocks: Vec<_> = sim
            .clocks
            .values()
            .filter(|c| c.front == front.id)
            .collect();
        check_eq!(clocks.len(), 1);
        Ok((front.id, clocks[0].clone()))
    };
    let (front, clock) = red_dies(&r)?;
    check_eq!(clock.segments, mush_faction::RED_DIES_SEGMENTS);
    check_eq!(clock.filled, 0);
    check_eq!(
        clock.advance_on.as_deref(),
        Some(mush_faction::RED_DIES_KIND)
    );
    {
        let engine = r.world_mut().engine_mut();
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        engine.emit_world_event(
            &mush_core::WorldEvent::CanonRecorded {
                canon_id: 0,
                kind: crate::minds::registry_inspection::unwarranted::KIND.into(),
                entities: Vec::new(),
                source: String::new(),
            },
            &mut out,
            &mut fx,
        );
    }
    r.flush();
    check_eq!(red_dies(&r)?.1.filled, 1, "the front's clock one step on");
    reboot(&mut r, &d)?;
    let (again, _) = red_dies(&r)?;
    check_eq!(again, front, "a restart makes no second front");
    Ok(())
}

/// Every line `line` sends `actor`, in order.
fn replies(r: &mut Rig, actor: ObjId, line: &str) -> Vec<String> {
    r.world_mut()
        .run(actor, line)
        .iter()
        .filter(|(audience, _)| *audience == mush_core::Audience::Actor(actor))
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}

/// **Doc 17h cut M8g G3, branch (B)**: no IC detection exists until the fight (16c H1), so the
/// earned-contacts index has no producer and every hull an author names is refused with the
/// observer rule's one sentence (`+hull/box`'s, no new wording), for a player and for staff
/// alike, before anything else is asked; a claim naming no hull, or with no words, is its
/// usage. Nothing is filed.
#[tokio::test(flavor = "current_thread")]
async fn m8g_a_claim_on_a_hull_never_earned_is_refused_with_the_observer_sentence() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, true)?;
    let buyer = r.buyer;
    let hull = r
        .world_mut()
        .world_mut()
        .create(ObjKind::Thing, "A hull seen", GOD, None)?;
    r.flush();
    for actor in [buyer, GOD] {
        check_eq!(
            replies(
                &mut r,
                actor,
                &format!(
                    "@mind/claim #{} It pauses on a lost contact the way mine does.",
                    hull.0
                )
            ),
            vec![crate::surface::hull::NO_BOX.to_owned()],
            "#{} names a hull it never earned",
            actor.0
        );
    }
    let usage = crate::minds::registry_inspection::claims::USAGE.to_owned();
    for malformed in [
        "@mind/claim It pauses the way mine does.".to_owned(),
        format!("@mind/claim #{}", hull.0),
        format!("@mind/claim #0 {}", "words"),
    ] {
        check_eq!(replies(&mut r, buyer, &malformed), vec![usage.clone()]);
    }
    r.flush();
    check!(r.rows("roll_mind_claims").is_empty(), "nothing is filed");
    Ok(())
}

/// **Doc 17h cut M8g G3**: `@registry/claims` is a line-mode read gated live by
/// `roll.registry.claims` (the lead's ruling (a): no projected offer, no `registry_verbs`
/// entry). A non-operator and an ungranted operator get the driver's one sentence; a granted
/// operator reads that nothing is filed; once the grant lapses, the operator is refused again
/// with the same sentence — across a restart.
#[tokio::test(flavor = "current_thread")]
async fn m8g_registry_claims_reach_only_an_operator_granted_now() -> Result {
    let (mut r, mut d) = setup()?;
    activate(&mut r, &mut d, false)?;
    let operator = operator(&d)?;
    let market = d.market.clone().ok_or("a format-2 market")?;
    let refused = vec![mush_core::action::SUBMISSION_REFUSED_LINE.to_owned()];
    for actor in [r.buyer, market.clerk, operator] {
        check_eq!(
            replies(&mut r, actor, "@registry/claims"),
            refused.clone(),
            "#{} before the grant",
            actor.0
        );
    }
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@org/grant #{}=Registry roll.registry.claims scope=#{}",
            operator.0, market.registry_authority.scope.0
        ),
    )?;
    r.flush();
    check_eq!(
        replies(&mut r, operator, "@registry/claims"),
        vec!["No claims are filed with the Registry.".to_owned()]
    );
    check!(
        registry_verbs(&mut r, operator)?.is_empty(),
        "the read projects no verb"
    );
    for actor in [r.buyer, market.clerk] {
        check_eq!(replies(&mut r, actor, "@registry/claims"), refused.clone());
    }
    // The grant lapses: the operator is refused again, with the same sentence.
    let now = r.world().world().now();
    let org = r.boot.get_mut("org").ok_or("the org boot data")?;
    let mut lapsed = 0;
    for (key, value) in &mut org.kv {
        if key.starts_with("grant:")
            && value.get("grantee") == Some(&json!(operator.0))
            && value.get("capability") == Some(&json!("roll.registry.claims"))
            && let Some(grant) = value.as_object_mut()
        {
            grant.insert("valid_until".into(), json!(now));
            lapsed += 1;
        }
    }
    check_eq!(lapsed, 1);
    reboot(&mut r, &d)?;
    check_eq!(replies(&mut r, operator, "@registry/claims"), refused);
    Ok(())
}
