//! TacSim domain module composition and adapters.
//!
//! Full server module composition (including host-level and unrelated modules
//! such as `mush-campaign`, `mush-housing`, `mush-mail`, `mush-party`,
//! `mush-privileges`, `mush-scenes`, `mush-script`, `mush-weather`, and
//! `mush-worldgen`) lives in `mush-composition`.

pub mod econ_alerts;
pub mod econ_canon;
pub mod econ_inbox;
pub mod industry_recipes;
pub mod industry_seams;
pub mod mind_commerce;
pub mod salvage_source;
pub mod seat_audit;
pub mod sim_routes;

use std::sync::Arc;

use mush_core::module::GameModule;
use mush_ruleset::Ruleset;

#[must_use]
pub fn domain_modules() -> Vec<Box<dyn GameModule>> {
    domain_modules_with(
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &crate::economy::GameEconomyConfig::default(),
    )
}

#[must_use]
pub fn domain_modules_with(
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    game: &crate::economy::GameEconomyConfig,
) -> Vec<Box<dyn GameModule>> {
    let composed = compose_domain_modules(
        economy,
        atlas,
        game,
        TacsimComposition::Absent,
        Ok::<_, std::convert::Infallible>,
    );
    match composed {
        Ok((modules, _inventory)) => modules,
        Err(never) => match never {},
    }
}

#[must_use]
pub fn domain_modules_with_tacsim() -> (
    Vec<Box<dyn GameModule>>,
    mush_inventory::InventoryHandleCell,
) {
    domain_modules_with_economy(
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &crate::economy::GameEconomyConfig::default(),
    )
}

#[must_use]
pub fn domain_modules_with_economy(
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    game: &crate::economy::GameEconomyConfig,
) -> (
    Vec<Box<dyn GameModule>>,
    mush_inventory::InventoryHandleCell,
) {
    let composed = compose_domain_modules(
        economy,
        atlas,
        game,
        TacsimComposition::Standard,
        Ok::<_, std::convert::Infallible>,
    );
    match composed {
        Ok(composed) => composed,
        Err(never) => match never {},
    }
}

pub fn domain_modules_with_mind_commerce(
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    game: &crate::economy::GameEconomyConfig,
    commerce: &mind_commerce::MindCommerce,
) -> Result<
    (
        Vec<Box<dyn GameModule>>,
        mush_inventory::InventoryHandleCell,
    ),
    mush_core::ModError,
> {
    compose_domain_modules(
        economy,
        atlas,
        game,
        TacsimComposition::Commerce(commerce),
        |module| commerce.configure_economy(module),
    )
}

pub fn domain_modules_with_launch(
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    game: &crate::economy::GameEconomyConfig,
    launch: &crate::launch::LaunchComposition,
) -> Result<
    (
        Vec<Box<dyn GameModule>>,
        mush_inventory::InventoryHandleCell,
    ),
    mush_core::ModError,
> {
    let commerce = launch.commerce();
    compose_domain_modules(
        economy,
        atlas,
        game,
        TacsimComposition::Launch(launch),
        |module| commerce.configure_economy(module),
    )
}

