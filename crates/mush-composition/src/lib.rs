//! Production module composition root.
//!
//! Wires the built-in server modules, cross-module adapters, and TacSim domain
//! modules into the server boot order, keeping full-server module dependencies
//! (`mush-campaign`, `mush-housing`, `mush-mail`, `mush-party`,
//! `mush-privileges`, `mush-scenes`, `mush-script`, `mush-weather`,
//! `mush-worldgen`) out of `tacsim`.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod campaign_formations;
pub mod mail_transport;

use std::sync::Arc;

use mush_core::module::GameModule;
use mush_llm::LlmService;
use mush_worldgen::WorldgenModule;
use smol_str::SmolStr;

pub use tacsim::composition::{
    Fetch, NAMED_LAUNCH_HARDWARE_CAPABILITIES, RulesetAgent, SCHEMA_KEY, atlas_module,
    boot_pack_list, content_module, content_module_with, econ_alerts, econ_canon, econ_inbox,
    housing_title_issuer, industry_recipes, industry_seams, market_title_issuer, mind_commerce,
    npc_agent_adapter, npc_module, salvage_source, seat_audit, sheet_modules, sim_routes,
    tacsim_module, tacsim_module_wired, tacsim_module_wired_with_mind_commerce, tacsim_module_with,
    tacsim_test_world_module, tacsim_test_world_module_wired,
};

/// Construct the full built-in module list in dependency order, with crate-default
/// weather, economy, atlas and onboarding configs and no rulesets enabled.
#[must_use]
pub fn server_modules() -> Vec<Box<dyn GameModule>> {
    server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &mush_core::OnboardingConfig::default(),
        &tacsim::economy::GameEconomyConfig::default(),
        no_recovery(),
    )
}

/// [`server_modules`] with explicit weather, economy, atlas, onboarding and
/// game-economy configs, so `mushd.toml`'s `[weather]`, `[economy]`, `[atlas]`,
/// `[onboarding]` and `[tacsim.economy]` sections reach their modules.
#[must_use]
pub fn server_modules_with(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &tacsim::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
) -> Vec<Box<dyn GameModule>> {
    let composed = compose_server_modules(
        weather,
        economy,
        atlas,
        onboarding,
        game,
        recovery,
        TacsimComposition::Absent,
        Ok::<_, std::convert::Infallible>,
    );
    match composed {
        Ok((modules, _inventory)) => modules,
        Err(never) => match never {},
    }
}

/// [`server_modules_with`] for a roster that ALSO composes the TacSim layer
/// (doc 18 cut IC5; §4.4, D-18-33): `mush-inventory` delegates its quantity
/// families to `"tacsim"` as well as `"industry"` and `"market"`, and the
/// inventory handle cell is returned so [`tacsim_module_wired`] (or
/// [`tacsim_test_world_module_wired`]) can give it to the layer.
#[must_use]
pub fn server_modules_with_tacsim(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &tacsim::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
) -> (
    Vec<Box<dyn GameModule>>,
    mush_inventory::InventoryHandleCell,
) {
    let composed = compose_server_modules(
        weather,
        economy,
        atlas,
        onboarding,
        game,
        recovery,
        TacsimComposition::Standard,
        Ok::<_, std::convert::Infallible>,
    );
    match composed {
        Ok(composed) => composed,
        Err(never) => match never {},
    }
}

/// The existing full built-in roster with an explicit, prepared Registry and
/// commerce configuration.
pub fn server_modules_with_tacsim_and_mind_commerce(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &tacsim::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
    commerce: &mind_commerce::MindCommerce,
) -> Result<
    (
        Vec<Box<dyn GameModule>>,
        mush_inventory::InventoryHandleCell,
    ),
    mush_core::ModError,
> {
    compose_server_modules(
        weather,
        economy,
        atlas,
        onboarding,
        game,
        recovery,
        TacsimComposition::Commerce(commerce),
        |module| commerce.configure_economy(module),
    )
}

/// The full built-in roster configured from a validated launch deployment.
pub fn server_modules_with_launch(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &tacsim::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
    launch: &tacsim::launch::LaunchComposition,
) -> Result<
    (
        Vec<Box<dyn GameModule>>,
        mush_inventory::InventoryHandleCell,
    ),
    mush_core::ModError,
> {
    let commerce = launch.commerce();
    compose_server_modules(
        weather,
        economy,
        atlas,
        onboarding,
        game,
        recovery,
        TacsimComposition::Launch(launch),
        |module| commerce.configure_economy(module),
    )
}

