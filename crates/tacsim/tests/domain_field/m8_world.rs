//! **Doc 17h cut M8g G1 — the combined M8 world.** One composed world holds every M8 owner at
//! once: the production `LaunchComposition` (the named House chain and the player market, the
//! Registry's recall, revocation, inspection and fine, the governed hardware with its fitted
//! catalog and refit, the procurement policy slot) over the test world's layer with the shipped
//! game content — so the corridor's ports, the Dray-P charter and refit, Occator's rig leases and
//! the surface excursion run in the same world as the Roll, the market and the Registry.
//!
//! It is `excursions.rs`'s rig in its launch mode (`Roster::launch`), booted as the production
//! launch boots: the port world first (the content seeds and stocks the ports and the pilot flies
//! a Dray-P to Ahuna Flank's pad), then the launch's identities made by the runbook's own
//! operator commands (the named launch story's `setup_with` and the hardware flow's `provision`,
//! ported here, `crates/tacsim/src/hardware/flow{,/launch}.rs`), the launch booted under
//! `Initialize`, its setup actions landed, the Registry's grants made, and the launch rebooted
//! `Active`. No module is impersonated and no typed row is seeded: every record is an action's.
//!
//! The rated half ([`rate_and_list`]) runs the real filing and the real rating — the launch's
//! `@mind/launch` import, `@mind/save`, `@registry/rate` through the Registry service, the House
//! filing with its certificate, the counter listing — for the three named Houses' sources, and
//! sells one paid House Pin to the pilot. It takes genuine Registry trials, so every test over it
//! is an `#[ignore]`d story the CI dispatch runs (doc 17h §3 M8g G5; the owner's rule: stories
//! never run locally).

use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_core::{AuthoritativeSubmit, DefinitionRef, GOD, ObjId, PerceivedRef};
use mush_economy::{AccountId, VaultId};
use mush_industry::core_production::DieClass;
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use serde_json::{Value, json};
use tacsim::launch::{
    HardwareCoordinates, HouseDeployment, LaunchComposition, LaunchDeployment, LaunchMode,
    LaunchVenue, MarketTerms, NamedHouses, PlayerMarket, RegistryAuthority,
};
use tacsim::surface::black_box::BlackBoxLine;

use crate::excursions::{Rig, economy, god, land_commit, launch_layer, reboot_with, run, stocked};

/// **The combined world, booted Active**: the rig, the deployment it was composed from, the
/// pilot (the Dray-P's, on Ahuna Flank's pad with its rigs in the bay), the Dray-P's map row
/// and the pilot's rigs.
pub(crate) struct M8World {
    pub(crate) rig: Rig,
    pub(crate) deployment: LaunchDeployment,
    pub(crate) pilot: ObjId,
    pub(crate) row: tacsim::port::HullRow,
    pub(crate) tvs: Vec<ObjId>,
    serial: u64,
}

fn org(rig: &Rig, name: &str) -> OrgId {
    rig.tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the Org owner")
        .state()
        .orgs()
        .find(|o| o.name.as_str() == name)
        .unwrap_or_else(|| panic!("the organisation {name}"))
        .id
}

fn scope(rig: &Rig, org: OrgId, name: &str) -> OrgScopeId {
    rig.tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the Org owner")
        .state()
        .scopes_in(org)
        .find(|s| s.name.as_str() == name)
        .unwrap_or_else(|| panic!("the scope {name}"))
        .id
}

fn account(rig: &Rig, owner: &PrincipalId, scope: Option<OrgScopeId>) -> AccountId {
    economy(&rig.tw)
        .snapshot()
        .accounts()
        .find(|a| &a.owner == owner && (scope.is_none() || a.scope == scope))
        .unwrap_or_else(|| panic!("the account of {owner:?}"))
        .id
}

fn system(name: &str) -> PrincipalId {
    PrincipalId::System(SystemPrincipalId::new(name).expect("a system principal"))
}

fn vault(rig: &Rig, name: &str) -> VaultId {
    economy(&rig.tw)
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == name)
        .unwrap_or_else(|| panic!("the vault {name}"))
        .id
}

fn thing(rig: &mut Rig, name: &str) -> ObjId {
    rig.tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, name, GOD, None)
        .expect("a thing")
}

/// **The governed hardware's coordinates** (the hardware flow's `provision`, ported): the Basin
/// importer and its die-stock and factory scopes and treasuries, the die depot and the two
/// agents' stock vaults, the fitting shop chartered `core_integration core_refit` and titled to
/// Basin, and the agents' grants — by the runbook's commands. Beside it, the Tether back room
/// (the hardware flow's ungoverning shop): its own organisation and `backroom` scope and
/// treasury, a facility chartered `core_ungovern` and titled to Tether, and GOD's and the
/// agent's `industry.core.ungovern` grants, named in the deployment's optional ungovern site
/// (doc 17h cut M8g, the lead's ruling (a)); a shipped deployment names none.
fn hardware(rig: &mut Rig) -> HardwareCoordinates {
    let agent = rig.tw.spawn_player("Hardware stock agent");
    let factory = thing(rig, "Basin fitting shop");
    let output = thing(rig, "Basin completed work");
    let containers = [
        thing(rig, "Basin die depot"),
        thing(rig, "Basin agent stock one"),
        thing(rig, "Basin agent stock two"),
    ];
    for line in [
        "@org/create Basin",
        "@org/scope create Basin=die-stock",
        "@org/scope create Basin=factory",
    ] {
        god(rig, line);
    }
    let basin = org(rig, "Basin");
    let stock_scope = scope(rig, basin, "die-stock");
    let factory_scope = scope(rig, basin, "factory");
    for (i, c) in containers.into_iter().enumerate() {
        god(
            rig,
            &format!(
                "@vault/create HardwareStock{i}=container:#{} owner=org:Basin scope={}",
                c.0, stock_scope.0
            ),
        );
    }
    for scope in [stock_scope, factory_scope] {
        god(
            rig,
            &format!("@account/open org:Basin/{}=tacsim.credit", scope.0),
        );
    }
    god(rig, "@account/system hardware.fixture=tacsim.credit");
    let supplier = account(rig, &PrincipalId::Organization(basin), Some(stock_scope));
    let integrator = account(rig, &PrincipalId::Organization(basin), Some(factory_scope));
    for line in [
        format!("@facility/open {}=2 core_integration core_refit", factory.0),
        format!("@facility/owner {}=org:Basin", factory.0),
        format!(
            "@asset/title #{}=org:Basin scope={}",
            factory.0, factory_scope.0
        ),
    ] {
        god(rig, &line);
    }
    for actor in [GOD, agent] {
        for cap in ["industry.core_die.issue", "vault.deposit", "vault.withdraw"] {
            god(
                rig,
                &format!(
                    "@org/grant #{}=Basin {cap} scope=#{}",
                    actor.0, stock_scope.0
                ),
            );
        }
        for cap in ["industry.core.integrate", "industry.core.refit"] {
            god(
                rig,
                &format!(
                    "@org/grant #{}=Basin {cap} scope=#{}",
                    actor.0, factory_scope.0
                ),
            );
        }
    }
    let ungovern = back_room(rig, agent);
    HardwareCoordinates {
        ungovern: Some(ungovern),
        bay: None,
        importer: basin,
        stock_scope,
        depot: vault(rig, "HardwareStock0"),
        agents: [
            mush_industry::core_production::OriginAgent {
                actor: GOD,
                stock: vault(rig, "HardwareStock1"),
            },
            mush_industry::core_production::OriginAgent {
                actor: agent,
                stock: vault(rig, "HardwareStock2"),
            },
        ],
        grades: [
            mush_inventory::Grade::new(10).expect("a grade"),
            mush_inventory::Grade::new(20).expect("a grade"),
            mush_inventory::Grade::new(30).expect("a grade"),
        ],
        supplier_account: supplier,
        integrator: mush_economy::core_integration::ScopedCoreAccount {
            account: integrator,
            organization: basin,
            scope: factory_scope,
        },
        factory,
        output_container: output,
    }
}

/// **The named launch's identities** (the launch story's `setup_with(2)`, ported): the counter
/// room, the Republic, the Registry's and the venue's system accounts, the three named Houses
/// (each an organisation with a `publication` scope and treasury, GOD granted to publish,
/// transfer titles and post, the treasury minted one filing fee and one more for the M8 stories),
/// and the player market's Registry (`Registry` org, `review` scope, one operator) and clerk.
fn deployment(rig: &mut Rig, hardware: HardwareCoordinates) -> (LaunchDeployment, ObjId) {
    let room = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Room, "Named launch counter", GOD, None)
        .expect("the counter");
    for line in [
        "@faction/create Republic",
        "@account/system launch.registry=tacsim.credit",
        "@account/system launch.venue=tacsim.credit",
    ] {
        god(rig, line);
    }
    let faucet = account(rig, &system("hardware.fixture"), None);
    let mut houses = Vec::new();
    for (name, source) in [
        ("Meridiani", "Meridiani Field Series"),
        ("Coprates", "Coprates Drill"),
        ("Tycho", "Safe Conduct"),
    ] {
        god(rig, &format!("@org/create {name}"));
        god(rig, &format!("@org/scope create {name}=publication"));
        let organization = org(rig, name);
        let publication = scope(rig, organization, "publication");
        god(
            rig,
            &format!("@account/open org:{name}/{}=tacsim.credit", publication.0),
        );
        for cap in [
            mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
            "asset.title.transfer",
            "economy.account.post",
        ] {
            god(
                rig,
                &format!(
                    "@org/grant #{}={name} {cap} scope=#{}",
                    GOD.0, publication.0
                ),
            );
        }
        let treasury = account(
            rig,
            &PrincipalId::Organization(organization),
            Some(publication),
        );
        // One launch filing fee (100cr), as the launch story mints.
        god(
            rig,
            &format!("@econ/mint {}={} 10000", faucet.0, treasury.0),
        );
        houses.push(HouseDeployment {
            organization,
            scope: publication,
            treasury,
            publisher: GOD,
            source_name: source.into(),
            // Doc 17h §3 M8h: no House attests its filings unasked (the procuring House's
            // policy agent does, in the zero-PC world); the stories attest by hand.
            attestation_agent: None,
        });
    }
    let [meridiani, coprates, tycho]: [HouseDeployment; 3] =
        houses.try_into().expect("three named houses");
    for line in ["@org/create Registry", "@org/scope create Registry=review"] {
        god(rig, line);
    }
    let registry = org(rig, "Registry");
    let review = scope(rig, registry, "review");
    let operator = rig.tw.spawn_player("Registry operator");
    let clerk = rig.tw.spawn_player("Market clerk");
    let recipient = |rig: &Rig, name: &str| mush_economy::managed_entitlement::Recipient {
        owner: system(name),
        account: account(rig, &system(name), None),
    };
    let republic = rig
        .tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .expect("the Faction owner")
        .faction_by_name("Republic")
        .expect("the Republic")
        .id;
    let deployment = LaunchDeployment {
        format: 2,
        mode: LaunchMode::Initialize,
        registry_issuer: SystemPrincipalId::new("hardware.factory").expect("the issuer"),
        registry: recipient(rig, "launch.registry"),
        venue: LaunchVenue {
            name: mush_market::MarketId::new("launch.counter").expect("the venue"),
            room,
            recipient: recipient(rig, "launch.venue"),
        },
        houses: NamedHouses {
            meridiani,
            coprates,
            tycho,
        },
        republic,
        hardware,
        market: Some(PlayerMarket {
            clerk,
            registry_authority: RegistryAuthority {
                organization: registry,
                scope: review,
                operators: vec![operator],
            },
            terms: MarketTerms::PROVISIONAL,
        }),
        procurement: Vec::new(),
    };
    (deployment, operator)
}

/// **The launch prepared from `deployment` for each boot** (`excursions::LaunchFactory`): the
/// production composition, `LaunchComposition::prepare` and nothing after it, which is what
/// `mush_server::boot` calls (doc 44 MW.0). An Active launch composes the IC mind claims in the
/// rigs' bays itself, so every M8 world is a claims world.
pub(crate) fn factory(deployment: &LaunchDeployment) -> crate::excursions::LaunchFactory {
    let deployment = deployment.clone();
    Arc::new(move || {
        LaunchComposition::prepare(&launch_layer(), deployment.clone())
            .expect("the launch composition prepares")
    })
}

/// The registered action `owner:name` (the hardware flow driver's `action`).
pub(crate) fn action(rig: &Rig, owner: &str, name: &str) -> DefinitionRef {
    rig.tw
        .engine()
        .registry
        .definitions()
        .expect("the definitions")
        .iter()
        .map(|d| d.reference())
        .find(|r| {
            r.key().owner.as_str() == owner
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == name
        })
        .cloned()
        .unwrap_or_else(|| panic!("the registered action {owner}:{name}"))
}

impl M8World {
    /// **Land `owner:name` as `actor`** through its current signed offer on the composed path
    /// (the hardware flow driver's `perform`): the act's commit token and its outcome.
    ///
    /// # Panics
    ///
    /// The act does not land.
    pub(crate) fn perform(
        &mut self,
        owner: &str,
        name: &str,
        actor: ObjId,
        parameters: Value,
    ) -> (mush_core::CommitToken, Value) {
        self.try_perform(owner, name, actor, parameters)
            .unwrap_or_else(|error| panic!("{owner}:{name} by #{}: {error}", actor.0))
    }

    /// [`M8World::perform`], its refusal returned.
    pub(crate) fn try_perform(
        &mut self,
        owner: &str,
        name: &str,
        actor: ObjId,
        parameters: Value,
    ) -> Result<(mush_core::CommitToken, Value), String> {
        self.serial += 1;
        let submit = AuthoritativeSubmit {
            actor,
            action: action(&self.rig, owner, name),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters,
            key: mush_core::IdempotencyKeyText::parse_canonical(format!(
                "m8g.{owner}.{name}.{}",
                self.serial
            ))
            .expect("a key"),
        };
        land_commit(&mut self.rig, &submit)
    }

    /// **The typed rows of `family`** (the hardware rig's `rows`): every kept post-image's value.
    pub(crate) fn rows(&self, family: &str) -> Vec<Value> {
        self.rig
            .typed
            .values()
            .filter(|v| v["family"]["id"].as_str() == Some(family))
            .filter_map(|v| v.get("value").cloned())
            .collect()
    }

    /// **One pump of the world's services** (the launch story's `pulse`): the scheduler's next
    /// due fired, then the one observed authoritative submission a service staged (the author
    /// service's save, the Registry's rating steps) landed on the composed path.
    pub(crate) fn pulse(&mut self) {
        if let Some(when) = self.rig.tw.engine_mut().sched.next_due() {
            // Keep the world clock with the scheduler's (`fire_due` takes milliseconds): the
            // surface tick reads the world's own time, so a pump that fired ahead of it would
            // leave every later surface act's due behind the tick's last fire.
            let now = (when / 1_000).max(self.rig.tw.world().now());
            self.rig.tw.set_now(now);
            self.rig.tw.fire_due(when);
        }
        if let Some(submitted) = self
            .rig
            .tw
            .engine_mut()
            .take_observed_authoritative_submit()
        {
            let (request, _) = submitted.into_parts();
            crate::excursions::land_service(&mut self.rig, &request)
                .expect("a service's submission lands");
        }
    }
}

/// The submission runtime the launch story installs: every observed submission is pending until
/// the harness lands it (an in-process adapter; it grants no authority).
struct Pending;
impl mush_core::submission_observation::SubmissionDispositionReader for Pending {
    fn no_intent(&self) -> bool {
        false
    }
}
struct Runtime;
impl mush_core::submission_observation::SubmissionRuntime for Runtime {
    fn observe(
        &self,
        _: &AuthoritativeSubmit,
    ) -> mush_core::submission_observation::SubmissionRuntimeState {
        mush_core::submission_observation::SubmissionRuntimeState::new(
            Arc::new(Pending),
            Box::new(()),
        )
    }
}

/// **The combined M8 world, booted Active, unrated** (G1's composition half; fast): the port
/// world with the pilot's Dray-P on Ahuna Flank's pad and one leased rig in its bay, the
/// launch's identities, the launch booted `Initialize` with its four setup actions landed
/// (`initialize_commerce` and the three hardware owners'), the runbook's Registry grants to the
/// operator (recall, revoke, inspect, fine), and the launch rebooted `Active`.
pub(crate) fn m8_world() -> M8World {
    m8_world_with(None)
}