#[derive(Clone, Copy)]
enum TacsimComposition<'a> {
    Absent,
    Standard,
    Commerce(&'a mind_commerce::MindCommerce),
    Launch(&'a crate::launch::LaunchComposition),
}

fn compose_domain_modules<E>(
    mut economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    game: &crate::economy::GameEconomyConfig,
    composition: TacsimComposition<'_>,
    configure_economy: impl FnOnce(
        mush_economy::EconomyModule,
    ) -> Result<mush_economy::EconomyModule, E>,
) -> Result<
    (
        Vec<Box<dyn GameModule>>,
        mush_inventory::InventoryHandleCell,
    ),
    E,
> {
    let (tacsim_delegate, commerce, launch) = match composition {
        TacsimComposition::Absent => (false, None, None),
        TacsimComposition::Standard => (true, None, None),
        TacsimComposition::Commerce(commerce) => (true, Some(commerce), None),
        TacsimComposition::Launch(launch) => (true, Some(launch.commerce()), Some(launch)),
    };
    economy
        .instruments
        .entry(crate::economy::CREDIT.to_owned())
        .or_insert(mush_economy::InstrumentCaps {
            mint_obligation_cap_major: crate::port::PORT_MINT_CAP_MAJOR,
        });
    let housing_on = game.housing;
    let mut issuers: Vec<mush_org::SystemPrincipalId> = if housing_on {
        vec![housing_title_issuer()]
    } else {
        Vec::new()
    };
    let market_on = game.market;
    if market_on {
        issuers.push(market_title_issuer());
    }
    issuers.push(mush_industry::industry_title_issuer());
    issuers.extend(mush_org::SystemPrincipalId::new(
        crate::port::hull::REGISTRY_ISSUER,
    ));
    if let Some(commerce) = commerce {
        let selected = commerce.filing_issuer();
        if !issuers.contains(selected) {
            issuers.push(selected.clone());
        }
    }
    let market_issuer = market_on.then(market_title_issuer);
    let inventory_handles: mush_inventory::InventoryHandleCell = Arc::default();
    let industry_handles: mush_industry::IndustryHandleCell = Arc::default();
    let mut delegates: Vec<&'static str> = vec!["industry", "market"];
    if tacsim_delegate {
        delegates.push(crate::NS);
    }
    let simulation = mush_sim::SimulationModule::new()
        .with_route_catalog(Arc::new(self::sim_routes::TacsimRouteCatalog))
        .with_carrier_hooks(Arc::new(self::sim_routes::TacsimCarrierHooks));
    let tactics = match launch {
        Some(launch) => launch.tactics(),
        None => mush_tactics::TacticsModule::with_catalog(tactics_catalog()),
    }
    .with_sensor_policy(Arc::new(mush_propagation::PropagationSensorPolicy));
    let (tactics, simulation) = match commerce {
        Some(c) => c.configure_ic_physical(tactics, simulation),
        None => (tactics, simulation),
    };
    let unit_authority = domain_unit_authority(tacsim_delegate);
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_comsys::ComsysModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_org::OrgModule::with_predicates(vec![std::sync::Arc::new(
                mush_faction::FactionStandingProvider,
            )])
            .with_capabilities(domain_org_capability_vocabulary())
            .with_scope_use(vec![std::sync::Arc::new(mush_economy::EconomyScopeUse)])
            .with_unit_authority(unit_authority),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(simulation),
        atlas_module(atlas),
        Box::new(tactics),
        Box::new(
            mush_propagation::PropagationModule::new()
                .with_metric(Arc::new(crate::relay_metric::RelayMetric))
                .with_service(Arc::new(crate::relay_service::RelayService::new())),
        ),
        Box::new({
            let module = mush_economy::EconomyModule::with_custody(vec![
                Arc::new(mush_sim::InteriorCustodyProvider),
                Arc::new(mush_industry::IndustryCustodyProvider),
            ])
            .with_config(economy)
            .with_facts({
                let mut facts: Vec<Arc<dyn mush_economy::FactProvider>> = vec![
                    Arc::new(mush_jobs::JobsFactProvider),
                    Arc::new(mush_inventory::InventoryFactProvider),
                    Arc::new(mush_industry::IndustryFactProvider),
                    Arc::new(mush_sim::SimFactProvider),
                    Arc::new(mush_market::MarketFactProvider),
                ];
                if tacsim_delegate {
                    facts.push(Arc::new(
                        crate::surface::survey_facts::SurveyFactProvider::default(),
                    ));
                    facts.push(Arc::new(
                        crate::port::passenger::facts::PassengerFactProvider::default(),
                    ));
                }
                facts
            })
            .with_observers(domain_econ_observers())
            .with_title_issuers(issuers)
            .with_canon_evidence(Arc::new(self::econ_canon::CanonEvidenceReader))
            .with_party_caps(Vec::new())
            .with_party_cap_source_if(tacsim_delegate)
            .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard));
            let module = configure_economy(module)?;
            match launch {
                Some(launch) => launch.configure_economy(module),
                None => module,
            }
        }),
        Box::new({
            let module = mush_inventory::InventoryModule::new()
                .with_quantity_delegates(delegates)
                .with_salvage_source(Arc::new(self::salvage_source::SimSalvageSource))
                .with_handle_cell(Arc::clone(&inventory_handles));
            match launch {
                Some(launch) => launch.configure_inventory(module),
                None => module,
            }
        }),
        Box::new({
            let module = mush_industry::IndustryModule::new()
                .with_inventory(Arc::clone(&inventory_handles))
                .with_recipes(Arc::new(self::industry_recipes::TacsimRecipeAuthority))
                .with_licence_if(tacsim_delegate)
                .with_handle_cell(Arc::clone(&industry_handles))
                .with_surface_seams_if(tacsim_delegate, &industry_handles);
            match launch {
                Some(launch) => launch.configure_industry(module),
                None => module,
            }
        }),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new({
            let module = mush_roll::RollModule::with_writers(tacsim_delegate.then_some(crate::NS));
            match commerce {
                Some(commerce) => commerce.configure_roll(module),
                None => module,
            }
        }),
        Box::new({
            let m = mush_market::MarketModule::new()
                .with_catalog_sources(Vec::new())
                .with_availability(Vec::new())
                .with_delivery(Vec::new())
                .with_inventory(Arc::clone(&inventory_handles))
                .with_port_seams_if(tacsim_delegate);
            let m = match market_issuer {
                Some(i) => m.with_issuer(i),
                None => m,
            };
            let m = match commerce {
                Some(commerce) => commerce.configure_market(m),
                None => m,
            };
            match launch {
                Some(launch) => launch.configure_market(m),
                None => m,
            }
        }),
    ];
    Ok((modules, Arc::clone(&inventory_handles)))
}