#[derive(Clone, Copy)]
enum TacsimComposition<'a> {
    Absent,
    Standard,
    Commerce(&'a mind_commerce::MindCommerce),
    Launch(&'a tacsim::launch::LaunchComposition),
}

#[allow(clippy::too_many_arguments)]
fn compose_server_modules<E>(
    weather: mush_weather::WeatherConfig,
    mut economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &tacsim::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
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
        .entry(tacsim::economy::CREDIT.to_owned())
        .or_insert(mush_economy::InstrumentCaps {
            mint_obligation_cap_major: tacsim::port::PORT_MINT_CAP_MAJOR,
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
        tacsim::port::hull::REGISTRY_ISSUER,
    ));
    if let Some(commerce) = commerce {
        let selected = commerce.filing_issuer();
        if !issuers.contains(selected) {
            issuers.push(selected.clone());
        }
    }
    let issuer = housing_on.then(housing_title_issuer);
    let market_issuer = market_on.then(market_title_issuer);
    let inventory_handles: mush_inventory::InventoryHandleCell = Arc::default();
    let industry_handles: mush_industry::IndustryHandleCell = Arc::default();
    let mut delegates: Vec<&'static str> = vec!["industry", "market"];
    if tacsim_delegate {
        delegates.push(tacsim::NS);
    }
    let reserved: Vec<mush_core::ObjId> = [
        onboarding.start_room,
        onboarding.start_room_unapproved,
        mush_core::LIMBO,
    ]
    .into_iter()
    .chain(game.reserved_rooms.iter().copied())
    .collect();
    let scene_movement_receipts = Arc::default();
    let simulation = mush_sim::SimulationModule::new()
        .with_atomic_scene_movement(Arc::clone(&scene_movement_receipts))
        .with_route_catalog(Arc::new(self::sim_routes::TacsimRouteCatalog))
        .with_carrier_hooks(Arc::new(self::sim_routes::TacsimCarrierHooks));
    let tactics = match launch {
        Some(launch) => launch.tactics(),
        None => mush_tactics::TacticsModule::with_catalog(tacsim::composition::tactics_catalog()),
    }
    .with_sensor_policy(Arc::new(mush_propagation::PropagationSensorPolicy));
    let (tactics, simulation) = match commerce {
        Some(c) => c.configure_ic_physical(tactics, simulation),
        None => (tactics, simulation),
    };
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(
            mush_comsys::ComsysModule::new()
                .with_transport(Arc::new(mail_transport::MailTransport)),
        ),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_privileges::PrivilegesModule::new()),
        Box::new(
            mush_org::OrgModule::with_predicates(vec![std::sync::Arc::new(
                mush_faction::FactionStandingProvider,
            )])
            .with_capabilities(org_capability_vocabulary())
            .with_scope_use(vec![std::sync::Arc::new(mush_economy::EconomyScopeUse)])
            .with_unit_authority(tacsim::composition::domain_unit_authority(tacsim_delegate)),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(
            mush_scenes::ScenesModule::new()
                .with_atomic_movement(scene_movement_receipts)
                .with_sealer_gate(Arc::new(
                    |w: &mush_core::World, run: u64, who: mush_core::ObjId| {
                        mush_economy::stands_in_contract(w, mush_economy::ContractId(run), who)
                    },
                )),
        ),
        Box::new(simulation),
        atlas_module(atlas),
        Box::new(tactics),
        Box::new(
            mush_propagation::PropagationModule::new()
                .with_dispatch_sources(Arc::new(mail_transport::MailTransport))
                .with_metric(Arc::new(tacsim::relay_metric::RelayMetric))
                .with_service(Arc::new(
                    tacsim::relay_service::RelayService::new()
                        .with_mail_visibility(Arc::new(mail_transport::mail_receipt_visible)),
                )),
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
                    Arc::new(mush_campaign::CampaignFactProvider),
                    Arc::new(mush_sim::SimFactProvider),
                    Arc::new(mush_market::MarketFactProvider),
                ];
                if tacsim_delegate {
                    facts.push(Arc::new(
                        tacsim::surface::survey_facts::SurveyFactProvider::default(),
                    ));
                    facts.push(Arc::new(
                        tacsim::port::passenger::facts::PassengerFactProvider::default(),
                    ));
                }
                facts
            })
            .with_observers(econ_observers())
            .with_title_issuers(issuers)
            .with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_scenes::KIND_SCENE_OUTCOME),
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ])
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
        Box::new(
            mush_campaign::CampaignModule::new()
                .with_formation_kinds(Arc::new(
                    self::campaign_formations::TacsimFormationAuthority,
                ))
                .with_seat_audit(Arc::new(self::seat_audit::SeatAuditLog::new()))
                .with_recovery(recovery),
        ),
        Box::new(mush_mail::MailModule.with_transport(Arc::new(mail_transport::MailTransport))),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new({
            let m = mush_housing::HousingModule::new().with_reserved_rooms(reserved);
            match issuer {
                Some(i) => m.with_issuer(i),
                None => m,
            }
        }),
        Box::new(mush_weather::WeatherModule::with_config(weather)),
        Box::new(mush_party::PartyModule::new()),
        Box::new({
            let module = mush_roll::RollModule::with_writers(tacsim_delegate.then_some(tacsim::NS));
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
        Box::new(mush_script::ScriptModule::new()),
    ];
    Ok((modules, Arc::clone(&inventory_handles)))
}