/// **The procuring House's identities** (doc 17h §3 M8f; the zero-PC story): its procurement
/// agent, a Player-kind character the operator creates with a login, a member of the House's
/// backing faction (the Republic), standing at the venue and granted `roll.holder.org` on the
/// House's ROOT scope with the monthly budget as its limit; and the House's root treasury
/// funded with exactly one month's budget. Returns the policy the deployment names.
fn procuring_house(
    rig: &mut Rig,
    deployment: &LaunchDeployment,
    name: &str,
    budget_minor: i64,
) -> tacsim::launch::ProcurementPolicy {
    let org = org(rig, name);
    let root = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the Org owner")
        .state()
        .org(org)
        .expect("the House")
        .root_scope;
    let agent = rig.tw.spawn_player(&format!("{name} procurement agent"));
    crate::excursions::with_account(rig, agent, &format!("{name}Agent"), 9_010);
    let agent_name = rig.tw.world().get(agent).expect("the agent").name.clone();
    god(rig, &format!("@faction/add Republic={agent_name}/level:1"));
    rig.tw
        .world_mut()
        .move_to(agent, deployment.venue.room)
        .expect("the agent at the counter");
    god(
        rig,
        &format!(
            "@org/grant #{}={name} roll.holder.org limit={budget_minor}",
            agent.0
        ),
    );
    god(
        rig,
        &format!("@account/open org:{name}/{}=tacsim.credit", root.0),
    );
    let treasury = account(rig, &PrincipalId::Organization(org), Some(root));
    let faucet = account(rig, &system("hardware.fixture"), None);
    god(
        rig,
        &format!("@econ/mint {}={} {budget_minor}", faucet.0, treasury.0),
    );
    tacsim::launch::ProcurementPolicy {
        org,
        scope: root,
        agent,
        budget_minor_per_ic_month: Some(budget_minor),
        standing: Some(deployment.republic),
        cap_permille: None,
        roles: tacsim::launch::ProcurementRole::all(),
    }
}

/// [`m8_world`], with `procurement` (a House, its monthly budget and its hull count) naming
/// that House's procurement policy in the deployment ([`procuring_house`]) and titling it that
/// many chartered hulls, before the launch boots.
pub(crate) fn m8_world_with(procurement: Option<(&str, i64, usize)>) -> M8World {
    m8_world_in(procurement, ("ahuna", "ahuna_port"), 1)
}

/// **The M8 world on `site`, with `n_rigs` rigs** (doc 23 cut S.4b-3): [`m8_world`] on any site
/// (`(venue, entry)`). The field runs at the stock tempo: a tuned layer would also run the
/// Registry's rating arenas at it, and a reboot to retune after the purchase loses it.
pub(crate) fn m8_world_field(site: (&str, &str), n_rigs: usize) -> M8World {
    m8_world_in(None, site, n_rigs)
}