trait PortPartyCapsExt: Sized {
    fn with_party_cap_source_if(self, on: bool) -> Self;
}

impl PortPartyCapsExt for mush_economy::EconomyModule {
    fn with_party_cap_source_if(self, on: bool) -> Self {
        if on {
            self.with_party_cap_source(Arc::new(crate::port::market::PortPartyCaps))
        } else {
            self
        }
    }
}

trait PortMarketSeamsExt: Sized {
    fn with_port_seams_if(self, on: bool) -> Self;
}

impl PortMarketSeamsExt for mush_market::MarketModule {
    fn with_port_seams_if(self, on: bool) -> Self {
        if on {
            self.with_venue_gate(Arc::new(crate::port::market::PortVenueGate))
                .with_capacity_provider(Arc::new(crate::port::market::PortCapacity))
                .with_proceeds_policy(Arc::new(crate::port::proceeds::CoopProceeds))
        } else {
            self
        }
    }
}

trait IndustryLicenceExt: Sized {
    fn with_licence_if(self, on: bool) -> Self;
}

impl IndustryLicenceExt for mush_industry::IndustryModule {
    fn with_licence_if(self, on: bool) -> Self {
        match (on, crate::economy::licence_instrument()) {
            (true, Some(id)) => self.with_extraction_licence(id),
            _ => self,
        }
    }
}

trait IndustrySurfaceExt: Sized {
    fn with_surface_seams_if(self, on: bool, cell: &mush_industry::IndustryHandleCell) -> Self;
}

impl IndustrySurfaceExt for mush_industry::IndustryModule {
    fn with_surface_seams_if(self, on: bool, cell: &mush_industry::IndustryHandleCell) -> Self {
        if on {
            self.with_system_opener(Arc::new(industry_seams::SiteOpener::new(Arc::clone(cell))))
                .with_external_draws(Arc::new(industry_seams::SurfaceDraws))
                .with_tenure_authority(Arc::new(industry_seams::SurfaceTenure::new()))
                .with_state_delegates([crate::NS])
        } else {
            self
        }
    }
}

fn domain_econ_observers() -> Vec<Arc<dyn mush_economy::EconomyObserver>> {
    vec![
        Arc::new(self::econ_canon::CanonEvidenceBridge),
        Arc::new(self::econ_inbox::OrgInbox),
        Arc::new(self::econ_alerts::StaffAlerts::default()),
        Arc::new(mush_market::MarketObserver),
        Arc::new(crate::port::coop::CoopObserver),
    ]
}