trait PortPartyCapsExt: Sized {
    fn with_party_cap_source_if(self, on: bool) -> Self;
}

impl PortPartyCapsExt for mush_economy::EconomyModule {
    fn with_party_cap_source_if(self, on: bool) -> Self {
        if on {
            self.with_party_cap_source(Arc::new(tacsim::port::market::PortPartyCaps))
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
            self.with_venue_gate(Arc::new(tacsim::port::market::PortVenueGate))
                .with_capacity_provider(Arc::new(tacsim::port::market::PortCapacity))
                .with_proceeds_policy(Arc::new(tacsim::port::proceeds::CoopProceeds))
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
        match (on, tacsim::economy::licence_instrument()) {
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
                .with_state_delegates([tacsim::NS])
        } else {
            self
        }
    }
}

fn econ_observers() -> Vec<Arc<dyn mush_economy::EconomyObserver>> {
    vec![
        Arc::new(self::econ_canon::CanonEvidenceBridge),
        Arc::new(mush_housing::HousingEconomyObserver),
        Arc::new(self::econ_inbox::OrgInbox),
        Arc::new(self::econ_alerts::StaffAlerts::default()),
        Arc::new(mush_market::MarketObserver),
        Arc::new(tacsim::port::coop::CoopObserver),
    ]
}

#[must_use]
pub fn org_capability_vocabulary() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    let mut set = tacsim::composition::domain_org_capability_vocabulary();
    for &c in mush_housing::CAPABILITIES
        .iter()
        .chain(mush_campaign::CAPABILITIES.iter())
    {
        if let Some(id) = mush_org::CapabilityId::new(c) {
            set.insert(id);
        }
    }
    set
}

#[must_use]
pub fn no_recovery() -> mush_campaign::RecoveryConfig {
    mush_campaign::RecoveryConfig::default()
}

#[must_use]
pub fn recovery_policy(cfg: &tacsim::TacsimRecoveryConfig) -> mush_campaign::RecoveryConfig {
    mush_campaign::RecoveryConfig {
        enabled: cfg.enabled,
        pool_account: mush_economy::AccountId(cfg.pool_account),
        pool_floor: cfg.pool_floor,
        tiers: cfg.tiers.iter().map(SmolStr::new).collect(),
        applications_per_principal: cfg.applications_per_principal,
        cooldown_secs: cfg.cooldown_secs,
        max_operational_assets: cfg.max_operational_assets,
        insurance: SmolStr::new(&cfg.insurance),
        upkeep_per_period: cfg.upkeep_per_period,
    }
}

#[must_use]
pub fn worldgen_module(llm: Arc<LlmService>) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::new(llm))
}

#[must_use]
pub fn worldgen_module_with(
    llm: Arc<LlmService>,
    config: mush_worldgen::WorldgenConfig,
) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::with_config(llm, config))
}

#[must_use]
pub fn npc_module_with(
    grounding: Option<Arc<dyn mush_npc::AgentGrounding>>,
    npc: &mush_config::config::NpcCfg,
) -> Box<dyn GameModule> {
    let module = match grounding {
        Some(g) => mush_npc::NpcModule::with_grounding(g),
        None => mush_npc::NpcModule::new(),
    };
    Box::new(module.with_scene_turn_deadline_secs(i64::from(npc.scene_turn_deadline_secs)))
}