fn m8_world_in(
    procurement: Option<(&str, i64, usize)>,
    site: (&str, &str),
    n_rigs: usize,
) -> M8World {
    let mut rig = stocked();
    let (pilot, row, tvs) =
        crate::excursions::on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), site, n_rigs);
    crate::excursions::enable_account_captures(&rig);
    // GOD publishes the named Houses' sources and authors them, a player with an account.
    crate::excursions::with_account(&mut rig, GOD, "Publisher", 9_001);
    let hulls = procurement.map_or_else(Vec::new, |(_, _, n)| charter_hulls(&mut rig, n));
    let coordinates = hardware(&mut rig);
    let (mut deployment, operator) = deployment(&mut rig, coordinates);
    if let Some((house, budget, _)) = procurement {
        title_hulls_to(&mut rig, house, &hulls);
        let policy = procuring_house(&mut rig, &deployment, house, budget);
        deployment.procurement = vec![policy];
    }
    // The named launch's `logins`: the clerk and the Registry operator share one staff login.
    let market = deployment.market.clone().expect("format 2");
    for actor in std::iter::once(market.clerk).chain(market.registry_authority.operators) {
        crate::excursions::with_account(&mut rig, actor, "LaunchStaff", 9_003);
    }
    rig.launch = Some(factory(&deployment));
    let rig = reboot_with(rig, &[pilot]);
    // Doc 44 MW.0: the claim path is the Active launch's alone. The `Initialize` boot, the
    // runbook's first, composes none of it (every claim owner is Active-only).
    assert!(
        !tacsim::ic_claim_path_composed(rig.tw.world()),
        "an Initialize launch composes no claim path"
    );
    let mut world = M8World {
        rig,
        deployment: deployment.clone(),
        pilot,
        row,
        tvs,
        serial: 0,
    };
    world.perform("roll", "initialize_commerce", GOD, json!({}));
    for name in [
        "initialize_core_origin",
        "initialize_core_production",
        "initialize_core_refit",
    ] {
        world.perform("industry", name, GOD, json!({}));
    }
    let authority = deployment
        .market
        .as_ref()
        .expect("format 2")
        .registry_authority
        .clone();
    for capability in [
        "roll.registry.recall",
        "roll.registry.revoke",
        "roll.registry.inspect",
        "roll.registry.fine",
        // Doc 17h cut M8d2: the seizure and the forfeit.
        "roll.registry.seize",
        "roll.registry.forfeit",
    ] {
        god(
            &mut world.rig,
            &format!(
                "@org/grant #{}=Registry {capability} scope=#{}",
                operator.0, authority.scope.0
            ),
        );
    }
    deployment.mode = LaunchMode::Active;
    world.deployment = deployment.clone();
    world.rig.launch = Some(factory(&deployment));
    let M8World {
        rig,
        deployment,
        pilot,
        row,
        tvs,
        serial,
    } = world;
    let mut rig = reboot_with(rig, &[pilot]);
    rig.tw
        .engine_mut()
        .set_submission_runtime(Arc::new(Runtime))
        .expect("the submission runtime");
    // The launch story's `connected`: GOD, the configured publisher, has a session.
    rig.tw.engine_mut().step(
        mush_core::Inbound::ConnUp {
            conn: 0,
            actor: GOD,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut mush_core::Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    M8World {
        rig,
        deployment,
        pilot,
        row,
        tvs,
        serial,
    }
}

/// **One named House's source, genuinely certified** (the launch story's `certify`, ported): the
/// import (`@mind/launch <selector>=<name>`), the real save through the author service, then
/// `@registry/rate` and the Registry service's trials until the rating's receipt names the saved
/// revision. Returns the revision and the rating.
pub(crate) async fn certify(
    world: &mut M8World,
    selector: &str,
    name: &str,
) -> (mush_roll::Revision, mush_roll::commerce::rating::RatingRef) {
    god(&mut world.rig, &format!("@mind/launch {selector}={name}"));
    certify_saved(world, name).await
}

/// **Save GOD's draft `name` and certify it** (`certify` after the import): the real save
/// through the author service, then `@registry/rate` and the Registry service's trials.
pub(crate) async fn certify_saved(
    world: &mut M8World,
    name: &str,
) -> (mush_roll::Revision, mush_roll::commerce::rating::RatingRef) {
    god(
        &mut world.rig,
        &format!("@mind/save {name}=the M8 world's save"),
    );
    let saved = |world: &M8World| -> Option<mush_roll::Revision> {
        let store = world
            .rig
            .tw
            .world()
            .module_state::<mush_roll::RollStore>()?;
        let mind = store.mind(GOD, name).ok()?;
        mind.latest()
            .and_then(|reference| store.revision(GOD, reference).ok())
            .cloned()
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let revision = loop {
        world.pulse();
        if let Some(revision) = saved(world) {
            break revision;
        }
        assert!(
            Instant::now() < deadline,
            "the author service did not save {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let reference = revision.reference();
    // The previous House's job is released a pulse after its receipt (the launch story's wait
    // on `registry_service::holds`): pump until no job of GOD's is still live, then rate once.
    let deadline = Instant::now() + Duration::from_secs(60);
    while world.rows("rating_jobs").iter().any(|job| {
        job.get("state")
            .and_then(|state| state.get("kind").or(Some(state)))
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "admitted" | "running" | "queued"))
    }) {
        assert!(
            Instant::now() < deadline,
            "the previous Registry job was never released"
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    // A job just finished may still be held for a pulse: retry only that refusal.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let out = run(
            &mut world.rig.tw,
            GOD,
            &format!(
                "@registry/rate {}/{}",
                reference.key.mind, reference.key.number
            ),
        );
        eprintln!("M8G_STORY rate {name} -> {out}");
        if !out.contains("unavailable") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the Registry never took {name}: {out}"
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let identity =
        mush_roll::commerce::RevisionIdentity::capture(&revision).expect("the revision identity");
    let deadline = Instant::now() + Duration::from_secs(7200);
    loop {
        world.pulse();
        for wire in world.rows("rating_receipts") {
            let receipt: mush_roll::commerce::rating::RatingReceipt =
                serde_json::from_value(wire).expect("a rating receipt");
            if receipt.subject().revision() == identity {
                return (revision, receipt.reference().expect("the rating"));
            }
        }
        assert!(
            Instant::now() < deadline,
            "the Registry did not certify {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// **One certified House line, filed and listed** (the launch story's `other_named_house`,
/// ported): the House filing with the certificate from the House treasury (the owner licence
/// held by the House's organisation), then the counter listing. Returns the owner licence and
/// the listing.
pub(crate) fn file_and_list(
    world: &mut M8World,
    house: &HouseDeployment,
    (revision, rating): &(mush_roll::Revision, mush_roll::commerce::rating::RatingRef),
) -> (mush_roll::commerce::LicenceId, mush_core::CommitToken) {
    let (shell, _) = world.perform(
        "roll",
        mush_core::reserved_shell::RESERVE_ACTION,
        GOD,
        json!({}),
    );
    let reference = revision.reference();
    let (filing, _) = world.perform(
        "roll",
        mush_economy::registry_filing::HOUSE_FILING_ACTION,
        GOD,
        json!({
            "shell_request": mush_core::RequestUid::from_bytes(*shell.as_bytes()),
            "mind": reference.key.mind, "revision": reference.key.number,
            "revision_digest": reference.digest, "rating_job": rating.job(),
            "rating_id": rating.id(), "rating_digest": rating.receipt_digest(),
            "payer": house.treasury, "charter_name": house.source_name,
            "organization": house.organization, "scope": house.scope
        }),
    );
    let birth = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .find(|row| row.get("commit") == Some(&json!(filing)))
        .expect("the House filing");
    let owner_licence: mush_roll::commerce::LicenceId =
        serde_json::from_value(birth.get("licence").expect("its owner licence").clone())
            .expect("an owner licence");
    let room = world.deployment.venue.room;
    world
        .rig
        .tw
        .world_mut()
        .move_to(GOD, room)
        .expect("GOD at the counter");
    let (listing, _) = world.perform(
        "market",
        "list_house_mind",
        GOD,
        json!({"owner_licence": owner_licence}),
    );
    (owner_licence, listing)
}

/// **The world's rated half** (G1): the three named Houses' sources certified, filed and listed
/// at the counter, in the launch story's order (Coprates, Meridiani, Safe Conduct). Returns each
/// House's owner licence and listing, Coprates first.
pub(crate) async fn rate_and_list(
    world: &mut M8World,
) -> Vec<(
    HouseDeployment,
    mush_roll::commerce::LicenceId,
    mush_core::CommitToken,
)> {
    let houses = world.deployment.houses.clone();
    let mut listed = Vec::new();
    for (selector, house) in [
        ("coprates", houses.coprates),
        ("meridiani", houses.meridiani),
        ("safe_conduct", houses.tycho),
    ] {
        let certified = certify(world, selector, &house.source_name.clone()).await;
        eprintln!("M8G_STORY rated {selector}: {}", census(world, "rating"));
        let (licence, listing) = file_and_list(world, &house, &certified);
        eprintln!(
            "M8G_STORY listed {selector}: ic now {}",
            world.rig.tw.world().world_ic_now().at
        );
        listed.push((house, licence, listing));
    }
    listed
}

/// **Coprates' line alone, rated and listed** (doc 23 cut S.4b-2: one licence is all a claim
/// story needs; [`rate_and_list`]'s first row): one genuine rating where the tier-2 world runs
/// three. Returns the House, the owner licence and the listing.
pub(crate) async fn rate_and_list_coprates(
    world: &mut M8World,
) -> (
    HouseDeployment,
    mush_roll::commerce::LicenceId,
    mush_core::CommitToken,
) {
    let house = world.deployment.houses.coprates.clone();
    let certified = certify(world, "coprates", &house.source_name.clone()).await;
    eprintln!("M8G_STORY rated coprates: {}", census(world, "rating"));
    let (licence, listing) = file_and_list(world, &house, &certified);
    (house, licence, listing)
}

/// **The pilot buys one paid House Pin** of `listing` (the launch story's purchase): a member of
/// the Republic (Coprates requires it), at the counter, quote, fund and settle. Returns the paid
/// licence.
pub(crate) fn buy(
    world: &mut M8World,
    listing: mush_core::CommitToken,
) -> mush_roll::commerce::LicenceId {
    let pilot = world.pilot;
    let name = world
        .rig
        .tw
        .world()
        .get(pilot)
        .expect("the pilot")
        .name
        .clone();
    let room = world.deployment.venue.room;
    let back = world.rig.tw.world().get(pilot).and_then(|o| o.location);
    // `@faction/add` names the pilot as GOD sees it: both at the counter, as the launch story's
    // buyer and publisher stand.
    for who in [GOD, pilot] {
        world
            .rig
            .tw
            .world_mut()
            .move_to(who, room)
            .expect("at the counter");
    }
    // A second purchase finds the pilot a member already: the membership is the standing
    // asked for, so "Already a member" is the same answer.
    let out = run(
        &mut world.rig.tw,
        GOD,
        &format!("@faction/add Republic={name}/level:1"),
    );
    assert!(
        !out.contains("No such") && !out.contains("not found"),
        "the pilot a Republic member: {out}"
    );
    world
        .rig
        .tw
        .world_mut()
        .move_to(pilot, room)
        .expect("the pilot at the counter");
    let (quote, _) = world.perform(
        "market",
        "quote_house_mind",
        pilot,
        json!({"listing": listing}),
    );
    let (fund, _) = world.perform(
        "market",
        "purchase_house_mind",
        pilot,
        json!({"quote": quote}),
    );
    world.perform(
        "market",
        "settle_house_mind",
        pilot,
        json!({"operation": fund}),
    );
    if let Some(back) = back {
        world
            .rig
            .tw
            .world_mut()
            .move_to(pilot, back)
            .expect("the pilot back aboard");
    }
    let paid: mush_roll::commerce::paid::PersonalPaidLicence = serde_json::from_value(
        world
            .rows("roll_mind_personal_paid_licence")
            .into_iter()
            .last()
            .expect("the paid licence"),
    )
    .expect("a paid licence");
    paid.id()
}

/// **G1's composition half boots** (fast, not ignored): the combined world composes and boots
/// Active — the launch's market verbs answer, the Roll is ready, the pilot's rig is in the bay —
/// and every write of the boot and the setup landed (no namespace write refused).
#[test]
fn m8g_the_combined_world_boots_active() {
    let mut world = m8_world();
    crate::excursions::no_refused_writes(&world.rig.tw);
    // The launch composed: its House listing and the Registry's inspection are registered
    // actions, and the House import verb answers the configured publisher.
    let _ = action(&world.rig, "market", "list_house_mind");
    let _ = action(&world.rig, "tacsim", "registry_mind_inspection");
    assert!(crate::excursions::in_bay(
        &world.rig.tw,
        world.tvs[0],
        &world.row
    ));
    let out = run(&mut world.rig.tw, GOD, "@mind/launch");
    assert!(out.contains("Named launch"), "{out}");
    // Doc 44 MW.0: the production composition composes the IC mind claims once it is Active;
    // no rig asks for them. (The world's own `Initialize` boot asserted it composes none.)
    assert!(
        tacsim::ic_claim_path_composed(world.rig.tw.world()),
        "an Active launch composes the claim path"
    );
    let _ = (world.deployment.houses.coprates.treasury, world.serial);
}

/// The organisation `name`'s root scope.
fn root_scope(rig: &Rig, name: &str) -> OrgScopeId {
    let org = org(rig, name);
    rig.tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the Org owner")
        .state()
        .org(org)
        .expect("the organisation")
        .root_scope
}

/// **Title `tv` to the House `house`** on its root scope, by staff: the title's claims released
/// first (a leased rig's lease claim among them), as [`title_hulls_to`] does for a hull. The rig
/// and its lessee's record stay where they are; only the title moves.
pub(crate) fn title_rig_to_house(world: &mut M8World, tv: ObjId, house: &str) {
    let root = root_scope(&world.rig, house);
    let claims = economy(&world.rig.tw)
        .snapshot()
        .asset(tv)
        .map_or(0, |a| a.encumbrances.len());
    for k in (1..=claims).rev() {
        god(&mut world.rig, &format!("@asset/release #{}={k}", tv.0));
    }
    god(
        &mut world.rig,
        &format!("@asset/transfer #{}=org:{house} scope={}", tv.0, root.0),
    );
    assert_eq!(
        title_of(world, tv).title_owner,
        PrincipalId::Organization(org(&world.rig, house)),
        "the rig is the House's"
    );
}

/// **Doc 23 cut S.4c (D-23-24; doc 44 MW.22): the hull rule over the world's real records**
/// (fast, not ignored). The rule the Roll's reader captures and the dispatch refuses by
/// (`mush_roll::commerce::ic_claims::hull_authority`), asked of the production composition's
/// own rig as its title changes hands:
/// 1. the pilot's rig as `+tv/lease go` left it, titled to Occator under its lease: the
///    pilot, its lessee, is admitted by the lease; a stranger is refused, the refusal naming
///    Occator;
/// 2. staff release the title's claims and title the rig to the Meridiani House: the lease's
///    contract is still Active but the title no longer carries its claim, and the pilot holds
///    no office, so the pilot is refused, the refusal naming the House;
/// 3. the pilot is granted `roll.holder.org` on the House's root scope, the grant its licence
///    use asks: admitted by the office, on the title's scope;
/// 4. staff title the rig to the pilot: admitted by the title, and nobody else is.
#[test]
fn mw22_the_hull_rule_reads_a_real_lease_a_house_title_and_a_personal_title() {
    use mush_roll::commerce::ic_claims::{HullAuthority, HullRefusal, hull_authority};
    let mut world = m8_world();
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    let stranger = world.rig.tw.spawn_player("Stranger");
    let ask = |world: &M8World, who: ObjId| {
        hull_authority(world.rig.tw.world(), tv, who).map(|admitted| admitted.authority)
    };
    // 1. The lease.
    let occator = title_of(&world, tv).title_owner;
    assert!(
        matches!(occator, PrincipalId::Organization(_)),
        "a leased rig is titled to its lessor's organisation: {occator:?}"
    );
    assert!(
        matches!(ask(&world, pilot), Ok(HullAuthority::Lease { ref lessor, .. }) if *lessor == occator),
        "the lessee flies the leased rig: {:?}",
        ask(&world, pilot)
    );
    assert_eq!(
        ask(&world, stranger),
        Err(HullRefusal::Anothers {
            owner: occator.clone()
        })
    );
    // 2. The House's hull, and no office.
    title_rig_to_house(&mut world, tv, "Meridiani");
    let house = org(&world.rig, "Meridiani");
    let root = root_scope(&world.rig, "Meridiani");
    let refused = Err(HullRefusal::Anothers {
        owner: PrincipalId::Organization(house),
    });
    assert_eq!(ask(&world, pilot), refused);
    // 3. The office.
    god(
        &mut world.rig,
        &format!("@org/grant #{}=Meridiani roll.holder.org", pilot.0),
    );
    assert_eq!(
        ask(&world, pilot),
        Ok(HullAuthority::Office {
            org: house,
            scope: Some(root),
        })
    );
    assert_eq!(ask(&world, stranger), refused);
    // 4. The pilot's own title.
    god(
        &mut world.rig,
        &format!("@asset/transfer #{}=#{}", tv.0, pilot.0),
    );
    assert_eq!(ask(&world, pilot), Ok(HullAuthority::Title));
    assert_eq!(
        ask(&world, stranger),
        Err(HullRefusal::Anothers {
            owner: PrincipalId::Object(pilot)
        })
    );
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **The procuring world boots** (fast, not ignored): Meridiani's procurement policy, its agent
/// granted and its root treasury funded, composes into the launch and boots Active.
#[test]
fn m8g_the_procuring_world_boots_active() {
    let world = m8_world_with(Some(("Meridiani", 160_000, 5)));
    assert_eq!(world.deployment.procurement.len(), 1);
    // The hulls the procurement cap counts (`procurement::tick::titled_hulls`): the House's
    // live titles that are hulls of a programme port's map.
    let org = world.deployment.houses.meridiani.organization;
    let mapped: std::collections::BTreeSet<ObjId> =
        tacsim::port::coop::programme_ports(world.rig.tw.world())
            .iter()
            .flat_map(|port| port.map.hulls.iter().map(|row| ObjId(row.hull)))
            .collect();
    let titled: Vec<ObjId> = economy(&world.rig.tw)
        .snapshot()
        .assets_of(&PrincipalId::Organization(org))
        .filter(|asset| asset.lifecycle == mush_economy::AssetLifecycle::Active)
        .map(|asset| asset.object)
        .collect();
    let counted = titled.iter().filter(|hull| mapped.contains(hull)).count();
    assert_eq!(
        counted, 5,
        "the cap's hulls: titled {titled:?}, mapped {mapped:?}"
    );
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **G1's rated half, the story** (dispatch-only, doc 17h §3 M8g G5): the combined world with
/// the three named Houses' sources genuinely certified, House-filed (each owner licence held by
/// its House's organisation) and listed at the counter, and the pilot holding one paid House
/// Pin of Coprates Drill — the filed, rated, licensed mind every M8 story stands on.
#[tokio::test(flavor = "current_thread")]
#[ignore = "M8 combined-world story: three genuine Registry ratings; the CI dispatch runs it"]
async fn m8g_story_the_combined_world_rates_files_lists_and_sells() {
    let mut world = m8_world();
    let listed = rate_and_list(&mut world).await;
    assert_eq!(listed.len(), 3, "three named House lines listed");
    eprintln!("M8G_STORY phase=rated event=complete");
    let coprates = listed[0].2;
    let paid = buy(&mut world, coprates);
    assert!(
        world
            .rows("roll_mind_personal_paid_licence")
            .iter()
            .any(|row| row.get("id") == Some(&json!(paid))),
        "the pilot's paid Pin"
    );
    eprintln!("M8G_STORY phase=bought event=complete");
    crate::excursions::no_refused_writes(&world.rig.tw);
}

impl M8World {
    /// **Reboot the combined world** from its store, the pilot and GOD reconnected (the launch
    /// rebooted as it was last prepared).
    pub(crate) fn reboot(self) -> M8World {
        let M8World {
            rig,
            deployment,
            pilot,
            row,
            tvs,
            serial,
        } = self;
        let mut rig = reboot_with(rig, &[pilot, GOD]);
        rig.tw
            .engine_mut()
            .set_submission_runtime(Arc::new(Runtime))
            .expect("the submission runtime");
        M8World {
            rig,
            deployment,
            pilot,
            row,
            tvs,
            serial,
        }
    }
}

/// The seal facts of `tv`'s open epoch.
pub(crate) fn open_seal(world: &mut M8World, tv: ObjId) -> tacsim::surface::black_box::SealFacts {
    crate::excursions::record_of(&mut world.rig.tw, tv)
        .and_then(|record| record.sortie)
        .and_then(|open| open.seal)
        .expect("the rig's open epoch is sealed")
}

/// `licence`'s seal-mark ring, if any: its marks, oldest first.
pub(crate) fn marks(world: &M8World, licence: mush_roll::commerce::LicenceId) -> Vec<Value> {
    world
        .rows(mush_roll::commerce::seal_marks::FAMILY)
        .into_iter()
        .find(|ring| ring.get("licence") == Some(&json!(licence)))
        .and_then(|ring| ring.get("marks").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// A one-line census of the typed families matching `prefix` (row counts, and how many rows
/// mention a won trial), for a story's log.
fn census(world: &M8World, prefix: &str) -> String {
    let mut counts = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for v in world.rig.typed.values() {
        let Some(family) = v["family"]["id"].as_str() else {
            continue;
        };
        if family.starts_with(prefix) {
            let entry = counts.entry(family.to_owned()).or_default();
            entry.0 += 1;
            entry.1 += v.to_string().matches("\"win\"").count();
        }
    }
    format!("{counts:?}")
}

/// Launch rig 1 with `line`, finish it, and return what the operator heard at the finish.
///
/// Doc 44 MW.0: the launch composes the claim path, so a filed mind that will fly opens its
/// claim at `go` and the launch is queued behind it (doc 23 D-23-21): the surface tick's claim
/// arm opens the claim in the bay and starts the launch, which is pumped here. Any other launch
/// (no mind, a draft, a licence fallen to the ROM) starts its act at `go`, as before.
pub(crate) fn launch(world: &mut M8World, line: &str) -> String {
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    let _ = crate::excursions::act_go(&mut world.rig, pilot, line);
    for _ in 0..12 {
        if crate::excursions::record_of(&mut world.rig.tw, tv).is_some_and(|r| r.act.is_some()) {
            break;
        }
        fires(world, 1);
    }
    let act = crate::excursions::record_of(&mut world.rig.tw, tv).and_then(|record| record.act);
    eprintln!(
        "M8G_STORY launch {line}: act due {:?}, ic now {}",
        act.map(|act| act.due_ic),
        world.rig.tw.world().world_ic_now().at
    );
    crate::excursions::finish(&mut world.rig, tv, pilot)
}

/// Recover rig 1 into the bay; where it flew under a claim (doc 44 MW.0), the claim arm lands
/// the Close the recover owes, so the rig's next launch is not held by a claim still closing.
pub(crate) fn recover(world: &mut M8World) {
    use tacsim::surface::excursion::ClaimCursor;
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    let _ = crate::excursions::act_go(&mut world.rig, pilot, "+tv/1 recover go");
    let _ = crate::excursions::finish(&mut world.rig, tv, pilot);
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));
    let owed = crate::excursions::record_of(&mut world.rig.tw, tv)
        .and_then(|record| record.claim)
        .and_then(|claim| match *claim {
            ClaimCursor::Closing { open, .. } => Some(open),
            _ => None,
        });
    if let Some(open) = owed {
        for _ in 0..12 {
            if tacsim::ic_claim_closed(world.rig.tw.world(), open).is_some() {
                break;
            }
            fires(world, 1);
        }
        assert!(
            tacsim::ic_claim_closed(world.rig.tw.world(), open).is_some(),
            "the claim's Close landed"
        );
    }
}

/// **Doc 17h cut M8g G2 — M8e's owed tier-2 stories, over the combined world** (doc 23 §10 tier
/// 2 as restated by Q-M8-1; the M8e ledger's six owed world tests). **Since doc 44 MW.0 the
/// world composes the claim path**, so a filed mind that flies opens a claim (the seal on the
/// open line is the claim's own), and since MW.22 (doc 23 cut S.4c) the pilot's leased rig is
/// admitted by its lease. The pilot's leased rig,
/// staff-bound to the governed Class II rig root (the named Houses' lines rate Class II, and a
/// governed core refuses a mind over its class, D-UM-17), flies:
/// 1. its own paid House Pin (`mind=filed:<paid>`): the open epoch seals the filed mind with
///    the licence's word and `active`, the pilot the holder; the licence's seal-mark ring takes
///    one governed mark for the flight; the line survives a restart;
/// 2. the Coprates House's OWNER licence, held by its organisation, flown by the pilot granted
///    `roll.holder.org` on the House's root and publication scopes (`org=Coprates`): sealed
///    through the owner rows, the holder the organisation with the pilot acting, the warrant
///    principal the pilot;
/// 3. the paid licence resold to another player: the seller's launch is refused as a licence
///    not held, the rig left in the bay (a pending transfer, D-UM-40's sentence naming the
///    mind, needs an open paid claim on the licence: the S.4b-2 story's step 3);
/// 4. the paid licence past its grace (the IC clock past the term and its three days): the ROM
///    is sealed, the line names the licence `lapsed`, the operator alone hears the clause in the
///    launch's end, and the ring takes no mark for the fall.
#[tokio::test(flavor = "current_thread")]
#[ignore = "M8 combined-world story: three genuine Registry ratings; the CI dispatch runs it"]
async fn m8g_story_tier2_held_minds_seal_aboard() {
    use tacsim::minds::licence_state::SealLicenceState;
    let mut world = m8_world();
    let listed = rate_and_list(&mut world).await;
    let (coprates, owner_licence, listing) = listed[0].clone();
    let paid = buy(&mut world, listing);
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    crate::ic_sortie_surface::fit(
        &mut world.rig,
        tv,
        tacsim::definitions::core_replacement::RIG_II,
    );
    eprintln!("M8G_STORY phase=tier2.setup event=complete");

    // 1. The paid Pin, sealed with its word; one governed mark; across a restart.
    let _ = launch(
        &mut world,
        &format!("+tv/1 launch mind=filed:{} go", paid.get()),
    );
    let facts = open_seal(&mut world, tv);
    assert_eq!(facts.mind.as_str(), coprates.source_name.clone());
    assert_eq!(facts.licence.as_str(), paid.get().to_string());
    assert_eq!(facts.licence_state, Some(SealLicenceState::Active));
    assert_eq!(facts.holder, Some(mush_economy::Holder::Character(pilot)));
    assert!(facts.governed);
    let ring = marks(&world, paid);
    assert_eq!(ring.len(), 1, "one mark for the flight: {ring:?}");
    assert_eq!(ring[0]["governed"], json!(true));
    assert_eq!(ring[0]["hull"], json!(tv.0));
    let opened = crate::ic_sortie_surface::lines(&world.rig, tv);
    let mut world = world.reboot();
    assert_eq!(
        crate::ic_sortie_surface::lines(&world.rig, tv),
        opened,
        "the held mind's line survives a restart"
    );
    recover(&mut world);
    eprintln!("M8G_STORY phase=tier2.paid event=complete");

    // 2. The House's owner licence, flown by a granted member: the org is the holder.
    god(
        &mut world.rig,
        &format!("@org/grant #{}=Meridiani roll.holder.org", pilot.0),
    );
    god(
        &mut world.rig,
        &format!("@org/grant #{}=Coprates roll.holder.org", pilot.0),
    );
    // The owner licence's row is projected on its charter title's scope, the House's
    // publication scope, so the acting member is granted there as well as on the root.
    god(
        &mut world.rig,
        &format!(
            "@org/grant #{}=Coprates roll.holder.org scope=#{}",
            pilot.0, coprates.scope.0
        ),
    );
    let _ = launch(
        &mut world,
        &format!(
            "+tv/1 launch mind=filed:{} org=Coprates go",
            owner_licence.get()
        ),
    );
    let facts = open_seal(&mut world, tv);
    assert_eq!(facts.licence.as_str(), owner_licence.get().to_string());
    assert_eq!(
        facts.holder,
        Some(mush_economy::Holder::Organization {
            org: coprates.organization,
            acting: pilot,
        })
    );
    let BlackBoxLine::MindSealed { principal, .. } =
        crate::ic_sortie_surface::lines(&world.rig, tv)
            .last()
            .cloned()
            .expect("the open line")
    else {
        panic!("the last line is the open line");
    };
    assert_eq!(
        principal,
        Some(tacsim::deliberation::order::CharacterRef::Character { id: pilot }),
        "the warrant principal is the member (D-23-5)"
    );
    recover(&mut world);
    eprintln!("M8G_STORY phase=tier2.org_owner event=complete");

    // 3. The paid licence resold: the seller holds it no more, and its launch is refused as a
    //    licence not held. (A pending transfer, a Funded resale head, needs an open paid claim
    //    on the licence; the rig is home and its claim closed, so the Accept transfers outright.
    //    MIND_PENDING's refusal is the S.4b-2 story's step 3.)
    let buyer = world.rig.tw.spawn_player("Resale buyer");
    crate::excursions::with_account(&mut world.rig, buyer, "ResaleBuyer", 9_002);
    let faucet = account(&world.rig, &system("hardware.fixture"), None);
    god(
        &mut world.rig,
        &format!("@account/open #{}=tacsim.credit", buyer.0),
    );
    let buyer_account = account(&world.rig, &PrincipalId::Object(buyer), None);
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} 100000", faucet.0, buyer_account.0),
    );
    let room = world.deployment.venue.room;
    let aboard = world.rig.tw.world().get(pilot).and_then(|o| o.location);
    for who in [pilot, buyer] {
        world
            .rig
            .tw
            .world_mut()
            .move_to(who, room)
            .expect("at the counter");
    }
    // Each step submits; the harness lands it as the world task would.
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        pilot,
        &format!("+minds/resell {}=Resale buyer price=1", paid.get()),
    );
    eprintln!("M8G_STORY resell -> {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    // The buyer stands at the counter as a Republic member (the venue's standing, as `buy`'s
    // purchase) and accepts: with no open paid claim the named buyer's
    // `+minds/resell/accept` is the Accept (17g C2d), which transfers the licence.
    god(&mut world.rig, "@faction/add Republic=Resale buyer/level:1");
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        buyer,
        &format!("+minds/resell/accept {} price=1", paid.get()),
    );
    eprintln!("M8G_STORY resale accept -> {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    if let Some(aboard) = aboard {
        world
            .rig
            .tw
            .world_mut()
            .move_to(pilot, aboard)
            .expect("the pilot back aboard");
    }
    let refused = crate::ic_sortie_surface::start_refusal(
        &mut world.rig,
        pilot,
        &format!("+tv/1 launch mind=filed:{} go", paid.get()),
    );
    let gone =
        tacsim::surface::dispatch::MIND_NO_LICENCE.replace("{licence}", &paid.get().to_string());
    assert!(
        refused.as_deref().is_some_and(|r| r.contains(&gone)),
        "a resold licence is not the seller's to fly: {refused:?}"
    );
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));
    eprintln!("M8G_STORY phase=tier2.resold event=complete");

    // 4. A second paid Pin (Meridiani's), its term and grace past: the ROM, told only here.
    let meridiani = listed[1].clone();
    let lapsing = buy(&mut world, meridiani.2);
    // Past the 90-day term and its 28-day Grace (owner rule, 2026-11-04; 3 days before).
    let past = world.rig.tw.world().now() + (90 + 28 + 1) * 86_400;
    world.rig.tw.world_mut().set_now(past);
    let heard = launch(
        &mut world,
        &format!("+tv/1 launch mind=filed:{} go", lapsing.get()),
    );
    let facts = open_seal(&mut world, tv);
    assert_eq!(facts.mind.as_str(), "Standard Doctrine");
    assert_eq!(facts.licence.as_str(), lapsing.get().to_string());
    assert_eq!(facts.licence_state, Some(SealLicenceState::Lapsed));
    let clause = tacsim::port::tv::LAUNCHED_DOCTRINE
        .replace("{licence}", &lapsing.get().to_string())
        .replace("{state}", "lapsed")
        .replace("{n}", "1");
    assert!(
        heard.contains(&clause),
        "the operator hears the fall: {heard}"
    );
    assert!(
        marks(&world, lapsing).is_empty(),
        "a seal that fell to the ROM marks nothing"
    );
    eprintln!("M8G_STORY phase=tier2.lapse event=complete");
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **A fourth line, rated and never listed** (the zero-PC story's unlisted line): GOD's own
/// draft from Standard Doctrine, saved and genuinely rated. A House filing admits only the
/// House's own launch source ("The rated revision cannot be filed for this organization
/// scope."), so the line stays a rated revision no listing names.
pub(crate) async fn rate_unlisted(world: &mut M8World) {
    god(&mut world.rig, "@mind/new Unlisted Survey FROM Standard");
    let _ = certify_saved(world, "Unlisted Survey").await;
}

/// **The procuring House's hulls** (the zero-PC story): `n` Dray hulls chartered at Ceres by
/// `n` pilots through `+coop/enrol`, each title issued by the economy's own `tacsim.registry`
/// issuer at the charter; [`title_hulls_to`] then moves them to the House. The charters are
/// made in the port world before the launch boots (the Co-op enrols there).
fn charter_hulls(rig: &mut Rig, n: usize) -> Vec<ObjId> {
    (0..n)
        .map(|i| {
            let (_, row) = crate::excursions::prospector(
                rig,
                &format!("Hull pilot {i}"),
                9_100 + i64::try_from(i).expect("small"),
                &format!("Survey Hull {i}"),
            );
            ObjId(row.hull)
        })
        .collect()
}

/// Each of `hulls`' titles transferred to `house`'s organisation on its root scope by staff,
/// its claims released first.
fn title_hulls_to(rig: &mut Rig, house: &str, hulls: &[ObjId]) {
    let org = org(rig, house);
    let root = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the Org owner")
        .state()
        .org(org)
        .expect("the House")
        .root_scope;
    for hull in hulls {
        let claims = economy(&rig.tw)
            .snapshot()
            .asset(*hull)
            .map_or(0, |a| a.encumbrances.len());
        for k in (1..=claims).rev() {
            god(rig, &format!("@asset/release #{}={k}", hull.0));
        }
        god(
            rig,
            &format!("@asset/transfer #{}=org:{house} scope={}", hull.0, root.0),
        );
    }
}

/// Fire the world's ticks `n` times, landing every submission (the procurement agent's buys
/// are its own submissions).
fn fires(world: &mut M8World, n: usize) {
    for _ in 0..n {
        let (_, queued) = crate::excursions::fire(&mut world.rig);
        crate::excursions::land_all(&mut world.rig, &queued);
        world.pulse();
    }
}

/// **Doc 17h cut M8g G2: M8f's zero-PC story** (doc 17h §3 M8f's gate; D-UM-54). No player
/// character acts. Meridiani's procurement agent has a budget of 1,600cr a month (four Pins),
/// its treasury one month's budget, and the House holds five titled hulls (the Ceres pool's
/// eight Drays bound the count); the three named lines are genuinely rated and listed, and a
/// fourth is rated and never listed. Two of the named lines win trials (Meridiani's and
/// Coprates' House minds); Safe Conduct scores 0 per mille. Over the month the agent's own
/// submissions buy the best line per credit up to the cap (400 per mille of five hulls: two)
/// and spend the rest on the second, within the budget; the 0 per mille line and the unlisted
/// one are untouched; every purchase is a ledger row. A restart between months changes
/// nothing, and the second month, the treasury spent, buys nothing.
#[tokio::test(flavor = "current_thread")]
#[ignore = "M8 combined-world story: four genuine Registry ratings; the CI dispatch runs it"]
async fn m8g_story_zero_pc_procurement_buys_the_best_two_under_the_cap() {
    // Four Pins at 400cr: the Ceres Co-op's pool holds eight Drays (the pilot charters one), so
    // the House is titled five and the cap (400 per mille) is two hulls of a charter.
    const BUDGET: i64 = 160_000;
    // The procurement tick says why it refuses or closes a month only through `tracing`
    // (`tacsim.procurement.refused`, `.month_closed`): print them with the story's log.
    let _procurement_log = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new(
                "tacsim::procurement=info",
            ))
            .with_writer(std::io::stderr)
            .finish(),
    );
    let mut world = m8_world_with(Some(("Meridiani", BUDGET, 5)));
    let meridiani = world.deployment.houses.meridiani.organization;
    // Procurement reads only the player market's listings (doc 17h §3 M8f, P5:
    // `ProcurementListings` scans the `@mind/publish` lines); the House counter's lines are out
    // of its scope, so the story publishes each rated line on the player market. Whether House
    // counter lines should feed procurement is with the owner.
    let published = rate_and_publish(&mut world).await;
    rate_unlisted(&mut world).await;
    eprintln!("M8G_STORY phase=zero_pc.rated event=complete");
    log_rating_scores(&world);
    // The House shops once per IC month (the lead's ruling, doc 17h, under the provisional
    // economy defaults): a month closed with nothing listed stays closed, and new listings are
    // seen the next month. The month open while the lines were rated closed at its first visit
    // ("nothing affordable under the cap is listed"), so the House shops in the next IC month,
    // every line listed by then. This is the designed behaviour, not a workaround.
    let next_month = world.rig.tw.world().now() + 31 * 86_400;
    world.rig.tw.set_now(next_month);
    fires(&mut world, 120);
    eprintln!(
        "M8G_STORY zero_pc ledger {} rows; market {}; ic now {}",
        world.rows("tacsim.procurement_ledger").len(),
        census(&world, "market_mind"),
        world.rig.tw.world().world_ic_now().at
    );
    let bought = procured(&world, meridiani);
    eprintln!("M8G_STORY zero_pc bought {bought:?}");
    let charter_of = |selector: &str| -> String {
        published
            .iter()
            .find(|(name, _)| *name == selector)
            .map(|(_, charter)| charter.clone())
            .expect("the published line")
    };
    let (best, second, zero) = (
        charter_of("meridiani"),
        charter_of("coprates"),
        charter_of("safe_conduct"),
    );
    assert_eq!(
        bought.get(&best).copied(),
        Some(2),
        "the best line per credit, to the cap: {bought:?}"
    );
    assert_eq!(
        bought.get(&second).copied(),
        Some(2),
        "the rest of the budget on the second: {bought:?}"
    );
    assert!(
        !bought.contains_key(&zero),
        "the 0 per mille line is untouched: {bought:?}"
    );
    let rows = world.rows("tacsim.procurement_ledger");
    let spent: i64 = rows
        .iter()
        .filter(|row| row["org"] == json!(meridiani.0) && row["phase"] != json!("refunded"))
        .map(|row| row["price_minor"].as_i64().expect("a price"))
        .sum();
    assert_eq!(spent, BUDGET, "the month's budget, exactly");
    // The unlisted line is untouched: every charter the House bought is a published line's.
    assert!(
        bought
            .keys()
            .all(|charter| published.iter().any(|(_, c)| c == charter)),
        "only published lines are bought: {bought:?} of {published:?}"
    );
    let mut world = world.reboot();
    assert_eq!(
        procured(&world, meridiani),
        bought,
        "a restart changes nothing"
    );
    let next_month = world.rig.tw.world().now() + 32 * 86_400;
    world.rig.tw.world_mut().set_now(next_month);
    fires(&mut world, 60);
    assert_eq!(
        procured(&world, meridiani),
        bought,
        "a second month, the treasury spent, buys nothing"
    );
    eprintln!("M8G_STORY phase=zero_pc event=complete");

    // The story's second half (doc 17h §3 M8f's gate, through M8e): a granted member flies one
    // of the House's bought Pins on a cored rig; the open line names the mind, with the House
    // the holder and the member the warrant principal.
    let house_pin = world
        .rows("tacsim.procurement_ledger")
        .into_iter()
        .find(|row| row["org"] == json!(meridiani.0) && row["phase"] == json!("granted"))
        .and_then(|row| {
            row["licence"]
                .as_u64()
                .or_else(|| row["licence"].as_str()?.parse().ok())
        })
        .expect("a licence the House bought");
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    god(
        &mut world.rig,
        &format!("@org/grant #{}=Meridiani roll.holder.org", pilot.0),
    );
    crate::ic_sortie_surface::fit(
        &mut world.rig,
        tv,
        tacsim::definitions::core_replacement::RIG_II,
    );
    let _ = launch(
        &mut world,
        &format!("+tv/1 launch mind=filed:{house_pin} org=Meridiani go"),
    );
    let facts = open_seal(&mut world, tv);
    assert_ne!(
        facts.mind.as_str(),
        "Standard Doctrine",
        "the bought mind flies"
    );
    assert_eq!(
        facts.holder,
        Some(mush_economy::Holder::Organization {
            org: meridiani,
            acting: pilot,
        }),
        "the House is the holder"
    );
    let BlackBoxLine::MindSealed {
        mind,
        principal,
        holder,
        ..
    } = crate::ic_sortie_surface::lines(&world.rig, tv)
        .last()
        .cloned()
        .expect("the open line")
    else {
        panic!("the last line is the open line");
    };
    assert_eq!(mind.as_deref(), Some(facts.mind.as_str()));
    assert_eq!(holder, facts.holder);
    assert_eq!(
        principal,
        Some(tacsim::deliberation::order::CharacterRef::Character { id: pilot })
    );
    eprintln!("M8G_STORY phase=zero_pc.second_half event=complete");
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **The combined world runs an excursion** (fast, not ignored): a rig staff-bound to the Class
/// II root launches and finishes as on the bare roster, its epoch open and sealed (the ROM), and
/// the recover closes it.
#[test]
fn m8g_the_combined_world_runs_an_excursion() {
    let mut world = m8_world();
    let tv = world.tvs[0];
    crate::ic_sortie_surface::fit(
        &mut world.rig,
        tv,
        tacsim::definitions::core_replacement::RIG_II,
    );
    let _ = launch(&mut world, "+tv/1 launch go");
    assert_eq!(open_seal(&mut world, tv).mind.as_str(), "Standard Doctrine");
    recover(&mut world);
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// The fee the Tether back room charges for ungoverning a core, in centicredits.
pub(crate) const BACK_ROOM_FEE: i64 = 25_000;

/// **The Tether back room** (the hardware flow's ungoverning shop, ported): Tether and its
/// `backroom` scope and treasury, a facility chartered `core_ungovern` (never `core_refit`),
/// owned by and titled to Tether, and `industry.core.ungovern` granted to GOD and `agent` on
/// the back room.
fn back_room(rig: &mut Rig, agent: ObjId) -> tacsim::launch::UngoverningCoordinates {
    let shop = thing(rig, "Back-room shop");
    let output = thing(rig, "Back-room completed work");
    for line in ["@org/create Tether", "@org/scope create Tether=backroom"] {
        god(rig, line);
    }
    let tether = org(rig, "Tether");
    let backroom = scope(rig, tether, "backroom");
    god(
        rig,
        &format!("@account/open org:Tether/{}=tacsim.credit", backroom.0),
    );
    let treasury = account(rig, &PrincipalId::Organization(tether), Some(backroom));
    for line in [
        format!("@facility/open {}=2 core_ungovern", shop.0),
        format!("@facility/owner {}=org:Tether", shop.0),
        format!("@asset/title #{}=org:Tether scope={}", shop.0, backroom.0),
    ] {
        god(rig, &line);
    }
    for actor in [GOD, agent] {
        god(
            rig,
            &format!(
                "@org/grant #{}=Tether industry.core.ungovern scope=#{}",
                actor.0, backroom.0
            ),
        );
    }
    tacsim::launch::UngoverningCoordinates {
        provider: mush_economy::core_integration::ScopedCoreAccount {
            account: treasury,
            organization: tether,
            scope: backroom,
        },
        factory: shop,
        output_container: output,
        fee_minor: Some(BACK_ROOM_FEE),
    }
}

impl M8World {
    /// **Advance the world `hours` hours, firing the industry schedule `name` each hour** (the
    /// hardware flow driver's `progress`, ported): the one due composed scheduled action of that
    /// name, prepared and installed as the world task does, its writes kept.
    pub(crate) fn progress(&mut self, name: &str, hours: usize) {
        for hour in 0..hours {
            let utc = self.rig.tw.world().now() + 3_600;
            self.rig.tw.set_now(utc);
            let page = self
                .rig
                .tw
                .engine()
                .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 256)
                .expect("the due schedules");
            let candidates: Vec<_> = page
                .candidates()
                .iter()
                .filter(|c| c.action().key().id.as_str() == name)
                .cloned()
                .collect();
            assert_eq!(candidates.len(), 1, "one due {name} at hour {hour}");
            let candidate = &candidates[0];
            let ingress =
                mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)
                    .expect("the schedule's ingress");
            let plan = self
                .rig
                .tw
                .engine()
                .plan_scheduled_action(candidate, &ingress)
                .expect("the schedule's plan");
            let (_, invocation, _) = plan.into_parts();
            let commit = mush_core::CommitToken::from_bytes(crate::excursions::next_token());
            let (prepared, _) = self
                .rig
                .tw
                .engine_mut()
                .prepare_registered_composed_scheduled_authoritative_action(
                    candidate,
                    &invocation,
                    commit,
                )
                .unwrap_or_else(|error| panic!("{name} at hour {hour}: {error:?}"));
            let (writes, install) = prepared.into_parts();
            self.rig
                .tw
                .engine_mut()
                .install_composed_action(install)
                .expect("the schedule installs");
            crate::excursions::keep_composed(&mut self.rig, writes);
        }
    }

    /// **One certified core of `class` for `requester`** (the hardware flow's
    /// `integrate_class_one`, ported over the class): the stock agent issues a die and offers
    /// the integration, `requester` accepts paying from `payer`, the order runs the shipped
    /// `core_integrate`'s 24, 48 or 96 hours, and the core is delivered. Returns the
    /// acceptance's commit and the core.
    pub(crate) fn integrate(
        &mut self,
        requester: ObjId,
        payer: AccountId,
        class: DieClass,
    ) -> (mush_core::CommitToken, ObjId) {
        let agent = self.deployment.hardware.agents[1].actor;
        let (_, die) = self.perform("industry", "issue_core_die", agent, json!({"class":class}));
        let lot: mush_inventory::LotId =
            serde_json::from_value(die["lot"].clone()).expect("the die's lot");
        let (offer, _) = self.perform(
            "industry",
            "offer_core_integration",
            agent,
            json!({"requester":requester.0,"class":class,"supplier_lot":lot.to_string()}),
        );
        let (accept, order) = self.perform(
            "industry",
            "accept_core_integration",
            requester,
            json!({"offer":offer.to_hex(),"payer":payer.0.to_string()}),
        );
        let id: mush_industry::workorder::WorkOrderId =
            serde_json::from_value(order["order"].clone()).expect("the work order");
        let params = json!({"order":id.to_string()});
        self.perform("industry", "reserve_core_output", requester, params.clone());
        self.progress("advance_core_integration", [24, 48, 96][class.index()]);
        let (_, completed) = self.perform(
            "industry",
            "complete_core_integration",
            requester,
            params.clone(),
        );
        let core: ObjId = serde_json::from_value(completed["object"].clone()).expect("the core");
        self.perform("industry", "deliver_core_output", requester, params);
        (accept, core)
    }

    /// **Refit `hull` with a core of `class`** (the hardware flow's `refit_class`, ported)
    /// under `recipe` at `facility`: `requester` and the hull stand in the facility, the stock
    /// agent offers, `requester` accepts paying from `payer`, the shipped fitting's 1, 2 or 4
    /// hours run, and the core is installed.
    pub(crate) fn refit(
        &mut self,
        (requester, payer): (ObjId, AccountId),
        hull: ObjId,
        ((accept, core), class): ((mush_core::CommitToken, ObjId), DieClass),
        (facility, recipe): (ObjId, Option<&str>),
    ) {
        for thing in [requester, hull] {
            self.rig
                .tw
                .world_mut()
                .move_to(thing, facility)
                .expect("into the facility");
        }
        let agent = self.deployment.hardware.agents[1].actor;
        let mut params = json!({"requester":requester.0,"hull":hull.0,"incoming":core.0,
            "manufacture":accept.to_hex(),"class":class});
        if let Some(recipe) = recipe {
            params["recipe"] = json!(recipe);
        }
        let (offer, _) = self.perform("industry", "offer_core_refit", agent, params);
        let (_, order) = self.perform(
            "industry",
            "accept_core_refit",
            requester,
            json!({"offer":offer.to_hex(),"payer":payer.0.to_string()}),
        );
        let id: mush_industry::workorder::WorkOrderId =
            serde_json::from_value(order["order"].clone()).expect("the work order");
        self.progress("advance_core_refit", [1, 2, 4][class.index()]);
        self.perform(
            "industry",
            "install_core_component",
            requester,
            json!({"order":id.to_string()}),
        );
    }

    /// **A managed core ungoverned in the Tether back room** (doc 17h cut M8g, ruling (a)): a
    /// certified core of `class` is fitted to `hull` at Basin's chartered shop, then a second is
    /// fitted as its ungoverned twin by the back room's `core_ungovern` refit, `requester` (the
    /// hull's titleholder) paying from `payer` throughout. Returns the ungoverned core.
    pub(crate) fn ungovern_in_back_room(
        &mut self,
        (requester, payer): (ObjId, AccountId),
        hull: ObjId,
        class: DieClass,
    ) -> ObjId {
        let back_room = self
            .deployment
            .hardware
            .ungovern
            .clone()
            .expect("the combined world composes the back room");
        let first = self.integrate(requester, payer, class);
        let second = self.integrate(requester, payer, class);
        let factory = self.deployment.hardware.factory;
        let who = (requester, payer);
        self.refit(who, hull, (first, class), (factory, None));
        let core = second.1;
        self.refit(
            who,
            hull,
            (second, class),
            (back_room.factory, Some("core_ungovern")),
        );
        core
    }
}

/// An account's posted balance in centicredits.
fn balance(world: &M8World, id: AccountId) -> i64 {
    economy(&world.rig.tw)
        .snapshot()
        .account(id)
        .expect("the account")
        .posted_balance
        .minor()
}

/// **Doc 17h cut M8g, ruling (a): a real ungoverning refit in the combined world.** A stock
/// prospector rig titled to the pilot is fitted at Basin's chartered shop and then ungoverned in
/// the Tether back room the deployment names; its root is the rig family's ungoverned Class I,
/// `read_core` reads it ungoverned, and Tether's treasury took exactly its fee.
#[tokio::test(flavor = "current_thread")]
async fn m8g_the_back_room_ungoverns_a_managed_core_in_the_combined_world() {
    let mut world = m8_world();
    let provider = world
        .deployment
        .hardware
        .ungovern
        .as_ref()
        .expect("the back room")
        .provider
        .account;
    let before = balance(&world, provider);
    let (hull, core) = ungoverned_rig(&mut world);
    let unit = mush_tactics::module::unit_of(world.rig.tw.world(), hull)
        .expect("the ungoverned rig")
        .clone();
    assert_eq!(
        unit.schema().key().id.as_str(),
        tacsim::definitions::core_replacement::RIG_I_UNGOVERNED
    );
    assert_eq!(
        tacsim::arena::core::read_core(&unit).map(|hw| (hw.class, hw.governed)),
        Some((tacsim::arena::core::CoreClass::I, false))
    );
    assert!(world.rig.tw.world().get(core).is_some(), "the core exists");
    assert_eq!(
        balance(&world, provider) - before,
        BACK_ROOM_FEE,
        "Tether took its fee"
    );
}

/// **A stock prospector rig titled to the pilot, ungoverned in the Tether back room** (ruling
/// (a)): the pilot's account funded from the hardware faucet, the rig made at Basin's shop,
/// fitted there and ungoverned in the back room. Returns the rig and its ungoverned core.
pub(crate) fn ungoverned_rig(world: &mut M8World) -> (ObjId, ObjId) {
    let pilot = world.pilot;
    let payer = account(&world.rig, &PrincipalId::Object(pilot), None);
    let faucet = account(&world.rig, &system("hardware.fixture"), None);
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} 1000000", faucet.0, payer.0),
    );
    let factory = world.deployment.hardware.factory;
    let hull = world
        .rig
        .tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Prospector rig",
            pilot,
            Some(factory),
        )
        .expect("a rig");
    god(
        &mut world.rig,
        &format!(
            "@unit/create #{}={}",
            hull.0,
            tacsim::definitions::content::TV_PROSPECTOR
        ),
    );
    god(
        &mut world.rig,
        &format!("@asset/title #{}=#{}", hull.0, pilot.0),
    );
    let core = world.ungovern_in_back_room((pilot, payer), hull, DieClass::I);
    (hull, core)
}