/// DT.2a: the shared grant owner reads actual Economy title/control facts.
/// OOC practice membership supplies no IC mission-role authority.
#[must_use]
pub fn domain_unit_authority(
    tacsim_delegate: bool,
) -> mush_org::unit_authority::UnitAuthorityConfig {
    let mut unit_requesters: std::collections::BTreeSet<_> = ["org", "economy", "tactics"]
        .into_iter()
        .map(|name| mush_core::mechanics::OwnerId::new(name).expect("static owner id"))
        .collect();
    if tacsim_delegate {
        unit_requesters
            .insert(mush_core::mechanics::OwnerId::new(crate::NS).expect("static Tacsim owner id"));
    }
    mush_org::unit_authority::UnitAuthorityConfig {
        facts: Arc::new(mush_economy::unit_authority::EconomyUnitAuthorityWorldFacts),
        requesters: unit_requesters,
        dependencies: std::collections::BTreeSet::from([mush_core::mechanics::OwnerId::new(
            "economy",
        )
        .expect("static Economy owner id")]),
    }
}

/// The system principal housing mints property titles under (08d §2, D-E3-13).
#[must_use]
pub fn housing_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("tacsim.housing")
        .expect("a built-in system principal id is well-formed")
}

/// The system principal the market mints a stocked asset's title under (E6c B11,
/// §2 item 8, D-E3-13).
#[must_use]
pub fn market_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("tacsim.market")
        .expect("a built-in system principal id is well-formed")
}

/// 17g C1b: the capabilities the named launch's governed hardware authorizes
/// (`mush_industry::core_production`'s die issue on the importer's stock scope,
/// and integration and refit on its factory scope). Doc 17h cut M8b (G8): and the ungoverning
/// refit's operator capability on its shop's scope (`RefitKind::CoreUngovern`), so a
/// deployment that composes a shop can grant it.
pub const NAMED_LAUNCH_HARDWARE_CAPABILITIES: [&str; 4] = [
    mush_industry::core_production::IMPORT_CAPABILITY,
    "industry.core.integrate",
    "industry.core.refit",
    "industry.core.ungovern",
];

/// Domain capability vocabulary recognized by TacSim and its economy/industry/roll
/// substrates. `mush-composition::org_capability_vocabulary` extends this with
/// host-level housing and campaign capabilities.
#[must_use]
pub fn domain_org_capability_vocabulary() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain([
            "unit.operate_crew",
            "unit.deploy",
            "unit.command_assigned_ai",
            "unit.recover_load",
            "unit.maintain_refit",
            "unit.transfer_abandon_title",
        ])
        .chain([
            "org.role.manage",
            "org.grant.delegate",
            "relay.manage",
            "relay.discover",
            "relay.use",
            "relay.speak",
            "relay.inspect",
            "relay.publish",
        ])
        .chain([mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY])
        .chain([mush_roll::commerce::registry_recall::REGISTRY_RECALL_CAPABILITY])
        .chain([mush_roll::commerce::registry_revocation::REGISTRY_REVOCATION_CAPABILITY])
        // Doc 17h cut M8a: the Registry inspection, the third twin.
        .chain([mush_roll::commerce::registry_inspection::REGISTRY_INSPECTION_CAPABILITY])
        // Doc 17h §3 M8c (B2): a character acting for an organisation that holds
        // a mind licence (`org_holder::org_actor`).
        .chain([mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY])
        // Doc 17h §3 M8c (F7): the Registry's seal-mark writer
        // (`seal_marks::authorized`); M8e's black-box participant writes.
        .chain([mush_roll::commerce::seal_marks::SEAL_MARK_CAPABILITY])
        .chain(NAMED_LAUNCH_HARDWARE_CAPABILITIES)
        .map(|c| mush_org::CapabilityId::new(c).expect("a built-in capability id is well-formed"))
        .collect()
}

fn rules_interface_for<R: mush_ruleset::Ruleset>(
    rs: &R,
) -> mush_ruleset::interface::RulesInterfaceModule {
    let stages = rs.chargen().map_or_else(Vec::new, |c| c.stages());
    mush_ruleset::interface::RulesInterfaceModule::new(rs.display_name().to_string(), stages)
}