/// The Red Dies clock's filled segments on the Republic's board.
fn red_dies_filled(world: &M8World) -> u32 {
    let sim = world
        .rig
        .tw
        .world()
        .module_state::<mush_faction::SimState>()
        .expect("the fronts");
    let front = sim
        .fronts
        .values()
        .find(|f| f.title == mush_faction::RED_DIES_TITLE)
        .expect("the Red Dies front");
    assert_eq!(front.faction, world.deployment.republic, "the Republic's");
    sim.clocks
        .values()
        .find(|c| c.front == front.id)
        .map(|c| u32::from(c.filled))
        .expect("its clock")
}

/// **Doc 17h cut M8g, ruling (a): a real ungoverned core is found, chronicled and revoked.**
/// The Registry operator inspects the pilot's leased rig after its back-room refit (the
/// inspection reads known port hulls only), and the record
/// reads the real core ungoverned (no seeded record): the core, Class I, `governed: false`,
/// the pilot the holder. The finding is chronicled as one canon line that names nobody, and
/// the Red Dies clock on the Republic's board moves one step. The operator then revokes the
/// pilot by that inspection for the configured term, and the pilot alone is told.
#[tokio::test(flavor = "current_thread")]
async fn m8g_a_real_ungoverned_core_is_found_chronicled_and_revoked() {
    use tacsim::minds::registry_inspection::{HullClass, InspectionHost};
    let mut world = m8_world();
    let pilot = world.pilot;
    let operator = world
        .deployment
        .market
        .as_ref()
        .expect("format 2")
        .registry_authority
        .operators[0];
    let hull = world.tvs[0];
    let core = ungovern_leased_rig(&mut world, DieClass::I);
    assert_eq!(red_dies_filled(&world), 0, "no finding yet");
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        operator,
        &format!("@registry/inspect #{}", hull.0),
    );
    assert_eq!(queued.len(), 1, "the inspection stages: {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    let record = crate::excursions::inspections(&world.rig.tw)
        .into_iter()
        .filter(|record| record.hull == hull)
        .max_by_key(|record| record.seq)
        .expect("the inspection landed");
    assert_eq!(record.core, Some(core), "the real core");
    assert_eq!(record.class, HullClass::I);
    assert_eq!(record.governed, Some(false), "read ungoverned");
    assert_eq!(record.holder, Some(PrincipalId::Object(pilot)));
    let family = InspectionHost::of(world.rig.tw.world())
        .expect("the inspection")
        .writer()
        .family()
        .expect("the record family");
    crate::ic_sortie_surface::announce(
        &mut world.rig,
        &family,
        vec![mush_roll::commerce::registry_inspection::inspection_key(
            hull, record.seq,
        )],
    );
    fires(&mut world, 3);
    assert_eq!(red_dies_filled(&world), 1, "the clock one step on");
    let lines: Vec<String> = world
        .rig
        .tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|state| state.recent())
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == tacsim::minds::registry_inspection::unwarranted::KIND)
        .map(|event| event.summary)
        .collect();
    assert_eq!(
        lines,
        vec![tacsim::minds::registry_inspection::unwarranted::LINE.to_owned()],
        "one canon line naming nobody"
    );
    let pilot_name = world
        .rig
        .tw
        .world()
        .get(pilot)
        .map(|o| o.name.to_string())
        .expect("the pilot");
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        operator,
        &format!("@registry/revoke principal=#{} #{}", pilot.0, hull.0),
    );
    assert_eq!(
        out,
        format!("Revoking {pilot_name} for 90 IC days.\nQueued.")
    );
    crate::excursions::land_all(&mut world.rig, &queued);
    let revoked = InspectionHost::of(world.rig.tw.world())
        .expect("the inspection")
        .writer()
        .revoked_principals()
        .expect("the family");
    let bar = world
        .rig
        .tw
        .engine()
        .action_state()
        .get(&revoked, &format!("{:020}", pilot.0))
        .expect("the family reads")
        .cloned()
        .expect("the pilot's bar");
    assert_eq!(bar.seq, record.seq, "by that inspection");
    assert_eq!(bar.until_ic - bar.from_ic, 90 * 86_400);
}

/// **The pilot's leased rig, ungoverned for real at `class`** (doc 17h cut M8g, ruling (a)):
/// staff release the rig's claims and title it to the pilot, the pilot's account is funded, the
/// rig is fitted at Basin's shop and ungoverned in the Tether back room, and staff put it and
/// the pilot back where they stood (the rig in the Dray-P's bay). Returns the ungoverned core.
pub(crate) fn ungovern_leased_rig(world: &mut M8World, class: DieClass) -> ObjId {
    on_leased_rig(world, |world, who, tv| {
        world.ungovern_in_back_room(who, tv, class)
    })
}

/// **Fit the pilot's leased rig by `fit`** ([`ungovern_leased_rig`]'s frame): the rig's claims
/// released and its title the pilot's, the pilot's account funded, `fit` run with the pilot and
/// that account, and the rig and the pilot put back where they stood. Returns `fit`'s core.
fn on_leased_rig(
    world: &mut M8World,
    fit: impl FnOnce(&mut M8World, (ObjId, AccountId), ObjId) -> ObjId,
) -> ObjId {
    let tv = world.tvs[0];
    on_leased(world, tv, fit)
}

/// [`on_leased_rig`] for any of the pilot's rigs (doc 23 cut S.4b-3).
fn on_leased(
    world: &mut M8World,
    tv: ObjId,
    fit: impl FnOnce(&mut M8World, (ObjId, AccountId), ObjId) -> ObjId,
) -> ObjId {
    let pilot = world.pilot;
    let where_tv = world
        .rig
        .tw
        .world()
        .get(tv)
        .and_then(|o| o.location)
        .expect("the rig is somewhere");
    let where_pilot = world
        .rig
        .tw
        .world()
        .get(pilot)
        .and_then(|o| o.location)
        .expect("the pilot is somewhere");
    let claims = economy(&world.rig.tw)
        .snapshot()
        .asset(tv)
        .map_or(0, |a| a.encumbrances.len());
    for k in (1..=claims).rev() {
        god(&mut world.rig, &format!("@asset/release #{}={k}", tv.0));
    }
    god(
        &mut world.rig,
        &format!("@asset/transfer #{}=#{}", tv.0, pilot.0),
    );
    let payer = account(&world.rig, &PrincipalId::Object(pilot), None);
    let faucet = account(&world.rig, &system("hardware.fixture"), None);
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} 1000000", faucet.0, payer.0),
    );
    let core = fit(world, (pilot, payer), tv);
    for (thing, place) in [(tv, where_tv), (pilot, where_pilot)] {
        world
            .rig
            .tw
            .world_mut()
            .move_to(thing, place)
            .expect("back where it stood");
    }
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));
    core
}

/// The filed revision a filing row names (`roll_mind_initial_filing`'s `filed`), by its
/// owner licence.
pub(crate) fn filed_of(world: &M8World, licence: mush_roll::commerce::LicenceId) -> u64 {
    let row = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .find(|row| row.get("licence") == Some(&json!(licence)))
        .expect("the licence's filing");
    row.get("filed")
        .and_then(|filed| filed.as_str()?.parse().ok())
        .unwrap_or_else(|| panic!("the filing names its filed revision: {row}"))
}

/// `@registry/inspect` the pilot's leased rig as the Registry operator, landed; the record.
fn inspect_leased(
    world: &mut M8World,
) -> mush_roll::commerce::registry_inspection::InspectionRecord {
    let operator = world
        .deployment
        .market
        .as_ref()
        .expect("format 2")
        .registry_authority
        .operators[0];
    let hull = world.tvs[0];
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        operator,
        &format!("@registry/inspect #{}", hull.0),
    );
    assert_eq!(queued.len(), 1, "the inspection stages: {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    crate::excursions::inspections(&world.rig.tw)
        .into_iter()
        .filter(|record| record.hull == hull)
        .max_by_key(|record| record.seq)
        .expect("the inspection landed")
}

/// What the Registry operator is told of `line`, its submission landed.
fn operator_says(world: &mut M8World, line: &str) -> String {
    let operator = world
        .deployment
        .market
        .as_ref()
        .expect("format 2")
        .registry_authority
        .operators[0];
    let (out, queued) = crate::excursions::command(&mut world.rig, operator, line);
    crate::excursions::land_all(&mut world.rig, &queued);
    out
}

/// The economy's title of `object`.
fn title_of(world: &M8World, object: ObjId) -> mush_economy::EconomicAsset {
    economy(&world.rig.tw)
        .snapshot()
        .asset(object)
        .cloned()
        .expect("a titled object")
}