#[must_use]
pub fn sheet_modules(
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
) -> Vec<Box<dyn GameModule>> {
    let rs = crate::sheet::PilotRuleset::new()
        .onboarding(onboarding.clone())
        .economy(game.clone());
    let rules_interface = rules_interface_for(&rs);
    vec![Box::new(rs), Box::new(rules_interface)]
}

#[must_use]
pub fn tacsim_module(cfg: crate::TacsimConfig) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, false))
}

#[must_use]
pub fn tacsim_module_with(cfg: crate::TacsimConfig, game_content: bool) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, game_content))
}

#[must_use]
pub fn tacsim_test_world_module(
    cfg: crate::TacsimConfig,
    game_content: bool,
) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, game_content).with_test_world(true))
}

#[must_use]
pub fn tacsim_module_wired(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
) -> Box<dyn GameModule> {
    Box::new(
        crate::TacsimRuleset::with_game_content(cfg, game_content)
            .with_inventory(inventory)
            .with_unit_authority(),
    )
}

pub fn tacsim_module_wired_with_mind_commerce(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
    commerce: &mind_commerce::MindCommerce,
) -> Result<Box<dyn GameModule>, mush_core::ModError> {
    let module = crate::TacsimRuleset::with_game_content(cfg, game_content)
        .with_inventory(inventory)
        .with_unit_authority();
    Ok(Box::new(commerce.configure_tacsim(module)?))
}

#[must_use]
pub fn tacsim_test_world_module_wired(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
) -> Box<dyn GameModule> {
    Box::new(
        crate::TacsimRuleset::with_game_content(cfg, game_content)
            .with_inventory(inventory)
            .with_unit_authority()
            .with_test_world(true),
    )
}

#[must_use]
pub fn tactics_catalog() -> mush_tactics::UnitCatalog {
    match crate::builtin_definitions() {
        Ok(catalog) => catalog.units,
        Err(error) => {
            tracing::error!(%error, "tacsim: the shipped unit content did not bind");
            mush_tactics::UnitCatalog::default()
        }
    }
}

#[must_use]
pub fn atlas_module(cfg: mush_atlas::AtlasConfig) -> Box<dyn GameModule> {
    Box::new(atlas_module_builder(cfg))
}

/// The real configured Atlas module before boxing, so the outer composition
/// can add owner capabilities without duplicating the authored pack shelf.
#[must_use]
pub fn atlas_module_builder(cfg: mush_atlas::AtlasConfig) -> mush_atlas::AtlasModule {
    let mut shelf = mush_atlas::starter::packs().unwrap_or_else(|error| {
        tracing::error!(%error, "atlas: the shipped starter packs did not load");
        Vec::new()
    });
    match crate::definitions::content::boot_packs() {
        Ok(game) => shelf.extend(game),
        Err(error) => tracing::error!(%error, "atlas: the game's boot packs did not load"),
    }
    match crate::definitions::content::station_packs() {
        Ok(stations) => shelf.extend(stations),
        Err(error) => tracing::error!(%error, "atlas: the station packs did not load"),
    }
    match crate::definitions::content::lane_packs() {
        Ok(lane) => shelf.extend(lane),
        Err(error) => tracing::error!(%error, "atlas: the lane pack did not load"),
    }
    match crate::relay_content::packs() {
        Ok(packs) => shelf.extend(packs),
        Err(error) => tracing::error!(%error, "relay shelf pack refused"),
    }
    match crate::port::passenger::content::packs() {
        Ok(packs) => shelf.extend(packs),
        Err(error) => tracing::error!(%error, "passenger shelf pack refused"),
    }
    match crate::quest::packs::packs() {
        Ok(packs) => shelf.extend(packs),
        Err(error) => tracing::error!(%error, "starter quest shelf packs refused"),
    }
    mush_atlas::AtlasModule::with_config(cfg).with_shelf(shelf)
}

pub fn content_module(starter_area: bool) -> Result<Box<dyn GameModule>, String> {
    content_module_with(starter_area, false)
}