/// **Doc 17h cut M8g G2: M8d2's seizure end to end over a REAL managed core** (doc 17h §3
/// M8d2, S1–S7; the lead's "impounded in place"). The pilot's leased rig is ungoverned in the
/// Tether back room; the Registry operator inspects it (the record reads the real core
/// ungoverned) and seizes it: the core's title takes the Registry's dispute claim and keeps its
/// holder, and the hull's next launch seals the ROM as no core (Class 0, governed, no core
/// named) while the ungoverned core stays fitted. Forfeit within the hold is refused by the
/// lead's sentence; after the hold it moves the core's title to the Registry organisation with
/// the claim released, and the hull still flies the ROM.
#[test]
#[allow(clippy::too_many_lines)]
fn m8g_a_real_ungoverned_core_is_seized_impounded_and_forfeited() {
    let mut world = m8_world();
    let tv = world.tvs[0];
    let n = tv.0;
    let core = ungovern_leased_rig(&mut world, DieClass::I);
    let record = inspect_leased(&mut world);
    assert_eq!(
        (record.core, record.governed),
        (Some(core), Some(false)),
        "the real core, read ungoverned"
    );
    let before = title_of(&world, core);
    let seized = operator_says(&mut world, &format!("@registry/seize #{n}"));
    eprintln!("M8G seize -> {seized}");
    let held = title_of(&world, core);
    let registry = PrincipalId::Organization(
        world
            .deployment
            .market
            .as_ref()
            .expect("format 2")
            .registry_authority
            .organization,
    );
    assert!(
        matches!(
            held.encumbrances.as_slice(),
            [mush_economy::Encumbrance::Disputed { claimant, .. }] if *claimant == registry
        ),
        "the Registry's claim on the core: {:?}",
        held.encumbrances
    );
    assert_eq!(
        held.title_owner, before.title_owner,
        "seizure keeps the title"
    );
    // Impounded in place: the next launch flies the ROM as no core, the core still fitted.
    let _ = launch(&mut world, "+tv/1 launch go");
    let facts = open_seal(&mut world, tv);
    assert_eq!(
        (facts.class, facts.governed, facts.core.clone()),
        (tacsim::arena::core::CoreClass::Zero, true, None),
        "the seized rig flies the ROM"
    );
    let unit = mush_tactics::module::unit_of(world.rig.tw.world(), tv)
        .expect("the rig")
        .clone();
    assert!(
        tacsim::arena::core::read_core(&unit).is_some_and(|hw| !hw.governed),
        "the ungoverned core stays fitted"
    );
    recover(&mut world);
    // Forfeit within the hold: refused.
    assert_eq!(
        operator_says(&mut world, &format!("@registry/forfeit #{n}")).trim(),
        "That seizure is still within its hold."
    );
    // After the hold: the core is the Registry's, the claim released.
    let later = world.rig.tw.world().now() + 3 * 86_400 + 60;
    world.rig.tw.set_now(later);
    let forfeit = operator_says(&mut world, &format!("@registry/forfeit #{n}"));
    eprintln!("M8G forfeit -> {forfeit}");
    let gone = title_of(&world, core);
    assert_eq!(gone.title_owner, registry, "forfeit to the Registry");
    assert!(gone.encumbrances.is_empty(), "the claim released");
    let _ = launch(&mut world, "+tv/1 launch go");
    let facts = open_seal(&mut world, tv);
    assert_eq!(
        facts.class,
        tacsim::arena::core::CoreClass::Zero,
        "the Registry's core flies nothing"
    );
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **Doc 17h cut M8g G2: M8b-2's grey die over the combined world** (doc 17h §3 M8b-2,
/// H1–H4). GOD issues a grey Class I die into the pilot's satchel, no origin recorded; the
/// Tether back room integrates it as an ungoverned Class I (a certified Class I fitted at Basin
/// first: `core_ungovern` never fits a coreless chassis) and fits it into the pilot's leased
/// rig; the Registry's inspection reads the core `Grey`, Class I, ungoverned, and the operator
/// may seize it. (M8b-2's heirloom is Class III, which integrates only under a Class III
/// entitlement the production launch does not compose: the M8g report.)
#[test]
fn m8g_a_grey_core_reads_grey_to_the_inspection_and_is_seizable() {
    use tacsim::minds::registry_inspection::{HullClass, Provenance};
    let mut world = m8_world();
    let n = world.tvs[0].0;
    let core = on_leased_rig(&mut world, |world, (pilot, payer), tv| {
        let satchel = world
            .rig
            .tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                "Pilot's satchel",
                pilot,
                Some(pilot),
            )
            .expect("a satchel");
        god(
            &mut world.rig,
            &format!(
                "@vault/create PilotStash=container:#{} owner=#{}",
                satchel.0, pilot.0
            ),
        );
        let stash = vault(&world.rig, "PilotStash");
        let (_, die) = world.perform(
            "industry",
            "issue_grey_die",
            GOD,
            json!({"class":DieClass::I,"vault":stash.0}),
        );
        let lot: mush_inventory::LotId =
            serde_json::from_value(die["lot"].clone()).expect("the grey die's lot");
        // A certified Class I at Basin, which the grey core displaces.
        let first = world.integrate(pilot, payer, DieClass::I);
        let factory = world.deployment.hardware.factory;
        world.refit((pilot, payer), tv, (first, DieClass::I), (factory, None));
        // The back room integrates the grey die and fits it ungoverned.
        let back_room = world
            .deployment
            .hardware
            .ungovern
            .clone()
            .expect("the back room");
        for thing in [pilot, tv] {
            world
                .rig
                .tw
                .world_mut()
                .move_to(thing, back_room.factory)
                .expect("into the back room");
        }
        let agent = world.deployment.hardware.agents[1].actor;
        let (offer, _) = world.perform(
            "industry",
            "offer_core_integration",
            agent,
            json!({"requester":pilot.0,"class":DieClass::I,"site":"core_ungovern"}),
        );
        let (accept, order) = world.perform(
            "industry",
            "accept_core_integration",
            pilot,
            json!({"offer":offer.to_hex(),"payer":payer.0.to_string(),"input_lot":lot.to_string()}),
        );
        let id: mush_industry::workorder::WorkOrderId =
            serde_json::from_value(order["order"].clone()).expect("the work order");
        let params = json!({"order":id.to_string()});
        world.perform("industry", "reserve_core_output", pilot, params.clone());
        world.progress("advance_core_integration", 24);
        let (_, completed) = world.perform(
            "industry",
            "complete_core_integration",
            pilot,
            params.clone(),
        );
        let grey: ObjId =
            serde_json::from_value(completed["object"].clone()).expect("the grey core");
        world.perform("industry", "deliver_core_output", pilot, params);
        world.refit(
            (pilot, payer),
            tv,
            ((accept, grey), DieClass::I),
            (back_room.factory, Some("core_ungovern")),
        );
        grey
    });
    let record = inspect_leased(&mut world);
    assert_eq!(record.core, Some(core));
    assert_eq!(record.class, HullClass::I);
    assert_eq!(record.governed, Some(false));
    assert_eq!(record.provenance, Provenance::Grey);
    assert!(title_of(&world, core).encumbrances.is_empty());
    let seized = operator_says(&mut world, &format!("@registry/seize #{n}"));
    eprintln!("M8G grey seize -> {seized}");
    assert!(
        matches!(
            title_of(&world, core).encumbrances.as_slice(),
            [mush_economy::Encumbrance::Disputed { .. }]
        ),
        "the grey core is seizable: {seized}"
    );
}

/// **Diagnostic** (doc 17h cut M8g, the zero-PC story): each rating's score as P1's
/// `rating_score` computes it over its result pages, and per class the rows by outcome.
fn log_rating_scores(world: &M8World) {
    let mut by_rating: std::collections::BTreeMap<
        String,
        Vec<mush_roll::commerce::rating::RatingPage>,
    > = std::collections::BTreeMap::new();
    for value in world.rows("rating_pages") {
        let rating = value["rating"].to_string();
        match serde_json::from_value::<mush_roll::commerce::rating::RatingPage>(value.clone()) {
            Ok(page) => by_rating.entry(rating).or_default().push(page),
            Err(error) => eprintln!("M8G_DIAG an undecodable page: {error}"),
        }
    }
    for (rating, pages) in &by_rating {
        let contents: Vec<_> = pages.iter().map(|page| page.content().clone()).collect();
        let score = mush_roll::commerce::rating::rating_score(&contents);
        let mut rows = std::collections::BTreeMap::<String, usize>::new();
        for page in pages {
            let value = serde_json::to_value(page.content()).unwrap_or_default();
            for row in value["rows"].as_array().cloned().unwrap_or_default() {
                let outcome = match &row["outcome"] {
                    Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
                    other => other.to_string(),
                };
                let win =
                    row.to_string().contains("\"Win\"") || row.to_string().contains("\"win\"");
                *rows
                    .entry(format!(
                        "{} {outcome}{}",
                        row["key"]["class"],
                        if win { " win" } else { "" }
                    ))
                    .or_default() += 1;
            }
        }
        eprintln!("M8G_DIAG rating {rating}: score {score:?}; rows {rows:?}");
    }
}

/// **The House's procured licences, by charter** (the zero-PC story): the procurement
/// ledger's rows for `org` that count (not refunded), each licence once.
fn procured(world: &M8World, org: OrgId) -> std::collections::BTreeMap<String, usize> {
    let mut seen = std::collections::BTreeSet::new();
    let mut by_charter = std::collections::BTreeMap::new();
    for row in world.rows("tacsim.procurement_ledger") {
        if row["org"] == json!(org.0)
            && row["phase"] != json!("refunded")
            && seen.insert(row["licence"].to_string())
        {
            *by_charter.entry(row["charter"].to_string()).or_insert(0) += 1;
        }
    }
    by_charter
}

/// **A line on the player market** (the zero-PC story; doc 17h §3 M8f, P5): GOD files the
/// certified revision personally (the Roll's `initial_filing`, paid from GOD's own account)
/// and publishes it at the counter (`@mind/publish <filed> form=sealed`, the market's List).
/// Procurement scans these listings (`ProcurementListings`), never the House counter's. Returns
/// the charter.
fn file_and_publish(
    world: &mut M8World,
    (revision, rating): &(mush_roll::Revision, mush_roll::commerce::rating::RatingRef),
    charter_name: &str,
) -> String {
    let existing = economy(&world.rig.tw)
        .snapshot()
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(GOD))
        .map(|a| a.id);
    let payer = if let Some(id) = existing {
        id
    } else {
        god(&mut world.rig, "@account/open #1=tacsim.credit");
        account(&world.rig, &PrincipalId::Object(GOD), None)
    };
    let faucet = account(&world.rig, &system("hardware.fixture"), None);
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} 100000", faucet.0, payer.0),
    );
    let (shell, _) = world.perform(
        "roll",
        mush_core::reserved_shell::RESERVE_ACTION,
        GOD,
        json!({}),
    );
    let reference = revision.reference();
    let (filing, _) = world.perform(
        "roll",
        "initial_filing",
        GOD,
        json!({
            "shell_request": mush_core::RequestUid::from_bytes(*shell.as_bytes()),
            "mind": reference.key.mind, "revision": reference.key.number,
            "revision_digest": reference.digest, "rating_job": rating.job(),
            "rating_id": rating.id(), "rating_digest": rating.receipt_digest(),
            "payer": payer.0, "charter_name": charter_name
        }),
    );
    let birth = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .find(|row| row.get("commit") == Some(&json!(filing)))
        .expect("the personal filing");
    let filed = birth
        .get("filed")
        .and_then(|filed| filed.as_str()?.parse::<u64>().ok())
        .expect("its filed revision");
    let charter = birth["charter"].to_string();
    let room = world.deployment.venue.room;
    world
        .rig
        .tw
        .world_mut()
        .move_to(GOD, room)
        .expect("GOD at the counter");
    let out = run(
        &mut world.rig.tw,
        GOD,
        &format!("@mind/publish {filed} form=sealed"),
    );
    let mut staged = Vec::new();
    while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
        staged.push(*submit);
    }
    assert_eq!(staged.len(), 1, "the List stages: {out}");
    crate::excursions::land(&mut world.rig, &staged[0])
        .unwrap_or_else(|error| panic!("the List lands: {error}"));
    eprintln!("M8G_STORY published {charter_name}: {out}");
    charter
}

/// **The zero-PC story's rated half**: the three named sources certified, then each filed by
/// GOD personally and published on the player market ([`file_and_publish`]). Returns each
/// line's selector and charter.
async fn rate_and_publish(world: &mut M8World) -> Vec<(&'static str, String)> {
    let houses = world.deployment.houses.clone();
    let mut published = Vec::new();
    for (selector, house) in [
        ("coprates", houses.coprates),
        ("meridiani", houses.meridiani),
        ("safe_conduct", houses.tycho),
    ] {
        let certified = certify(world, selector, &house.source_name.clone()).await;
        let charter = file_and_publish(world, &certified, &format!("{} Market", house.source_name));
        published.push((selector, charter));
    }
    published
}

/// **`go` a licensed launch of rig `n` and fly it out** (doc 23 cut S.4b-2's steps 2 and 3): the
/// pilot's `line` lands, the record holds `Opening` under `licence` with the launch queued
/// behind it and the rig in its bay; the surface tick's claim arm opens the claim in the bay
/// (the book's active row, the pilot its dispatcher) and starts the queued launch; the launch
/// finishes. Returns the claim's open token.
fn claimed_launch(
    world: &mut M8World,
    n: usize,
    line: &str,
    licence: u64,
) -> mush_core::CommitToken {
    use tacsim::surface::excursion::ClaimCursor;
    let (pilot, tv) = (world.pilot, world.tvs[n - 1]);
    let (out, queued) = crate::excursions::command(&mut world.rig, pilot, line);
    assert_eq!(queued.len(), 1, "one start: {out}");
    crate::excursions::land(&mut world.rig, &queued[0])
        .unwrap_or_else(|refused| panic!("{line}: the go lands: {refused}"));
    let record = crate::excursions::record_of(&mut world.rig.tw, tv).expect("the record");
    assert!(
        matches!(record.claim.as_deref(), Some(ClaimCursor::Opening { operator, licence: l, .. })
            if *operator == pilot && *l == licence),
        "{:?}",
        record.claim
    );
    assert!(record.act.is_none(), "no act before the claim opens");
    assert!(
        record
            .queued
            .as_ref()
            .is_some_and(|queued| queued.launch.is_some())
    );
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));
    let mut open = None;
    for _ in 0..12 {
        fires(world, 1);
        if let tacsim::IcClaimRead::Active {
            open: token,
            dispatcher,
        } = tacsim::ic_claim_of(world.rig.tw.world(), tv)
        {
            assert_eq!(dispatcher, pilot);
            open = Some(token);
        }
        if crate::excursions::record_of(&mut world.rig.tw, tv).is_some_and(|r| r.act.is_some()) {
            break;
        }
    }
    let Some(open) = open else {
        panic!(
            "{line}: the claim did not open in the bay: {:?}",
            crate::excursions::record_of(&mut world.rig.tw, tv).and_then(|r| r.claim)
        )
    };
    let record = crate::excursions::record_of(&mut world.rig.tw, tv).expect("the record");
    assert!(
        matches!(record.claim.as_deref(), Some(ClaimCursor::Open { open: o, .. }) if *o == open),
        "{:?}",
        record.claim
    );
    let _ = crate::excursions::finish(&mut world.rig, tv, pilot);
    open
}

/// **Bring rig `n` home and land its claim's Close** (doc 23 cut S.4b-2's step 4): the recover
/// closes the epoch and owes the Close; the claim arm lands it (the book's row closed, no
/// active claim on the rig).
fn claimed_recover(world: &mut M8World, n: usize, open: mush_core::CommitToken) {
    use tacsim::surface::excursion::ClaimCursor;
    let (pilot, tv) = (world.pilot, world.tvs[n - 1]);
    let _ = crate::excursions::act_go(&mut world.rig, pilot, &format!("+tv/{n} recover go"));
    let _ = crate::excursions::finish(&mut world.rig, tv, pilot);
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));
    let record = crate::excursions::record_of(&mut world.rig.tw, tv).expect("the record");
    assert!(
        matches!(record.claim.as_deref(), Some(ClaimCursor::Closing { open: o, .. }) if *o == open),
        "{:?}",
        record.claim
    );
    for _ in 0..12 {
        fires(world, 1);
        if tacsim::ic_claim_closed(world.rig.tw.world(), open).is_some() {
            break;
        }
    }
    assert!(
        tacsim::ic_claim_closed(world.rig.tw.world(), open).is_some(),
        "the Close landed"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), tv),
        tacsim::IcClaimRead::None
    );
}

/// **Doc 23 cuts S.4b-2 and S.4c; doc 44 MW.0 and MW.22: on the production composition, a
/// licensed rig's claim opens in its bay and closes at home, whoever's hull it is by right**
/// (§17.10, §18; D-23-13, D-23-14, D-23-21, D-23-22, D-23-23 step 1, D-23-24). The combined
/// world as the production boot composes it ([`m8_world_field`]: `LaunchComposition::prepare`
/// and nothing after it), two rigs leased from Occator in the Dray-P's bay, both bound to the
/// governed Class II root; one genuine rating; the pilot's paid one-seat House Pin and the
/// Coprates House's own owner licence. Through the production command paths:
/// 1. **a chartered pilot's leased rig** (titled to Occator, the pilot its lessee): `go` lands,
///    the record holds `Opening` and the launch queued behind it, the rig in its bay and no act;
///    the surface tick's claim arm opens the claim in the bay, then starts the queued launch;
///    the launch's finish opens the epoch with the claim's own seal (`seal_digest` equal by
///    construction, the licence the paid Pin's, the seal's sortie the epoch's IC id);
/// 2. **the one seat** (D-23-16): the second leased rig's `go` under the same Pin is refused
///    while the first is out, nothing filed;
/// 3. **a resale pending** (D-UM-40): the pilot offers the Pin to a buyer, who accepts; with a
///    claim open under it the acceptance is escrowed, not settled, and the second rig's `go`
///    under the Pin is refused by the pending sentence, naming the mind;
/// 4. `+tv/1 recover go` closes the epoch `ReturnedToBase` and owes the Close; the arm lands
///    it (the book's row closed, no active claim);
/// 5. **a hull the pilot has no authority over**: staff title the first rig to the Meridiani
///    House. The pilot, granted to fly the Coprates House's licence, holds no office with
///    Meridiani: the `go` is refused by name, with the hull's and the House's, nothing filed;
/// 6. **a House's licence on a House's hull**: staff title the second rig to the Coprates
///    House. The pilot's `go` under the House's owner licence (`org=Coprates`) opens the claim
///    in the bay and launches; the open line names the House the holder with the pilot acting;
///    the recover closes it, and a plain launch afterwards is not held by the spent cursor.
#[tokio::test(flavor = "current_thread")]
#[ignore = "M8 combined-world story: a genuine Registry rating; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home() {
    use mush_roll::commerce::ic_claims::{HullAuthority, hull_authority};
    use tacsim::minds::licence_state::SealLicenceState;
    // The owners' refusal reasons, which the player never hears (`ic_claim` at debug).
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,ic_claim=debug")
        .with_test_writer()
        .try_init();
    let mut world = m8_world_field(("ahuna", "ahuna_port"), 2);
    assert!(tacsim::ic_claim_path_composed(world.rig.tw.world()));
    let (coprates, owner_licence, listing) = rate_and_list_coprates(&mut world).await;
    let paid = buy(&mut world, listing);
    let (pilot, tv, second) = (world.pilot, world.tvs[0], world.tvs[1]);
    for rig_tv in [tv, second] {
        crate::ic_sortie_surface::fit(
            &mut world.rig,
            rig_tv,
            tacsim::definitions::core_replacement::RIG_II,
        );
    }
    let pin = |n: usize| format!("+tv/{n} launch mind=filed:{} go", paid.get());
    let nothing_filed = |world: &mut M8World, rig_tv: ObjId| {
        assert!(crate::excursions::in_bay(&world.rig.tw, rig_tv, &world.row));
        assert!(
            crate::excursions::record_of(&mut world.rig.tw, rig_tv)
                .is_none_or(|record| record.claim.as_deref().is_none_or(|c| !c.is_live())),
            "nothing filed"
        );
    };

    // 1. A chartered pilot's leased rig: the lease is the authority (doc 23 cut S.4c).
    assert!(
        matches!(
            hull_authority(world.rig.tw.world(), tv, pilot).map(|admitted| admitted.authority),
            Ok(HullAuthority::Lease { .. })
        ),
        "the rig is the pilot's by lease, not by title"
    );
    let open = claimed_launch(&mut world, 1, &pin(1), paid.get());
    let facts = open_seal(&mut world, tv);
    let seal = tacsim::ic_claim_seal(world.rig.tw.world(), open).expect("the claim's seal");
    assert_eq!(
        facts.seal_digest,
        tacsim::surface::black_box::SealFacts::of(&seal, SealLicenceState::Active).seal_digest,
        "one seal: the claim's"
    );
    assert_eq!(facts.licence.as_str(), paid.get().to_string());
    assert_eq!(facts.holder, Some(mush_economy::Holder::Character(pilot)));
    let launch_ic = crate::excursions::record_of(&mut world.rig.tw, tv)
        .and_then(|record| record.excursion)
        .expect("the rig is out")
        .id
        .launch_ic;
    assert_eq!(
        seal.sortie,
        tacsim::arena::mind::SortieId::ic(tv, launch_ic),
        "the claim seals the epoch's own id"
    );
    eprintln!("M8G_STORY phase=s4c.leased event=complete");

    // 2. D-23-16: the one-seat Pin's seat is held; the second leased rig is refused at `go`.
    let refused = crate::ic_sortie_surface::start_refusal(&mut world.rig, pilot, &pin(2))
        .expect("the second rig is refused");
    assert!(
        refused.contains(
            &tacsim::surface::dispatch::MIND_SEATS_HELD
                .replace("{licence}", &paid.get().to_string())
                .replace("{held}", "1")
                .replace("{seats}", "1")
        ),
        "{refused}"
    );
    nothing_filed(&mut world, second);
    eprintln!("M8G_STORY phase=s4c.seat event=complete");

    // 3. A resale pending: with the claim open under the Pin the buyer's acceptance is
    //    escrowed (`ResaleStep::Fund`), and the Pin is not sealed aboard until it settles.
    let buyer = world.rig.tw.spawn_player("Resale buyer");
    crate::excursions::with_account(&mut world.rig, buyer, "ResaleBuyer", 9_002);
    let faucet = account(&world.rig, &system("hardware.fixture"), None);
    god(
        &mut world.rig,
        &format!("@account/open #{}=tacsim.credit", buyer.0),
    );
    let buyer_account = account(&world.rig, &PrincipalId::Object(buyer), None);
    god(
        &mut world.rig,
        &format!("@econ/mint {}={} 100000", faucet.0, buyer_account.0),
    );
    let room = world.deployment.venue.room;
    let aboard = world.rig.tw.world().get(pilot).and_then(|o| o.location);
    for who in [pilot, buyer] {
        world
            .rig
            .tw
            .world_mut()
            .move_to(who, room)
            .expect("at the counter");
    }
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        pilot,
        &format!("+minds/resell {}=Resale buyer price=1", paid.get()),
    );
    eprintln!("M8G_STORY resell -> {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    god(&mut world.rig, "@faction/add Republic=Resale buyer/level:1");
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        buyer,
        &format!("+minds/resell/accept {} price=1", paid.get()),
    );
    eprintln!("M8G_STORY resale accept -> {out}");
    crate::excursions::land_all(&mut world.rig, &queued);
    if let Some(aboard) = aboard {
        world
            .rig
            .tw
            .world_mut()
            .move_to(pilot, aboard)
            .expect("the pilot back aboard");
    }
    let refused = crate::ic_sortie_surface::start_refusal(&mut world.rig, pilot, &pin(2))
        .expect("the second rig is refused while the Pin changes hands");
    assert!(
        refused.contains(
            &tacsim::surface::dispatch::MIND_PENDING
                .replace("{mind}", coprates.source_name.as_str())
        ),
        "{refused}"
    );
    nothing_filed(&mut world, second);
    eprintln!("M8G_STORY phase=s4c.pending event=complete");

    // 4. Home: the epoch closes and the claim's Close lands.
    claimed_recover(&mut world, 1, open);
    eprintln!("M8G_STORY phase=s4c.home event=complete");

    // 5. A hull the pilot has no authority over. The pilot may fly the Coprates House's
    //    licence (the grants its licence use asks, on the root and the publication scopes);
    //    the first rig is now the Meridiani House's, and the pilot holds no office there.
    for line in [
        format!("@org/grant #{}=Coprates roll.holder.org", pilot.0),
        format!(
            "@org/grant #{}=Coprates roll.holder.org scope=#{}",
            pilot.0, coprates.scope.0
        ),
    ] {
        god(&mut world.rig, &line);
    }
    let house = |n: usize| {
        format!(
            "+tv/{n} launch mind=filed:{} org=Coprates go",
            owner_licence.get()
        )
    };
    title_rig_to_house(&mut world, tv, "Meridiani");
    let mark = world.rig.tw.world().get(tv).expect("the rig").name.clone();
    let refused = crate::ic_sortie_surface::start_refusal(&mut world.rig, pilot, &house(1))
        .expect("a hull with no title, lease or office is refused");
    assert!(
        refused.contains(&format!("{mark} is "))
            && refused.contains("Meridiani")
            && refused.contains(
                "a licensed mind flies a hull titled to you, leased to you, or held by an \
                 organisation you act for"
            ),
        "{refused}"
    );
    nothing_filed(&mut world, tv);
    eprintln!("M8G_STORY phase=s4c.refused event=complete");

    // 6. A House's licence on a House's hull: the second rig is the Coprates House's, and the
    //    pilot acts for Coprates.
    title_rig_to_house(&mut world, second, "Coprates");
    assert!(
        matches!(
            hull_authority(world.rig.tw.world(), second, pilot).map(|admitted| admitted.authority),
            Ok(HullAuthority::Office { org, .. }) if org == coprates.organization
        ),
        "the rig is the pilot's by office"
    );
    let open = claimed_launch(&mut world, 2, &house(2), owner_licence.get());
    let facts = open_seal(&mut world, second);
    assert_eq!(facts.mind.as_str(), coprates.source_name.clone());
    assert_eq!(facts.licence.as_str(), owner_licence.get().to_string());
    assert_eq!(
        facts.holder,
        Some(mush_economy::Holder::Organization {
            org: coprates.organization,
            acting: pilot,
        }),
        "the House is the holder, the pilot acting"
    );
    let seal = tacsim::ic_claim_seal(world.rig.tw.world(), open).expect("the claim's seal");
    assert_eq!(
        facts.seal_digest,
        tacsim::surface::black_box::SealFacts::of(&seal, SealLicenceState::Active).seal_digest,
        "one seal: the claim's"
    );
    claimed_recover(&mut world, 2, open);
    let _ = crate::excursions::act_go(&mut world.rig, pilot, "+tv/2 launch go");
    assert!(
        crate::excursions::record_of(&mut world.rig.tw, second).is_some_and(|r| r.act.is_some()),
        "a spent claim holds nothing"
    );
    eprintln!("M8G_STORY phase=s4c.house event=complete");
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **Hand the M8 world's live world to the field harness** (doc 23 cut S.4b-3): the same
/// `TestWorld`, store and typed rows under a `driving_rig::Rig`, whose field helpers (the arena's
/// egress pumped by hand, the composed tick, the raiders) run the field. The world is not
/// rebooted after the hand-over: a `driving_rig` reboot composes no launch.
///
/// Doc 23 cut D-23-18: the M8 world's own parts are kept beside it ([`Kept`]), so the field
/// harness's world can be handed back and rebooted as the M8 world.
pub(crate) fn into_field_rig_kept(world: M8World) -> (crate::driving_rig::Rig, Kept) {
    let M8World {
        rig,
        deployment,
        pilot,
        row,
        tvs,
        serial,
    } = world;
    let Rig {
        tw,
        oracle,
        store,
        typed,
        commerce,
        fitted,
        launch,
    } = rig;
    (
        crate::driving_rig::Rig {
            tw,
            cfg: launch_layer().config().clone(),
            oracle: Arc::new(crate::driving_rig::MapOracle(std::sync::Mutex::new(
                std::collections::BTreeMap::new(),
            ))),
            store,
            typed,
            commerce: None,
        },
        Kept {
            oracle,
            commerce,
            fitted,
            launch,
            deployment,
            pilot,
            row,
            tvs,
            serial,
        },
    )
}

/// **What [`into_field_rig_kept`] sets aside** (doc 23 cut D-23-18): the M8 world's own parts,
/// so the field harness's world can be handed back ([`Kept::back`]) and rebooted as the M8 world
/// (its launch, claims and commerce composed), then handed to the field harness again.
pub(crate) struct Kept {
    oracle: Arc<crate::excursions::MapOracle>,
    commerce: Option<crate::excursions::CommerceFactory>,
    fitted: bool,
    launch: Option<crate::excursions::LaunchFactory>,
    deployment: LaunchDeployment,
    pilot: ObjId,
    row: tacsim::port::HullRow,
    tvs: Vec<ObjId>,
    serial: u64,
}

impl Kept {
    /// **The field harness's world, handed back as the M8 world** (doc 23 cut D-23-18).
    pub(crate) fn back(self, field: crate::driving_rig::Rig) -> M8World {
        M8World {
            rig: Rig {
                tw: field.tw,
                oracle: self.oracle,
                store: field.store,
                typed: field.typed,
                commerce: self.commerce,
                fitted: self.fitted,
                launch: self.launch,
            },
            deployment: self.deployment,
            pilot: self.pilot,
            row: self.row,
            tvs: self.tvs,
            serial: self.serial,
        }
    }
}

/// **Where one size's story stands** (doc 23 cut S.4b-3, the S.4b-T stories' cells): the site, the
/// licensed rig's working cell, and the raider seeded there.
struct FieldSite {
    venue: &'static str,
    entry: &'static str,
    cell: (i64, i64),
    wide: bool,
}