pub fn boot_pack_list(
    starter_area: bool,
    game_content: bool,
) -> Result<Vec<mush_content::Pack>, String> {
    let mut packs = Vec::new();
    if starter_area {
        packs.extend(mush_atlas::starter::packs().map_err(|e| format!("starter packs: {e}"))?);
    }
    if game_content {
        packs.extend(crate::definitions::content::boot_packs()?);
        packs.extend(crate::definitions::content::station_packs()?);
        packs.extend(crate::definitions::content::lane_packs()?);
        packs.extend(crate::relay_content::packs()?);
        packs.extend(crate::port::passenger::content::packs()?);
        packs.extend(crate::quest::packs::packs()?);
    }
    Ok(packs)
}

pub fn content_module_with(
    starter_area: bool,
    game_content: bool,
) -> Result<Box<dyn GameModule>, String> {
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).map_err(|e| format!("atlas appliers: {e}"))?;
    mush_npc::content::register_appliers(&mut appliers)
        .map_err(|e| format!("npc appliers: {e}"))?;
    crate::register_appliers(&mut appliers).map_err(|e| format!("tacsim appliers: {e}"))?;
    let mut module = mush_content::ContentModule::new().with_appliers(appliers);
    let packs = boot_pack_list(starter_area, game_content)?;
    if game_content {
        for pack in crate::relay_content::packs()? {
            module = module.wait_for_dependency_upgrades(pack.manifest().id().clone());
        }
        for pack in crate::port::passenger::content::packs()? {
            module = module.wait_for_dependency_upgrades(pack.manifest().id().clone());
        }
        for pack in crate::quest::packs::packs()? {
            module = module.wait_for_dependency_upgrades(pack.manifest().id().clone());
        }
    }
    if !packs.is_empty() {
        module = module.with_boot_packs(packs);
    }
    Ok(Box::new(module))
}

#[must_use]
pub fn npc_module(grounding: Option<Arc<dyn mush_npc::AgentGrounding>>) -> Box<dyn GameModule> {
    let module = match grounding {
        Some(g) => mush_npc::NpcModule::with_grounding(g),
        None => mush_npc::NpcModule::new(),
    };
    Box::new(module)
}

#[must_use]
pub fn npc_agent_adapter() -> Option<Arc<dyn mush_npc::AgentGrounding>> {
    Some(Arc::new(RulesetAgent::new(
        crate::sheet::PilotRuleset::new(),
    )))
}

pub struct RulesetAgent<R: Ruleset> {
    ruleset: R,
    schema: Arc<mush_ruleset::schema::SheetSchema>,
}

impl<R: Ruleset> RulesetAgent<R> {
    #[must_use]
    pub fn new(ruleset: R) -> RulesetAgent<R> {
        let schema = Arc::new(ruleset.sheet_schema().clone());
        RulesetAgent { ruleset, schema }
    }
}

impl<R: Ruleset + Send + Sync + 'static> mush_npc::AgentGrounding for RulesetAgent<R> {
    fn profile(
        &self,
        w: &mush_core::World,
        npc: mush_core::ObjId,
    ) -> Option<mush_ruleset::AgentCapabilityProfile> {
        let doc = mush_ruleset::SheetStore::new(Arc::clone(&self.schema)).get(w, npc)?;
        Some(self.ruleset.agent_profile(&doc))
    }

    fn invoke_ability(
        &self,
        w: &mut mush_core::World,
        out: &mut mush_core::Outbox,
        npc: mush_core::ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<mush_ruleset::AbilityOutcome, mush_ruleset::AbilityError> {
        self.ruleset.invoke_ability(w, out, npc, name, input)
    }

    fn projected_ability(
        &self,
        w: &mush_core::World,
        npc: mush_core::ObjId,
        name: &str,
        input: &serde_json::Value,
    ) -> Option<mush_ruleset::ProjectedAbility> {
        self.ruleset.projected_ability(w, npc, name, input)
    }

    fn init_sheet(
        &self,
        w: &mut mush_core::World,
        npc: mush_core::ObjId,
        template: &str,
        by: mush_core::ObjId,
    ) -> Result<String, String> {
        self.ruleset.init_sheet(w, npc, template, by)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    Narrowed,
    Whole,
}

pub const SCHEMA_KEY: &str = "schema";