/// **Doc 23 cut S.4b-3, the licensed field story** (§17.10's acceptance, one script at both
/// sizes): the claims world on the site, one genuine rating, the paid Pin over two rigs bound to
/// the governed Class II root and titled to the pilot. Through the production command paths:
/// 1. rig 1 `go` opens its claim in the bay and launches under the claim's seal;
/// 2. rig 2's `go` under the same one-seat Pin is refused at `go` with the seat sentence
///    (D-23-16), nothing filed;
/// 3. the raider's leader marks rig 1 Hostile and the field promotes; rig 1 is admitted **under
///    its claim's own seal** — the field's `mind_sealed` line names the filed mind, the Pin's
///    licence and `active`, the claim's seal digest — its retained source installed (a missing
///    source fails the field's spawn), and it flies the field with no `mind_dropped` and no
///    `mind_fault`;
/// 4. the raiders withdraw (`@field/raiders/withdraw`, the field story's own ending), their barge
///    lifts and the field demotes, checking rig 1 in (the check-in writes the hull);
/// 5. `+tv/1 recover go` brings rig 1 home and the claim's Close lands **over the live hull**
///    the field wrote (D-23-15), the seat released: rig 2's `go` now files its own claim.
///
/// **With `carry`, doc 23 cut D-23-18's carried memory** (§17.7) between steps 4 and 5:
/// 4a. the demotion's check-in writes rig 1's mind — its `think_seq`, its memory and no drop,
///     under the claim's seal — onto its open epoch's record, `think_seq` positive and within
///     what the field's span allows;
/// 4b. the server restarts from its store with the epoch open, and the record still carries it;
/// 4c. the raiders land again, mark rig 1, and a second field opens on the same ground and the
///     same epoch: rig 1 is admitted under its claim's seal again and flies it with no drop and
///     no fault;
/// 4d. the raiders withdraw and the second field demotes; its check-in carries a `think_seq`
///     past the carried one by no more than the second field's span allows, and past anything a
///     row starting from nought could reach in that span — so the second field's row started from
///     the carried mind, `think_seq` monotone across the epoch;
/// 5'. the recover closes the epoch, and the carried memory goes with it.
async fn licensed_field_story(at: &FieldSite, carry: bool) {
    use tacsim::arena::ledger::{EventKind, LedgerEvent};
    use tacsim::surface::excursion::ClaimCursor;
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,ic_claim=debug")
        .with_test_writer()
        .try_init();
    let mut world = m8_world_field((at.venue, at.entry), 2);
    let (_, _, listing) = rate_and_list_coprates(&mut world).await;
    let paid = buy(&mut world, listing);
    let (pilot, tv, second_tv) = (world.pilot, world.tvs[0], world.tvs[1]);
    for rig_tv in [tv, second_tv] {
        crate::ic_sortie_surface::fit(
            &mut world.rig,
            rig_tv,
            tacsim::definitions::core_replacement::RIG_II,
        );
        let _ = on_leased(&mut world, rig_tv, |_, _, tv| tv);
    }
    // 1. Rig 1's claim in the bay, then its launch under the claim's seal.
    let (out, queued) = crate::excursions::command(
        &mut world.rig,
        pilot,
        &format!("+tv/1 launch mind=filed:{} go", paid.get()),
    );
    assert_eq!(queued.len(), 1, "one start: {out}");
    crate::excursions::land(&mut world.rig, &queued[0]).expect("the go lands");
    for _ in 0..12 {
        fires(&mut world, 1);
        if crate::excursions::record_of(&mut world.rig.tw, tv).is_some_and(|r| r.act.is_some()) {
            break;
        }
    }
    let _ = crate::excursions::finish(&mut world.rig, tv, pilot);
    let open = match crate::excursions::record_of(&mut world.rig.tw, tv)
        .and_then(|record| record.claim.map(|claim| *claim))
    {
        Some(ClaimCursor::Open { open, .. }) => open,
        other => panic!("rig 1's claim is open: {other:?}"),
    };
    let seal = tacsim::ic_claim_seal(world.rig.tw.world(), open).expect("the claim's seal");
    // 2. D-23-16: the one-seat Pin's seat is held; rig 2 is refused at `go`.
    let refused = crate::ic_sortie_surface::start_refusal(
        &mut world.rig,
        pilot,
        &format!("+tv/2 launch mind=filed:{} go", paid.get()),
    )
    .expect("the second rig is refused");
    assert!(
        refused.contains(
            &tacsim::surface::dispatch::MIND_SEATS_HELD
                .replace("{licence}", &paid.get().to_string())
                .replace("{held}", "1")
                .replace("{seats}", "1")
        ),
        "{refused}"
    );
    assert!(
        crate::excursions::record_of(&mut world.rig.tw, second_tv)
            .is_none_or(|record| record.claim.is_none())
    );
    // 3. The field, over the field harness.
    let (mut rig, kept) = into_field_rig_kept(world);
    crate::driving_rig::to_cell_on(&mut rig, at.entry, tv, at.cell);
    let raider = if at.wide {
        "tacsim.raider.urvara_s4b3"
    } else {
        crate::field_raiders_rig::SKULKS
    };
    let mut def = tacsim::field::raiders::raiders()
        .expect("the fixture raiders")
        .into_iter()
        .find(|def| def.id.as_str() == crate::field_raiders_rig::SKULKS)
        .expect("the content's raider");
    let raiders = if at.wide {
        def.id = crate::driving_rig::cid(raider);
        def.site = crate::driving_rig::cid(at.entry);
        let seeded =
            crate::driving_rig::staff_hand(&mut rig, |ctx| tacsim::field::raiders::seed(ctx, &def));
        assert!(
            seeded.as_deref().is_ok_and(|s| s.contains("seeded")),
            "{seeded:?}"
        );
        let launched = crate::driving_rig::staff_hand(&mut rig, |ctx| {
            tacsim::field::raiders::launch(ctx, &def)
        });
        assert!(launched.is_ok(), "{launched:?}");
        crate::field_raiders_rig::raider_hulls(&rig)
    } else {
        crate::field_raiders_rig::seed_and_launch(&mut rig).0
    };
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    let mut rx = crate::field_host_rig::egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = crate::field_host_rig::second_composed(&mut rig);
    }
    let (out, queued) = crate::driving_rig::command(&mut rig, leader, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land_composed(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
            crate::field_host_rig::task(rig).is_some()
        })
        .await,
        "the field opens on {}",
        at.entry
    );
    let live = tacsim::field::live_on(rig.tw.world(), &crate::driving_rig::cid(at.entry))
        .expect("the field's live record");
    eprintln!(
        "S4B3 {}: units {:?}",
        at.entry,
        live.units
            .iter()
            .map(|row| (row.unit.0, row.party, row.pilot.as_ref().map(|(p, _)| p.0)))
            .collect::<Vec<_>>()
    );
    assert!(
        live.units.iter().any(|row| row.unit == tv),
        "rig 1 is admitted to the field"
    );
    let sealed_line = |lines: &[LedgerEvent]| {
        lines.iter().find_map(|line| match &line.kind {
            EventKind::MindSealed {
                unit,
                mind,
                licence,
                licence_state,
                card_digest,
                ..
            } if *unit == tv => Some((
                mind.clone(),
                licence.clone(),
                *licence_state,
                card_digest.clone(),
            )),
            _ => None,
        })
    };
    assert!(
        crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 300, |_, lines| {
            sealed_line(lines).is_some()
        })
        .await,
        "rig 1's seal line lands on the field's log: {:?}",
        lines
            .iter()
            .filter_map(|line| match &line.kind {
                EventKind::MindSealed {
                    unit,
                    mind,
                    licence,
                    ..
                } => {
                    Some(format!("{}:{mind}:{licence}", unit.0))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    let (mind, licence, state, card) = sealed_line(&lines).expect("the seal line");
    let facts = tacsim::surface::black_box::SealFacts::of(
        &seal,
        tacsim::minds::licence_state::SealLicenceState::Active,
    );
    assert_eq!(mind, facts.mind, "the field admits the claim's own mind");
    assert_eq!(licence.as_str(), paid.get().to_string());
    assert_eq!(
        state,
        Some(tacsim::minds::licence_state::SealLicenceState::Active)
    );
    assert_eq!(card, facts.card_digest, "the claim's card, not the ROM's");
    // It flies the field: twenty seconds with no drop and no fault.
    let _ = crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 20, |_, _| false)
        .await;
    let troubled = lines.iter().any(|line| {
        matches!(&line.kind, EventKind::MindDropped { unit, .. } | EventKind::MindFault { unit, .. } if *unit == tv)
    });
    assert!(!troubled, "the licensed mind neither dropped nor faulted");
    // 4. The raiders withdraw (the field story's own ending), their barge lifts and the field
    // demotes on its quiet interval, checking rig 1 in. A field the fight already ended is over.
    if crate::field_host_rig::task(&rig).is_some() {
        let ordered = crate::driving_rig::run(
            &mut rig.tw,
            GOD,
            &format!("@field/raiders/withdraw {raider}"),
        );
        assert!(ordered.contains("make for the"), "{ordered}");
    }
    assert!(
        crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 600, |rig, _| {
            crate::field_host_rig::task(rig).is_none()
        })
        .await,
        "the field demotes"
    );
    if carry {
        rig = carried_memory(
            rig,
            kept,
            (&mut rx, &mut lines),
            (at, &def),
            (tv, seal.digest()),
        )
        .await;
    } else {
        drop(kept);
    }
    // 5. Home: the Close over the live hull, the seat released. The field's close lands on the
    // site record first (a surface act on a site still promoted waits for it to settle).
    assert!(
        crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
            crate::field_host_rig::site_record_at(rig, at.entry)
                .is_some_and(|record| record.promoted.is_none())
        })
        .await,
        "the field's close lands on the site record"
    );
    // The pilot's own start (a tick's submission may sit beside it in the queue), landed on
    // the composed path; the recover's finish and the claim's Close are the composed ticks'.
    let (out, queued) = crate::driving_rig::command(&mut rig, pilot, "+tv/1 recover go");
    let start = queued
        .iter()
        .find(|submit| submit.actor == pilot)
        .unwrap_or_else(|| panic!("the pilot's recover: {out}"));
    crate::driving_rig::land_composed(&mut rig, start).expect("the recover starts");
    let closed =
        crate::field_host_rig::until_composed(&mut rig, &mut rx, &mut lines, 600, |rig, _| {
            tacsim::ic_claim_closed(rig.tw.world(), open).is_some()
        })
        .await;
    assert!(
        closed,
        "the claim's Close lands over the field-written hull"
    );
    if carry {
        // D-23-18 5': the epoch closed, and its carried memory with it.
        assert!(
            crate::excursions::record_of(&mut rig.tw, tv).is_some_and(|r| r.sortie.is_none()),
            "the closed epoch carries no memory"
        );
    }
    let (out, queued) = crate::driving_rig::command(
        &mut rig,
        pilot,
        &format!("+tv/2 launch mind=filed:{} go", paid.get()),
    );
    assert_eq!(queued.len(), 1, "rig 2's start: {out}");
    crate::driving_rig::land_composed(&mut rig, &queued[0]).expect("the seat is free");
}

/// **Rig 1's carried mind on its open epoch's record** (doc 23 cut D-23-18), once the composed
/// ticks land the field's close: a mind other than `before`, the one an earlier field carried.
async fn carried_on_record(
    rig: &mut crate::driving_rig::Rig,
    (rx, lines): (
        &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
        &mut Vec<tacsim::arena::ledger::LedgerEvent>,
    ),
    tv: ObjId,
    before: Option<&tacsim::arena::mind::CarriedMind>,
) -> tacsim::arena::mind::CarriedMind {
    let mut carried = None;
    let landed = crate::field_host_rig::until_composed(rig, rx, lines, 180, |rig, _| {
        carried = crate::excursions::record_of(&mut rig.tw, tv)
            .and_then(|record| record.sortie)
            .and_then(|open| open.mind);
        carried.as_deref().is_some_and(|mind| Some(mind) != before)
    })
    .await;
    assert!(
        landed,
        "the check-in carries rig 1's mind on its epoch's record: {:?}",
        carried
            .as_deref()
            .map(|mind| (mind.think_seq, mind.memory.len(), mind.dropped))
    );
    *carried.expect("the carried mind")
}

/// **D-23-18's carried memory, steps 4a–4d of [`licensed_field_story`]**: the record after the
/// first field, a restart from the store, a second field on the same epoch, its demotion.
async fn carried_memory(
    mut rig: crate::driving_rig::Rig,
    kept: Kept,
    (rx, lines): (
        &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
        &mut Vec<tacsim::arena::ledger::LedgerEvent>,
    ),
    (at, def): (&FieldSite, &tacsim::field::raiders::RaiderDef),
    (tv, sealed): (ObjId, [u8; 32]),
) -> crate::driving_rig::Rig {
    use tacsim::arena::ledger::EventKind;
    // Rig 1's mind thinks on the governed Class II root's period; a row thinks at most once a
    // period, from its first think.
    let period = tacsim::arena::core::ClassTable::STOCK
        .row(tacsim::arena::core::CoreClass::II)
        .think_ms;
    let most = |span: i64| u64::try_from(span / period).unwrap_or(0) + 2;
    // A field's span, in arena milliseconds: its `match_ended` line's instant.
    let ended = |lines: &[tacsim::arena::ledger::LedgerEvent]| {
        lines
            .iter()
            .rev()
            .find_map(|line| matches!(line.kind, EventKind::MatchEnded { .. }).then_some(line.t_ms))
            .expect("the field's match_ended line")
    };
    // 4a. The demotion's check-in, on the epoch's record.
    let carried = carried_on_record(&mut rig, (rx, lines), tv, None).await;
    let span = ended(lines);
    eprintln!(
        "D23_18 {}: carried think_seq {} memory {} B after a {} ms field",
        at.entry,
        carried.think_seq,
        carried.memory.len(),
        span
    );
    assert!(
        carried.think_seq > 0 && carried.think_seq <= most(span),
        "the first field's mind thought, within its span"
    );
    assert_eq!(carried.seal_digest, sealed, "under the claim's seal");
    assert_eq!(
        carried.dropped, None,
        "a mind that never dropped carries no drop"
    );
    // 4b. A restart from the store, the epoch open.
    let open = crate::excursions::record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .map(|open| open.id)
        .expect("the epoch is open");
    let world = kept.back(rig).reboot();
    let (mut rig, _kept) = into_field_rig_kept(world);
    let reread = crate::excursions::record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch survives the restart");
    assert_eq!(reread.id, open);
    assert_eq!(
        reread.mind.as_deref(),
        Some(&carried),
        "the carried mind survives the restart"
    );
    *rx = crate::field_host_rig::egress(&mut rig);
    lines.clear();
    // 4c. The raiders land again and mark rig 1: a second field on the same epoch.
    // A raider that boarded before the first field demoted lands again; one that had not yet
    // reached its ramp still stands on the ground, and marks from there.
    let landed =
        crate::driving_rig::staff_hand(&mut rig, |ctx| tacsim::field::raiders::launch(ctx, def));
    assert!(
        landed.is_ok()
            || landed
                .as_ref()
                .is_err_and(|why| why.contains("nobody aboard")),
        "{landed:?}"
    );
    let raiders = crate::field_raiders_rig::raider_hulls(&rig);
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    for _ in 0..3 {
        let _ = crate::field_host_rig::second_composed(&mut rig);
    }
    let (out, queued) = crate::driving_rig::command(&mut rig, leader, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "the second promotion: {out}");
    crate::driving_rig::land_composed(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        crate::field_host_rig::until_composed(&mut rig, rx, lines, 120, |rig, _| {
            crate::field_host_rig::task(rig).is_some()
        })
        .await,
        "the second field opens on {}",
        at.entry
    );
    let resealed = |lines: &[tacsim::arena::ledger::LedgerEvent]| {
        lines
            .iter()
            .any(|line| matches!(&line.kind, EventKind::MindSealed { unit, .. } if *unit == tv))
    };
    assert!(
        crate::field_host_rig::until_composed(&mut rig, rx, lines, 300, |_, lines| {
            resealed(lines)
        })
        .await,
        "rig 1 is admitted to the second field under its seal"
    );
    let troubled = lines.iter().any(|line| {
        matches!(&line.kind, EventKind::MindDropped { unit, .. } | EventKind::MindFault { unit, .. } if *unit == tv)
    });
    assert!(!troubled, "the resumed mind neither dropped nor faulted");
    // 4d. The raiders withdraw again; the second field's check-in carries the later mind.
    if crate::field_host_rig::task(&rig).is_some() {
        let ordered = crate::driving_rig::run(
            &mut rig.tw,
            GOD,
            &format!("@field/raiders/withdraw {}", def.id),
        );
        assert!(ordered.contains("make for the"), "{ordered}");
    }
    assert!(
        crate::field_host_rig::until_composed(&mut rig, rx, lines, 600, |rig, _| {
            crate::field_host_rig::task(rig).is_none()
        })
        .await,
        "the second field demotes"
    );
    let later = carried_on_record(&mut rig, (rx, lines), tv, Some(&carried)).await;
    let second = ended(lines);
    eprintln!(
        "D23_18 {}: the second check-in carries think_seq {} memory {} B after a {} ms field",
        at.entry,
        later.think_seq,
        later.memory.len(),
        second
    );
    assert!(
        later.think_seq > carried.think_seq && later.think_seq <= carried.think_seq + most(second),
        "the second field ran on from the carried think_seq: {} after {}",
        later.think_seq,
        carried.think_seq
    );
    assert!(
        later.think_seq > most(second),
        "more thinks than a row starting from nought could take in {second} ms: {}",
        later.think_seq
    );
    assert_eq!(later.seal_digest, sealed);
    rig
}

/// **S.4b-3 at 256** (Ahuna Flank).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "M8 combined-world story: a genuine Registry rating; the CI dispatch runs it"]
async fn s4b3_a_licensed_mind_flies_a_field_at_256() {
    licensed_field_story(
        &FieldSite {
            venue: "ahuna",
            entry: "ahuna_port",
            cell: (190, 126),
            wide: false,
        },
        false,
    )
    .await;
}

/// **S.4b-3 at 512** (Urvara Reach).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "M8 combined-world story: a genuine Registry rating; the CI dispatch runs it"]
async fn s4b3_a_licensed_mind_flies_a_field_at_512() {
    licensed_field_story(
        &FieldSite {
            venue: "urvara",
            entry: "urvara_port",
            cell: (300, 257),
            wide: true,
        },
        false,
    )
    .await;
}

/// **D-23-18 at 256** (Ahuna Flank): the S.4b-3 story with the carried memory between two
/// fields on one epoch, and a restart from the store between them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "M8 combined-world story: a genuine Registry rating; the CI dispatch runs it"]
async fn d23_18_a_licensed_mind_carries_its_memory_between_fields_at_256() {
    licensed_field_story(
        &FieldSite {
            venue: "ahuna",
            entry: "ahuna_port",
            cell: (190, 126),
            wide: false,
        },
        true,
    )
    .await;
}

/// **D-23-18 at 512** (Urvara Reach).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "M8 combined-world story: a genuine Registry rating; the CI dispatch runs it"]
async fn d23_18_a_licensed_mind_carries_its_memory_between_fields_at_512() {
    licensed_field_story(
        &FieldSite {
            venue: "urvara",
            entry: "urvara_port",
            cell: (300, 257),
            wide: true,
        },
        true,
    )
    .await;
}
