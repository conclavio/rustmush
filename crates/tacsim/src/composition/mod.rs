//! The production composition root (16b cut R-a, R2; formerly `mush-server/src/modules.rs`):
//! which [`GameModule`]s the server boots with, their
//! persisted-data load path, and the per-namespace module migrations.
//!
//! Boot order (driven from `main.rs` / test rigs via `mush_server::modules::boot_modules`):
//!
//! 1. [`mush_core::Engine::compose_modules`] has already run every module's `register`
//!    (commands, hooks, ticks, state slots) against the one registry and
//!    frozen mechanics before world hydration; [`mush_core::Engine::from_composition`]
//!    then installed the result on the loaded world.
//! 2. `mush_server::modules::run_module_migrations` brings each registered namespace's
//!    persisted documents up to its module's current schema (BGSAVE first —
//!    the mandatory pre-migrate snapshot), **before** any module parses
//!    them.
//! 3. The boot loader partitions the coordinator-reserved `action-state/*`
//!    documents away from ordinary module data and atomically hydrates them
//!    through their registered typed decoders.
//! 4. The loader supplies every remaining kv doc (minus the
//!    reserved [`SCHEMA_KEY`]) plus the streams the module names via
//!    [`GameModule::boot_stream_requests`].
//! 5. [`mush_core::Engine::modules_loaded`] hands each module its
//!    [`mush_core::ModuleBootData`]; whatever the modules persist during boot
//!    (first-boot seeding, orphan-doc cleanup) is drained and written
//!    through **durably, acked** before the server starts serving.
//!
//! The module list itself is [`server_modules`]: comsys first (its
//! [`AliasStage`](mush_core::AliasStage) is the earliest MODULE alias stage
//! in dispatch; the kernel's macro stage runs before it), then canon, org, faction, scenes, simulation, tactics, economy,
//! mail, jobs, weather, then script (both M5).
//! Their persistence namespaces are disjoint, so the order is otherwise
//! cosmetic — but it is the registration order hooks and observers fire in,
//! and it is the order `on_world_loaded` runs in, so keep it stable.
//!
//! Two orderings are **load-bearing** and documented at their entries: org
//! before economy (economy authorizes through org), and — since E3c — sim
//! before economy, because the economy's boot asks `mush-sim`'s custody
//! provider about interior vault backings and a module state slot exists,
//! empty, from registration onward.
//!
//! Script goes **last** deliberately. Its object `$`-command matcher is a
//! post-command-table [`CommandFallback`](mush_core::CommandFallback), not an
//! alias stage, so it never competes with comsys for dispatch order. Running
//! its `register` after the others means its `on_world_loaded` snapshots a
//! registry that already carries every other module's attr namespaces (the
//! off-loop session enforces module-namespace ownership against that
//! snapshot), and its `AttrSet`/`Moved`/`Said` observers fire after the
//! game-system modules' observers for the same event — the intended order,
//! since a scripted reflex should see the world the built-ins already left.
//!
//! The M0 `--repl` stays module-free on purpose (offline poking of the core
//! surface; it has no store to load module data from).

// The composition adapters that used to be `mush-server` modules (16b cut R-a, R2):
// each maps one library seam onto the game's data and is wired below.
pub mod campaign_formations;
pub mod coprates_membership;
pub mod econ_alerts;
pub mod econ_canon;
pub mod econ_inbox;
pub mod industry_recipes;
pub mod industry_seams;
pub mod mail_transport;
pub mod mind_commerce;
pub mod salvage_source;
pub mod seat_audit;
pub mod sim_routes;

use std::sync::Arc;

use mush_core::GameModule;
use mush_llm::LlmService;
use mush_ruleset::Ruleset;
use mush_worldgen::WorldgenModule;
use smol_str::SmolStr;

/// Reserved kv key per module namespace carrying `{"version": N}` — the
/// namespace's schema version for `mush_server::modules::run_module_migrations`. Filtered out
/// of [`mush_core::ModuleBootData::kv`] so modules never see it.
pub const SCHEMA_KEY: &str = "schema";

/// The built-in game modules a production `mushd` boots with, in
/// registration order, with **default** module config. Config-selected
/// [`sheet_modules`] are appended **after** these (see `main.rs`); hand the
/// concatenation to [`mush_core::Engine::with_modules`].
#[must_use]
pub fn server_modules() -> Vec<Box<dyn GameModule>> {
    server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        // S1: default `[atlas]` limits, an empty shelf but for the shipped packs.
        mush_atlas::AtlasConfig::default(),
        &mush_core::OnboardingConfig::default(),
        // 16b cut R-b: the one game's economy wiring, at its defaults (housing on,
        // market off, the credit).
        &crate::economy::GameEconomyConfig::default(),
        // E7r1 R4: the default composition composes no TacSim layer, so the provider
        // is disabled — `@recovery` registers and answers "not enabled".
        mush_campaign::RecoveryConfig::default(),
    )
}

/// [`server_modules`] with explicit module configs: the `[weather]` knobs (the M10
/// flavor-variant sizing), the `[economy]` knobs (E4's mint bound on a ruleset's
/// faucet-funded obligations), `[onboarding]`'s start rooms (E5b's reserved-room
/// list), and `[wod.economy]` (E5c: whether the housing title issuer is registered at
/// all, plus any extra reserved rooms the ruleset names). The production root
/// (`mush_server::boot::game_modules`) builds the same list through
/// [`server_modules_with_tacsim`] (doc 18 cut IC5), which adds only the `tacsim`
/// quantity delegation and the cell, so those sections in `mushd.toml` reach their
/// modules; this function is the roster WITHOUT the TacSim layer, and every other
/// caller (tests) takes the crate defaults via [`server_modules`].
///
/// `game` is `[tacsim.economy]` (16b cut R-b): whether the housing and market title
/// issuers are registered at all, and the extra reserved rooms. It replaces the
/// per-ruleset economy configs of the two deleted character rulesets.
#[must_use]
pub fn server_modules_with(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
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
        Ok(composed) => composed.0,
        Err(never) => match never {},
    }
}

/// [`server_modules_with`] for a roster that ALSO composes the TacSim layer (doc 18 cut IC5,
/// D-18-33, §4.4 "Offline execution" prerequisite): `mush-inventory` names `tacsim` its third
/// quantity delegate, and the returned cell is the one the layer must be handed
/// ([`tacsim_module_wired`] / [`tacsim_test_world_module_wired`]). A delegation naming an owner
/// the roster does not compose refuses the composition (`StateFamilyRegistryError::UnknownOwner`),
/// so the delegation travels with the layer and never with `server_modules_with`.
#[must_use]
pub fn server_modules_with_tacsim(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
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
/// commerce configuration. Passing this value authorizes only its exact filing
/// issuer in addition to the existing issuer set. Owner order, custody,
/// providers, and inventory delegation remain the ordinary roster's.
///
/// Compose the returned inventory cell with
/// [`tacsim_module_wired_with_mind_commerce`] using this same configuration.
/// This creates no acknowledged deployment or commercial grant.
///
/// # Errors
/// The actual native Economy configuration error from the supplied composition.
pub fn server_modules_with_tacsim_and_mind_commerce(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
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

/// Explicit named M7 host entry using the same ordinary roster and owner order.
///
/// # Errors
/// The actual native Economy configuration error from the supplied launch.
pub fn server_modules_with_launch(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
    recovery: mush_campaign::RecoveryConfig,
    launch: &crate::launch::LaunchComposition,
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
        TacsimComposition::Launch(launch),
        |module| launch.commerce().configure_economy(module),
    )
}

enum TacsimComposition<'a> {
    Absent,
    Standard,
    Commerce(&'a mind_commerce::MindCommerce),
    Launch(&'a crate::launch::LaunchComposition),
}

/// The one body of [`server_modules_with`] and [`server_modules_with_tacsim`] (doc 18 cut
/// IC5): `tacsim_delegate` adds [`crate::NS`] to `mush-inventory`'s quantity delegates, and
/// the returned cell is the roster's inventory handle cell (filled by `mush-inventory`'s own
/// `register`, so any module composed after it finds it filled).
#[allow(clippy::too_many_arguments)]
fn compose_server_modules<E>(
    weather: mush_weather::WeatherConfig,
    mut economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
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
    // Doc 18 cut IC3 (D-18-14; §4.7 "Config flips"): the corridor's ports
    // capitalize their institutions by one-shot treasury obligations of at most
    // 800,000 cr each, so the credit's own mint ceiling is contributed here as a
    // per-instrument override of 1,000,000 cr — on the precedent of
    // `mush_server::config_ext::economy_config_with`'s contributed cadences —
    // **when the operator declared no per-instrument value for
    // `tacsim.credit`**. An operator's `[economy.instruments."tacsim.credit"]`
    // value stands (and one below a needed tranche refuses the lane before
    // anything is made). The global `[economy].mint_obligation_cap_major` does
    // NOT apply to `tacsim.credit` once this is contributed
    // (`EconomyConfig::cap_major` prefers the instrument override). The cap is
    // per instrument, not per faucet, so every `tacsim.credit` faucet obligation
    // — including a staff `@market/vendor … fund faucet=tacsim.treasury` drip
    // once the market composes — gets the 1,000,000 cr ceiling.
    economy
        .instruments
        .entry(crate::economy::CREDIT.to_owned())
        .or_insert(mush_economy::InstrumentCaps {
            mint_obligation_cap_major: crate::port::PORT_MINT_CAP_MAJOR,
        });
    // The issuer is registered when the WoD ruleset is configured **and** its housing
    // knob is on. `issue_title` refuses an issuer that is not on both the economy's
    // list and `HousingModule::with_issuer`, so the two are computed once, here, and
    // a mismatch is impossible by construction.
    let housing_on = game.housing;
    let mut issuers: Vec<mush_org::SystemPrincipalId> = if housing_on {
        vec![housing_title_issuer()]
    } else {
        Vec::new()
    };
    // E6c (§2 item 8, B11): the market mints the title of a stocked asset, so
    // `sr.market` joins the frozen issuer table when the Shadowrun economic layer is
    // enabled — and only then, exactly as housing's joins only for a configured WoD.
    //
    // WS04-S7: computed ONCE, here, for the same reason `housing_on` is. It gates both
    // this list and `MarketModule::with_issuer` below, so the economy's frozen issuer
    // table and the market's own name cannot disagree by construction — the
    // disagreement being what `issue_title` answers `Unauthorized` to, from inside a
    // verb whose operator never named an issuer at all.
    let market_on = game.market;
    if market_on {
        issuers.push(market_title_issuer());
    }
    // E7 cut 3: the claim's titled object is minted through
    // `mush_economy::issue_title` under this crate's own system principal, which
    // must be named in BOTH this list and `IndustryModule`'s own
    // `industry_title_issuer()` or the verb refuses loudly. It is unconditional:
    // `mush-industry` composes on every ruleset.
    issuers.push(mush_industry::industry_title_issuer());
    // Doc 18 cut IC4 (§4.2 schema addition 3, §4.7 part f): the registry that
    // titles programme hulls at the lane's apply. Unconditional, like industry's:
    // the port applier runs wherever the lane does. `issue_title` refuses an
    // issuer not on this list, and the applier preflights its programme's
    // `issuer` against it (`EconomyStore::is_title_issuer`), so a typo in the
    // lane is a loud refusal before any hull is made.
    issuers.extend(mush_org::SystemPrincipalId::new(
        crate::port::hull::REGISTRY_ISSUER,
    ));
    if let Some(commerce) = commerce {
        let selected = commerce.filing_issuer();
        if !issuers.contains(selected) {
            issuers.push(selected.clone());
        }
    }
    let issuer = housing_on.then(housing_title_issuer);
    let market_issuer = market_on.then(market_title_issuer);
    // E7 cut 3 B1: the composition root's own wire between two modules whose
    // order it already fixes. `mush-inventory` fills it while registering;
    // `mush-industry`, composed after it, takes the handles out.
    let inventory_handles: mush_inventory::InventoryHandleCell = Arc::default();
    // Doc 20 cut SM.6 (§3.4): `mush-industry`'s own cell, the same wire the other way round —
    // industry fills it while registering, and the site opener composed onto it reads
    // deposits through it.
    let industry_handles: mush_industry::IndustryHandleCell = Arc::default();
    // Doc 18 cut IC5 (D-18-33; §4.4 "Offline execution" prerequisite): the
    // quantity delegates. `tacsim` joins only on a roster that composes the TacSim
    // layer ([`server_modules_with_tacsim`]): the state-family freeze refuses a
    // delegation naming an owner the roster does not compose (`UnknownOwner`).
    let mut delegates: Vec<&'static str> = vec!["industry", "market"];
    if tacsim_delegate {
        delegates.push(crate::NS);
    }
    // `[onboarding]`'s two start rooms plus Limbo (E5b fold H4), extended by whatever
    // the ruleset knows is never somebody's living room (E5b's handoff to E5c).
    let reserved: Vec<mush_core::ObjId> = [
        onboarding.start_room,
        onboarding.start_room_unapproved,
        mush_core::LIMBO,
    ]
    .into_iter()
    .chain(game.reserved_rooms.iter().copied())
    .collect();
    let simulation = mush_sim::SimulationModule::new()
        .with_route_catalog(Arc::new(self::sim_routes::TacsimRouteCatalog))
        .with_carrier_hooks(Arc::new(self::sim_routes::TacsimCarrierHooks));
    let tactics = match launch {
        Some(launch) => launch.roster.tactics(),
        None => mush_tactics::TacticsModule::with_catalog(tactics_catalog()),
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
        // D1: explicit-only privileges and their audit ledger (`@privilege`).
        Box::new(mush_privileges::PrivilegesModule::new()),
        // E1: organization identity + scoped authorization (the economy's
        // ownership/permission substrate). E1c wires the faction-standing predicate
        // provider so an org AccessPolicy can gate on live faction membership /
        // officer / intel standing (the faction level model stays faction-side).
        // The provider is evaluated at authorize time, never at boot, so org may
        // load BEFORE faction — and since the post-E3 owner decision it does: the
        // faction module validates every `Faction.org` backing link against loaded
        // org state at its own boot (reporting a missing org or a two-factions-one-
        // org conflict loudly, never unbinding). Org's boot reads no faction state.
        // E3b2 freezes two more org tables at wiring: the capability VOCABULARY (so
        // `@org/grant` refuses a typo instead of storing a grant that authorizes
        // nothing), and the scope-use binder table (so `@org/scope retire` can refuse
        // a compartment the economy still binds — org sits below economy and cannot
        // look for an account itself).
        Box::new(
            mush_org::OrgModule::with_predicates(vec![std::sync::Arc::new(
                mush_faction::FactionStandingProvider,
            )])
            .with_capabilities(org_capability_vocabulary())
            .with_scope_use(vec![std::sync::Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        // After org (see above): `Faction.org` links are checked at faction boot.
        Box::new(mush_faction::FactionModule::new()),
        // E6b independent verify fold (M4): the **sealer gate**. `+scene/outcome` is
        // a settlement authority for whatever run staff bound the scene to, and
        // `@scene/facilitate` had no party check at all — so a run's own payee could
        // be conferred the seal that decides which row pays them. `mush-scenes` has
        // no economy edge and must not gain one, so the question is asked through a
        // boxed closure frozen here, exactly as the economy's own seven wiring tables
        // are: the run id stays opaque on the scenes side, and `stands_in_contract`
        // (party through `acts_for`, or author / payer / Johnson / facilitator) is
        // the economy's own answer.
        Box::new(mush_scenes::ScenesModule::new().with_sealer_gate(Arc::new(
            |w: &mush_core::World, run: u64, who: mush_core::ObjId| {
                mush_economy::stands_in_contract(w, mush_economy::ContractId(run), who)
            },
        ))),
        // E7c7 (cut 6 fold, B6): the route catalog reaches `mush-sim` by
        // composition, not by an edge — the impl reads `TacsimStore` at the root,
        // exactly as `TacsimRecipeAuthority` does for `mush-industry`. Unwired,
        // `@journey/depart <id>` builds the manual fixture route; wired, a
        // pack-authored id resolves to its declared legs, drive class,
        // propellant and alignment band.
        // Doc 18 cut IC9 (lead ruling L8): the carrier hooks reach `mush-sim` the
        // same way — the manual-departure refusal on the resolved route and the
        // trade run's boundary and arrival narration (a no-op for anything that is
        // not a programme hull, so every roster carries it).
        Box::new(simulation),
        // S1 (14-spatial-implementation-plan §3.1): the atlas — places with geometry
        // — immediately after `mush-sim`, whose placement graph S2's journeys read.
        // It owns the `atlas` records and the `+atlas`/`@atlas` surface; the packs
        // it applies arrive through the content module, which composes LAST (see
        // [`content_module`]). `mush-atlas -> {core, content, proto}` only.
        atlas_module(atlas),
        // E7 cut 7 B3 (OPEN 1): the observation POLICY is `mush-propagation`'s,
        // but the observation INGRESS has to be `mush-tactics`' — the kernel's
        // owner fence (`ScheduleSourceRegistryBuilder::register` and
        // `scheduled_action_ingress`) forces source-owner ==
        // schedule-family-owner == fired-action-owner, and the six `contact_*`
        // actions are tactics'. So the policy arrives here, at the composition
        // root, exactly as `IndustryModule::with_recipes` and
        // `SimulationModule::with_accounts` do, and **`mush-tactics` gains no
        // `mush-propagation` edge**: the trait is `mush-core`'s.
        // 16b cut C.2 (C1/C3): the catalog is the game's shipped content — the
        // stock tank of doc 16 §3.3 — so `@unit/create <obj>=tacsim.unit.line_tank`
        // resolves in production and the arena pack's applier and this verb make
        // the same unit from the same spec. The module composed with an empty
        // catalog until this cut and every `@unit/create` refused for want of a
        // schema.
        Box::new(tactics),
        // E7 cut 7 (10b §3.7): the propagation layer itself. It composes AFTER
        // `mush-sim` (whose `SpatialState` every context is built from) and
        // after `mush-tactics` (whose leak-safe contact ledger its policy
        // reads); `on_world_loaded` runs in list order, so both are hydrated
        // before propagation's boot. It registers NO contact family and NO
        // contact guard validator — `mush-tactics` is the sole `ContactState`
        // authority (08i answer 1, D-E7-36).
        Box::new(
            mush_propagation::PropagationModule::new()
                .with_dispatch_sources(Arc::new(mail_transport::MailTransport))
                .with_metric(Arc::new(crate::relay_metric::RelayMetric))
                .with_service(Arc::new(crate::relay_service::RelayService)),
        ),
        // E2: the accountable-value engine — the double-entry journal, accounts, and
        // the strict durable money ack-gate. After org (it authorizes through it).
        // E3b1 freezes the custody provider table at wiring: the built-in container
        // provider is appended by the module itself, and E3c adds the interior
        // provider for `@vault/create … interior:<space>`.
        //
        // E3c moved economy **after** `SimulationModule` (org must still come
        // first; scenes and sim read no economy state at boot). Boot order is
        // `on_world_loaded` order, and `SpatialState` exists — EMPTY — from
        // registration onward, so an economy booting first would ask the interior
        // provider about a vault backing and get a definite `Some(false)` from a
        // module that simply has not loaded its documents yet: every interior
        // vault would boot `Frozen` and every deposited asset would latch
        // `custody-mismatch`. With sim ahead of it, the economy's custody
        // reconciliation and vault-backing validation see real state, and a
        // `Some(false)` can be trusted as the drift it claims to be.
        Box::new({
            // E7 cut 3: `mush-industry` registers a `CustodyProvider` for its own
            // `CustodyRef::Registered { provider: "industry", .. }` backings, on
            // E3's rules unchanged (exactly one claiming provider,
            // validate-then-mutate). It needs no module ordering of its own:
            // a registered backing has no world object, so `backing_valid`
            // answers without reading any state and cannot boot `Frozen` the way
            // an interior would.
            let module = mush_economy::EconomyModule::with_custody(vec![
                Arc::new(mush_sim::InteriorCustodyProvider),
                Arc::new(mush_industry::IndustryCustodyProvider),
            ])
            .with_config(economy)
            // E4c (08c §6.1): three more wiring-frozen tables, none of which
            // changes the module ORDER above. The fact table answers a contract's
            // `Fact` conditions (jobs today; doc 10 §15's sim/tactics boundary
            // facts next), the observer table carries the canon evidence bridge,
            // and the title-issuer table is empty until a ruleset registers one —
            // `@asset/title` stays the staff path meanwhile.
            // E7 cut 2 (OPEN 5, D-E7-30, §10): `inventory.*` joins `jobs.*`.
            // One provider per crate — `with_facts` PANICS on a duplicate
            // namespace — and both inventory facts are answered from the
            // committed bundle's own applied marker, never from a status.
            // E7 cut 3 (D-E7-30, §10): `industry.*` joins them. One provider
            // per crate; `industry.workorder.completed` is **cut 4's arm on
            // this same provider**, never a second registration.
            // E7 cut 9 (D-E7-30, §10): `campaign.*` joins them as the FIFTH and
            // last namespace on that list. One provider per crate, and
            // `campaign.operation.resolved` is answered from the operation's own
            // applied root marker — never from its `Resolving` status, so a
            // contract gated on it cannot settle from a resolution that merely
            // started (08c §6.2).
            // E7 cut 10 (D-E7-30, 10b 10): `sim.*` joins them - the cut's FIRST
            // and `mush-sim`'s only registration, answering
            // `sim.station.served` from the applied `sim.station_service`
            // marker (never a status: a sortie that merely started answers
            // unknown). Five crates, one namespace each, and `with_facts`
            // PANICS on a duplicate.
            //
            // Doc 18 cut IC8 (lead ruling L13): `market.*` joins them as the SIXTH
            // namespace — `market.lot.drawn`, doc 18 cut IC7's, answered from the
            // draw-receipt mirror on the quote (`Quote.drawn_at`), never a status.
            // On every roster, because the market composes on every roster and the
            // namespace is its own; a Draw sale whose fact nobody answers would never
            // settle, and the market refuses one up front when this is missing.
            .with_facts({
                let mut facts: Vec<Arc<dyn mush_economy::FactProvider>> = vec![
                    Arc::new(mush_jobs::JobsFactProvider),
                    Arc::new(mush_inventory::InventoryFactProvider),
                    Arc::new(mush_industry::IndustryFactProvider),
                    Arc::new(mush_campaign::CampaignFactProvider),
                    Arc::new(mush_sim::SimFactProvider),
                    Arc::new(mush_market::MarketFactProvider),
                ];
                // Survey receipts are private typed state. Only a roster with
                // their actual owner can grant Economy its scoped read handle.
                if tacsim_delegate {
                    facts.push(Arc::new(
                        crate::surface::survey_facts::SurveyFactProvider::default(),
                    ));
                }
                facts
            })
            // E5b: housing's exact-mode mirror joins the canon bridge. It hears the
            // obligation and contract transitions a lease is made of; abstract mode
            // does not need it at all.
            .with_observers(econ_observers())
            // E5b: `system:wod.housing` mints the title record on a property's anchor
            // room through `mush_economy::issue_title` (D-E3-13, 08d §2). The same
            // principal is wired into `HousingModule::with_issuer` below, and
            // `issue_title` refuses any issuer that is not on BOTH lists.
            //
            // E5c: 08d §2's "only when the WoD ruleset is configured" is now real —
            // the list is empty when no WoD ruleset is enabled or
            // `[wod.economy].housing = false`, and `@property/create` then refuses
            // rather than minting title nobody asked for.
            .with_title_issuers(issuers)
            // E4c verify fold (E1): the canon kinds that count as a
            // **producer-declared outcome**. `scene.concluded` is a lifecycle record
            // any participant can produce, so a payee gated on one could seal their
            // own settlement — it is deliberately **never** registered.
            //
            // E6b registers exactly `scene.outcome`, which `mush-scenes` writes only
            // from `+scene/outcome` under a **staff-conferred sealer** (08f §4.1,
            // D-E6-7). Who may seal is the producer's gate: `CanonRecorded` carries
            // no `detail`, so the economy checks kind + source + party intersection
            // and nothing else. Every other `canon:` attestor set is still refused at
            // `@contract/offer`.
            //
            // E7 cut 9 (10b §3.9 OPEN 3) registers the table's SECOND entry,
            // `campaign.operation.resolved`, which `mush-campaign` writes exactly
            // once per operation, from the terminal settlement, under
            // `source: "op:{id}"`. It is an outcome and not a lifecycle record: an
            // operation reaches it only through `enter_resolving`, which is gated
            // on the frozen consent surface still hashing, and no participant can
            // produce one for an operation they do not command.
            // `AttestorSet::Canon { kind, source }` still requires the specific
            // source, so a contract cites ONE operation and never "any resolution".
            .with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_scenes::KIND_SCENE_OUTCOME),
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ])
            // E6b (DL-09): the bounded canon read seam. The economy's canon
            // evidence is edge-triggered and its RAM tail dies with the process, so
            // a seal committed while the economy was frozen — or before a restart —
            // was unrecoverable. This is the composition root reading canon's own
            // open tail on the economy's behalf; the edge still runs the other way.
            .with_canon_evidence(Arc::new(self::econ_canon::CanonEvidenceReader))
            // E6b (DL-11 / M15): the organizations whose contract **acceptance** may
            // be performed programmatically. A staff-built NPC fence has no officer
            // and an org party accepts only through `acts_for` holders or staff,
            // which would put a staffer in every single deal. Ids only, from
            // `[shadowrun.economy].vendor_orgs`, frozen here — and conferring no
            // money authority whatever: the escrow still posts under the org
            // account's own debit authority and still refuses when it is short.
            .with_party_caps(Vec::new())
            // Doc 18 cut IC8 (lead ruling L13; doc 18 cut IC7's seam): on a roster
            // that composes the TacSim layer, the content-resolved party-cap source —
            // the port institutions the lane's port maps record (vendors and the
            // programme org) accept programmatically. Asked only after the table
            // said no, and conferring no money authority whatever. A roster without
            // the layer composes none.
            .with_party_cap_source_if(tacsim_delegate)
            // Doc 18 cut IC14 row 7 (lead ruling L6): the market's dispute veto, on
            // every roster — game-neutral, it refuses only a seller disputing their
            // own sale into a bid and answers nothing with no market store.
            .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard));
            let module = configure_economy(module)?;
            match launch {
                Some(launch) => launch.roster.economy(module),
                None => module,
            }
        }),
        // E7 cut 2 (10b §3.2, Q13A/Q2D, D-E7-3): the neutral quantity authority.
        // It boots **immediately after `mush-economy`** because its own boot
        // reconciliation asks the economy about vault status — the housing
        // precedent exactly — and before `mush-market`.
        //
        // **E7r2 P1 amends the rest of this note in place rather than deleting
        // it, because it is now false by decision.** It used to end "…and before
        // `mush-market`, which must never learn that quantities have another
        // owner: a listing's `quantity` stays the market's own advertised
        // counter and becomes no `Pool` claim." That is still true of E6c's
        // three `Supply` arms and it is exactly why they can over-advertise
        // harmlessly. It is deliberately NOT true of `Supply::Lot`
        // (10b §15.1 item 3; the owner's fix-before sentence: *"a produced lot
        // that cannot be sold leaves the extract → produce → sell loop inert,
        // and the corridor's first sale must be a real listing"*). A lot listing
        // advertises finite matter this module is the truth for, so it holds a
        // real reservation on this module's own `ReservationBook` — through the
        // delegation below, never through a second quantity authority of the
        // market's own.
        //
        // `mush-inventory -> {core, economy}` is unchanged and is pinned by
        // `mush-inventory/tests/e7c2.rs`: the new edge runs the other way.
        Box::new({
            let module = mush_inventory::InventoryModule::new()
                // E7 cut 3 B1: the cross-owner write bridge cut 2's own handoff
                // named as cut 3's. `mush-inventory` delegates its typed
                // families to `"industry"` so an extraction cycle debits the
                // deposit's pool, mints the operator's lot and advances the run
                // in **one** bundle. The delegation widens the bundle, never the
                // gate: every quantity mutation still rides
                // `InventoryFamilies`' exported verbs and
                // `ReservationBook::checked_apply_resource`.
                //
                // **E7r2 P1 (§15.5 (b′)) adds `"market"` on exactly the same
                // terms**, and it is the SECOND consumer of one pattern rather
                // than a new one: the market's own `market_lot` action reserves
                // at `@market/vendor stock`, settles at `+market/accept` and
                // releases at `@market/vendor withdraw`, each in one bundle,
                // under `mush_market::listing_holder(ListingId)` — which is
                // `mush_industry::wo_holder`'s shape. No `InventoryVerb` is
                // added and no `ENGINE_OWNED_RESERVATIONS` entry is registered.
                //
                // **Doc 18 cut IC5 (§4.4, D-18-33) adds `"tacsim"` as the THIRD,
                // on the roster that composes it** ([`server_modules_with_tacsim`],
                // which `mush_server::boot::game_modules` uses): the port tick reads
                // pools through its own `Ctx::action_state`, and IC8's stock arm and
                // IC9's burn will write through the same exported verbs in one
                // bundle. Three of `MAX_STATE_FAMILY_DELEGATES = 4` are then used.
                .with_quantity_delegates(delegates)
                // ---- E7r2 P2: the wreck oracle, hosted HERE ----
                //
                // `mush-inventory → {core, economy}` and nothing else, pinned by
                // `mush-inventory/tests/e7c2.rs`; whether a carrier root is a
                // recoverable wreck is `mush-sim`'s fact. So the trait is declared in
                // `mush-inventory` and implemented in `self::salvage_source`, the
                // one crate with edges to both — `sim_routes::TacsimRouteCatalog`'s
                // shape exactly, and for exactly the same reason. Naming it here is
                // what makes `@salvage` exist at all: an unwired composition
                // registers no salvage action and refuses byte-identically to a
                // wreck that does not exist.
                .with_salvage_source(Arc::new(self::salvage_source::SimSalvageSource))
                .with_handle_cell(Arc::clone(&inventory_handles));
            match launch {
                Some(launch) => launch.roster.inventory(module),
                None => module,
            }
        }),
        // E7 cut 3 (10b §3.3, D-E7-3, doc 10 §14): the neutral industry
        // authority. It composes **immediately after `mush-inventory`** — whose
        // handles and delegation it needs — and **before `mush-market`**, which
        // it does not touch at all: a `Representation::Lot` listing is still
        // refused at creation and lifting that refusal is cut 11's.
        Box::new({
            let module = mush_industry::IndustryModule::new()
                .with_inventory(Arc::clone(&inventory_handles))
                // E7c4 B2: the recipe authority reaches industry by composition,
                // not by an edge — the impl reads `TacsimStore` at the root.
                .with_recipes(Arc::new(self::industry_recipes::TacsimRecipeAuthority))
                // Doc 20 cut SM.5 (§9.1, D-20-12): the Freeport Registry's
                // prospecting licence, `crate::economy::LICENCE_FREEPORT`, named to
                // the industry authority so `mush_industry::extraction_authorized`'s
                // licence half (`crates/mush-industry/src/licence.rs`) can at last
                // answer Ok. Wired only on the roster that composes the TacSim
                // layer, whose sheet module registers the instrument at boot; on
                // every other roster industry still answers `NoLicenceInstrument`.
                .with_licence_if(tacsim_delegate)
                // Doc 20 cut SM.6 (§3.4): the handle cell, on every roster.
                .with_handle_cell(Arc::clone(&industry_handles))
                // Doc 20 cut SM.6 (§3.4, D-20-5, D1, D11, D13): on the roster that composes
                // the TacSim layer, the site clerks' system opener, the surface draws the
                // sweep reconciles each surface pool against, and the typed-state delegation
                // to `tacsim` (the port tick and readiness read deposits).
                .with_surface_seams_if(tacsim_delegate, &industry_handles);
            match launch {
                Some(launch) => launch.roster.industry(module),
                None => module,
            }
        }),
        // E7 cut 9 (10b §3.9, D-E7-18/-19/-20/-30/-32): the campaign layer. It
        // composes **after `mush-economy`, `mush-org` and `mush-canon`** — its
        // bond is an E4 hold on the economy's own carve-out lane, its `force.*`
        // authority resolves through `mush_org::authorize`, and its terminal root
        // is one `mush_canon::commit` — and it needs no ordering of its own
        // beyond that: its boot reads only its own documents and the
        // composition-frozen formation authority. `mush-campaign -> {core,
        // economy, canon, org}`, and deliberately **no `mush-faction` edge**: the
        // front consequence is discharged by that one root canon record, which
        // `mush-faction`'s pre-existing `on_canon_recorded` observer already turns
        // into a `Clock::advance` and an `on_complete_control` flip.
        Box::new(
            mush_campaign::CampaignModule::new()
                // OPEN 4: formation KINDS reach the campaign layer by
                // composition, not by an edge — the impl reads `TacsimStore` at
                // the root, exactly as `TacsimRecipeAuthority` does for
                // `mush-industry`. Unwired, `@force/create` refuses every kind.
                .with_formation_kinds(Arc::new(
                    self::campaign_formations::TacsimFormationAuthority,
                ))
                // E7 cut 10 B3 (OPEN 4): the server-side, audit-only seat
                // attribution log, on the same composition seam. The row
                // linking a driver token to an account NEVER becomes a
                // world document, a journal line or a canon record; one
                // staff verb asks whether a token is attributed, and it
                // prints no id. Unwired is fail-closed.
                .with_seat_audit(Arc::new(self::seat_audit::SeatAuditLog::new()))
                // E7 residue pass 1, R4/R3: `[tacsim.recovery]`, mapped from the
                // ruleset's frozen block by [`recovery_policy`] on this same
                // composition seam. The campaign crate takes no `tacsim`
                // edge; `cargo tree -p mush-campaign` is the test. Unwired (the
                // `mush-server` bootstrap path, which composes no TacSim layer) is
                // the disabled default — fail closed.
                .with_recovery(recovery),
        ),
        Box::new(mush_mail::MailModule.with_transport(Arc::new(mail_transport::MailTransport))),
        Box::new(mush_jobs::JobsModule::new()),
        // E5: housing boots after `{org, sim, tactics, economy, jobs}` (08a decision
        // 19, 08d §2). `on_world_loaded` runs in list order and housing's boot
        // reconciliation asks all four of them questions: org for the landlord
        // compartment, the economy for cadence positions and rent obligations, jobs
        // for the story queue, and the world itself for the rooms.
        // E5b verify fold (H4): the reserved-room list is wired **now**, not left to
        // E5c. D-E5-12 says a chargen start room is never somebody's living room, and
        // housing enforces it without a `mush-config` edge by being told — so until
        // the host tells it, `@property/create <name>=<the start room>` was accepted
        // and every fresh character would have walked out of chargen into a private
        // residence the access hook then answered for. `[onboarding]` owns the two
        // rooms; Limbo is on the list belt-and-braces (housing refuses it anyway).
        // E5c **extends** that list with `[wod.economy].reserved_rooms` (E5b's
        // handoff) — it does not re-wire it.
        Box::new({
            let m = mush_housing::HousingModule::new().with_reserved_rooms(reserved);
            match issuer {
                Some(i) => m.with_issuer(i),
                None => m,
            }
        }),
        Box::new(mush_weather::WeatherModule::with_config(weather)),
        // E6a (08f §2): the ephemeral party subsystem. `mush-party -> {core}` only —
        // it is **not** an economic principal and must never learn about money
        // (answers 5 and 21, D-E6-5). It reads no economy state and the economy reads
        // none of it, so it sits anywhere before the rulesets; here, beside the other
        // player-facing subsystems and ahead of `mush-script`'s deliberate last place.
        Box::new(mush_party::PartyModule::new()),
        // 17d M6.1: neutral authoring records load before the game audits their
        // immutable source references in on_all_loaded. Cross-namespace writes
        // are delegated only on rosters which also compose the TacSim owner.
        // Roll must also precede Market: its final commercial audit opens the
        // readiness gate required by Market's paired renewal recovery. All
        // ordinary owner load hooks finish before either final audit runs.
        Box::new({
            let module = mush_roll::RollModule::with_writers(tacsim_delegate.then_some(crate::NS));
            match commerce {
                Some(commerce) => commerce.configure_roll(module),
                None => module,
            }
        }),
        // E6c (08f §2, §5): the neutral market. It boots **after** `mush-economy` —
        // it validates listings against real accounts, assets and vaults, and reads
        // the economy's own contract state for the trade tape the safe projection
        // folds — and before the rulesets, which register content, dice and delivery
        // into its three wiring-frozen tables. `mush-market -> {core, org, economy,
        // inventory}` (E7r2 P1's one new edge); nothing Shadowrun-shaped is inside it
        // (answer 20).
        //
        // It also boots **after `mush-inventory`**, and now for a second reason: the
        // handle cell must be filled by the time this module registers, which it is —
        // inventory composes above and `Engine::compose_modules` registers in list
        // order.
        Box::new({
            let m = mush_market::MarketModule::new()
                .with_catalog_sources(Vec::new())
                .with_availability(Vec::new())
                .with_delivery(Vec::new())
                // E7r2 P1 (§15.5 (b′)): `IndustryModule::with_inventory`'s shape
                // exactly. It carries a **capability**; the authority to write with it
                // is `with_quantity_delegates(["industry", "market"])` above (with
                // `"tacsim"` third on a roster that composes the TacSim layer, doc
                // 18 cut IC5, §4.4, D-18-33), and without that name the very first
                // `settle_sale` would answer `ActionStateError::OwnerMismatch`. Unwired is fail-closed: no lot
                // action registers and `@market/vendor stock` refuses a
                // lot-represented good byte-identically to E6c.
                .with_inventory(Arc::clone(&inventory_handles))
                // Doc 18 cut IC8 (lead ruling L13; doc 18 cut IC7's two seams): on a
                // roster that composes the TacSim layer, the port presence gate and
                // the hull capacity provider; `plan(..).is_some() == governed(..)` for
                // every endpoint, because both answer from `port::market::governed_hold`.
                // A roster without the layer composes neither.
                .with_port_seams_if(tacsim_delegate);
            // WS04-S7: housing's shape at the top of this list, for housing's reason.
            // Wired unconditionally, `sr.market` was a name the market held and the
            // economy did not list on every non-Shadowrun server, so the one arm that
            // could reach `issue_title` answered `Unauthorized` — a permission
            // refusal for a capability the operator never asked for — instead of the
            // market's own "this server registers no market title issuer".
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

/// **Doc 18 cut IC8's economy seam, composed only where the TacSim layer is**
/// (lead ruling L13): [`crate::port::market::PortPartyCaps`].
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

/// **Doc 18 cut IC8's market seams, composed only where the TacSim layer is**
/// (lead ruling L13): [`crate::port::market::PortVenueGate`] and
/// [`crate::port::market::PortCapacity`] — and, since doc 18 cut IC10, the proceeds
/// policy [`crate::port::proceeds::CoopProceeds`] (the Co-op's share and the sale
/// line; unwired on every roster without the layer).
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

/// **Doc 20 cut SM.5's licence wiring, composed only where the TacSim layer is**
/// (the [`PortMarketSeamsExt`] shape): the industry authority is told the
/// prospecting-licence instrument ([`crate::economy::LICENCE_FREEPORT`]); the
/// account in it is opened by `+licence/buy go` (`crate::port::licence`), and the
/// term every licence-gated act reads first lives on tacsim's own record.
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

/// **Doc 20 cut SM.6's industry seams, composed only where the TacSim layer is** (the
/// [`IndustryLicenceExt`] shape): [`industry_seams::SiteOpener`] as the system opener,
/// [`industry_seams::SurfaceDraws`] as the external draws, (doc 20 cut SM.10)
/// [`industry_seams::SurfaceTenure`] as the tenure authority, and the typed-state delegation of
/// industry's families to `tacsim` (D1) — a capability to read deposits, which tacsim's source
/// tripwire (`tests/deposits.rs`, D10) holds to reads. Unwired, `DepositOpen` stays staff-only,
/// the delegation is absent, and a surface deposit (none exists there) would read
/// `ExternalUnwired`.
trait IndustrySurfaceExt: Sized {
    fn with_surface_seams_if(self, on: bool, cell: &mush_industry::IndustryHandleCell) -> Self;
}

impl IndustrySurfaceExt for mush_industry::IndustryModule {
    fn with_surface_seams_if(self, on: bool, cell: &mush_industry::IndustryHandleCell) -> Self {
        if on {
            self.with_system_opener(Arc::new(industry_seams::SiteOpener::new(Arc::clone(cell))))
                .with_external_draws(Arc::new(industry_seams::SurfaceDraws))
                // Doc 20 cut SM.10 (§9, D1/D2): the claim acts' composed authority.
                .with_tenure_authority(Arc::new(industry_seams::SurfaceTenure::new()))
                .with_state_delegates([crate::NS])
        } else {
            self
        }
    }
}

/// The economy observer table this server freezes at wiring (16b cut R-b: the one
/// game's; the Shadowrun board's mirror left with its ruleset).
fn econ_observers() -> Vec<Arc<dyn mush_economy::EconomyObserver>> {
    vec![
        Arc::new(self::econ_canon::CanonEvidenceBridge),
        Arc::new(mush_housing::HousingEconomyObserver),
        // E6a step 1 (DL-04): an organization's officers are told, kinds only, what
        // its contracts and obligations did.
        Arc::new(self::econ_inbox::OrgInbox),
        // E6a step 1 (DL-05): the ladder's pages reach connected wizards on the
        // ladder's own re-notify schedule, deduped per step.
        Arc::new(self::econ_alerts::StaffAlerts::default()),
        // E6c: the market's ear. It hears one thing — a contract reaching a terminal
        // state — and turns a settlement into the trade-tape row the safe public
        // projection folds, a cancellation or expiry back into stock.
        Arc::new(mush_market::MarketObserver),
        // Doc 18 §4.1 (cut IC6): a staff-ended charter is seen — a Co-op lease
        // reaching a terminal state folds its enrolment into `revoked` and
        // tells the pilot. It no-ops on a roster without the TacSim layer.
        Arc::new(crate::port::coop::CoopObserver),
    ]
}

/// The system principal housing mints property titles under (08d §2, D-E3-13).
///
/// It is named in exactly two places — `EconomyModule::with_title_issuers` and
/// `HousingModule::with_issuer` — and `mush_economy::issue_title` refuses an issuer
/// that is not on the economy's list, so a typo in either one is a loud refusal at
/// `@property/create` rather than a silent capability.
#[must_use]
pub fn housing_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("tacsim.housing")
        .expect("a built-in system principal id is well-formed")
}

/// The system principal the market mints a stocked asset's title under (E6c B11,
/// §2 item 8, D-E3-13).
///
/// Named in exactly two places — `EconomyModule::with_title_issuers` and
/// `MarketModule::with_issuer` — and `mush_economy::issue_title` refuses an issuer
/// that is not on the economy's list, so a typo is a loud refusal rather than a
/// silent capability. Registration, not code.
///
/// WS04-S7: **both** of those places are gated by the one `market_on` flag, exactly
/// as housing's two are gated by `housing_on`. The second used to be unconditional,
/// so every non-Shadowrun server held a name the economy did not list.
#[must_use]
pub fn market_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("tacsim.market")
        .expect("a built-in system principal id is well-formed")
}

/// 17g C1b: the capabilities the named launch's governed hardware authorizes
/// (`mush_industry::core_production`'s die issue on the importer's stock scope,
/// and integration and refit on its factory scope). Part of
/// [`org_capability_vocabulary`].
pub const NAMED_LAUNCH_HARDWARE_CAPABILITIES: [&str; 3] = [
    mush_industry::core_production::IMPORT_CAPABILITY,
    "industry.core.integrate",
    "industry.core.refit",
];

/// The capability vocabulary this server recognizes (E3b2 §4.8.4 / §4.10): every
/// capability the economy asks org about (`mush_economy::CAPABILITIES`) plus the two
/// org-administrative ones the org module's own doctrine names.
///
/// It is a **typo catcher**, not an authorization gate — `authorize` matches
/// capability ids by equality, so an unknown one already grants nothing; what this
/// buys is that granting `economy.acount.post` fails loudly at the command instead
/// of quietly producing an inert record. Add to it whenever a module starts asking
/// about a new capability.
#[must_use]
pub fn org_capability_vocabulary() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_housing::CAPABILITIES.iter().copied())
        // E7 cut 9 (OPEN 5, D-E7-19): the five `force.*` ids `mush-campaign` asks
        // `mush_org::authorize` about. Without them `@org/grant <who>=force.create/...`
        // fails LOUDLY at the command instead of storing a grant that authorizes
        // nothing — which is exactly what this non-empty set is for.
        .chain(mush_campaign::CAPABILITIES.iter().copied())
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
        // Vocabulary declaration only; default composition grants nothing.
        .chain([mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY])
        .chain([mush_roll::commerce::registry_recall::REGISTRY_RECALL_CAPABILITY])
        .chain([mush_roll::commerce::registry_revocation::REGISTRY_REVOCATION_CAPABILITY])
        // 17g C1b (a stop-rule fix, older than C1): the named launch's governed
        // hardware asks org about these three (`mush-industry`'s die issue,
        // integration and refit authorities), and its operator grants them with
        // `@org/grant` on the importer's scopes. Without them the first boot of
        // the named launch through `game_modules` cannot be set up at all.
        // Vocabulary only: nothing is granted.
        .chain(NAMED_LAUNCH_HARDWARE_CAPABILITIES)
        .map(|c| mush_org::CapabilityId::new(c).expect("a built-in capability id is well-formed"))
        .collect()
}

/// S6: the rules-interface module for `rs`: its display name and its chargen
/// stages, snapshotted at composition.
fn rules_interface_for<R: mush_ruleset::Ruleset>(
    rs: &R,
) -> mush_ruleset::interface::RulesInterfaceModule {
    let stages = rs.chargen().map_or_else(Vec::new, |c| c.stages());
    mush_ruleset::interface::RulesInterfaceModule::new(rs.display_name().to_string(), stages)
}

/// The one game's character ruleset and its rules interface (16b cut R-b, D-16-14):
/// the pilot sheet, composed unconditionally **after** [`server_modules`] so its
/// commands, panel feed and approval queue wire cleanly and its payload handler
/// (registered in `on_world_loaded`) runs after the jobs module's state slot
/// exists.
#[must_use]
pub fn sheet_modules(
    onboarding: &mush_core::OnboardingConfig,
    game: &crate::economy::GameEconomyConfig,
) -> Vec<Box<dyn GameModule>> {
    let rs = crate::sheet::PilotRuleset::new()
        .onboarding(onboarding.clone())
        .economy(game.clone());
    // S6 (doc 14 §4.6): beside the ruleset, the small module that fronts its
    // chargen stages and the help topics as the off-duty gamemaster's interface;
    // the stages are snapshotted here, before the ruleset is boxed.
    let rules_interface = rules_interface_for(&rs);
    vec![Box::new(rs), Box::new(rules_interface)]
}

/// The **TacSim layer** (E7 cut 1, 10b §3.1), composed **after** the character
/// ruleset.
///
/// The pilot sheet from [`sheet_modules`] is unconditional. This layer supplies
/// its physical and civil context and is switched by `[tacsim].enabled`.
///
/// **Order position matters and is pinned** (`tacsim_composes_after_the_character_ruleset`).
/// `on_world_loaded` runs in list order, so registering last is what lets TacSim read
/// the character ruleset's facts at boot while that ruleset reads TacSim's
/// environmental facts through the generic seam.
///
/// The module is composed **unconditionally**, and `enabled` gates behaviour rather
/// than existence: a disabled
/// layer answers "not enabled on this server" instead of hiding a verb, which is also
/// what keeps `@tacsim` visible to the rule-1 allowlist tripwire in a default
/// composition.
#[must_use]
pub fn tacsim_module(cfg: crate::TacsimConfig) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, false))
}

/// [`tacsim_module`] with the game's own seed packs (doc 18 cut IC2; lead ruling
/// R2): `game_content` true loads the lane
/// ([`crate::definitions::content::game_seed_sources`]) into the store ahead of
/// the operator's packs. The production composition passes the same boolean it
/// hands [`content_module_with`] (`[tacsim].enabled`), so `TacsimStore`'s loaded
/// packs and the content latch's boot list are switched together and the lane's
/// routes materialize over the places the latch applied.
#[must_use]
pub fn tacsim_module_with(cfg: crate::TacsimConfig, game_content: bool) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, game_content))
}

/// [`tacsim_module_with`] as a **test world's** (doc 18 cut IC3; lead ruling
/// L15): the readiness report accepts a world rate other than 24 there, and
/// (doc 18 cut IC5) the port tick registers its probe action. For the
/// in-process test engines only; the production roster
/// (`mush_server::boot::game_modules`) never calls it, so dev1 can never be a
/// test world (doc 18 §4.7).
#[must_use]
pub fn tacsim_test_world_module(
    cfg: crate::TacsimConfig,
    game_content: bool,
) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, game_content).with_test_world(true))
}

/// The TacSim layer with the inventory handle cell (doc 18 cut IC5): the production roster's
/// spelling. NO test-world flag: production can never be a test world (IC3 L15).
#[must_use]
pub fn tacsim_module_wired(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_game_content(cfg, game_content).with_inventory(inventory))
}

/// The real production layer with explicit prepared Registry configuration.
/// Its exact config/catalog check refuses stale composition; no test-world
/// flag or runtime authority is created by this constructor.
pub fn tacsim_module_wired_with_mind_commerce(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
    commerce: &mind_commerce::MindCommerce,
) -> Result<Box<dyn GameModule>, mush_core::ModError> {
    let module =
        crate::TacsimRuleset::with_game_content(cfg, game_content).with_inventory(inventory);
    Ok(Box::new(commerce.configure_tacsim(module)?))
}

/// [`tacsim_module_wired`] as a **test world's** (IC3 L15; doc 18 cut IC5): the readiness
/// report's relaxed world rate, and the port tick's probe action. For the in-process test
/// engines only; `mush_server::boot::game_modules` never calls it.
#[must_use]
pub fn tacsim_test_world_module_wired(
    cfg: crate::TacsimConfig,
    game_content: bool,
    inventory: mush_inventory::InventoryHandleCell,
) -> Box<dyn GameModule> {
    Box::new(
        crate::TacsimRuleset::with_game_content(cfg, game_content)
            .with_inventory(inventory)
            .with_test_world(true),
    )
}

/// **The game's unit catalog** (16b cut C.2), built from the content compiled
/// into this crate.
///
/// The shipped content is digest-pinned by `crates/tacsim/tests/content.rs`, so
/// this cannot fail on a tree that passed them; an empty catalog is the honest
/// degradation if it ever did, and `@unit/create` then refuses by name exactly
/// as it did before this cut.
fn tactics_catalog() -> mush_tactics::UnitCatalog {
    match crate::builtin_definitions() {
        Ok(catalog) => catalog.units,
        Err(error) => {
            tracing::error!(%error, "tacsim: the shipped unit content did not bind");
            mush_tactics::UnitCatalog::default()
        }
    }
}

/// The **atlas** (S1, 14-spatial-implementation-plan §3.1): the spatial records, the
/// rasterizer, the `+atlas` views and `@atlas/pack`, with `[atlas]`'s limits and the
/// operator's packs, and the shipped starter packs on the shelf whatever
/// `[game].starter_area` says — the shelf is what staff plan and apply from, and
/// listing a pack applies nothing.
///
/// **16b cut Z.3** shelves the game's own two boot packs after them
/// ([`crate::definitions::content::boot_packs`]: the proving ground's ground,
/// then the arena), on the same rule and whatever `[tacsim].enabled` says. A
/// standing world has latched both at a first boot, and the shelf is the one way
/// it takes a newer version by hand — which Z.3's version 3 of the arena, with
/// its tanks 5 and 6, is the first to need. Shelving applies nothing: the boot
/// pass is unchanged, and `@atlas/pack plan` on an arena this world has never
/// latched plans a first apply that staff may read and decline.
#[must_use]
pub fn atlas_module(cfg: mush_atlas::AtlasConfig) -> Box<dyn GameModule> {
    let mut shelf = mush_atlas::starter::packs().unwrap_or_else(|error| {
        // The shipped packs are digest-pinned in `mush-atlas`'s own tests; this
        // cannot fail on a tree that passed them, and a shelf without them is the
        // honest degradation if it ever did.
        tracing::error!(%error, "atlas: the shipped starter packs did not load");
        Vec::new()
    });
    match crate::definitions::content::boot_packs() {
        Ok(game) => shelf.extend(game),
        // Pinned in `crates/tacsim/tests/content.rs`, so the same argument: a
        // shelf without them is the honest degradation, and the boot pass (which
        // loads the same bytes) says so again in its own words.
        Err(error) => tracing::error!(%error, "atlas: the game's boot packs did not load"),
    }
    // Doc 18 cut IC1 (§4.7): the trade run's station packs (the corridor's four and,
    // since doc 19 cut A.8, the moon ports' six), after the proving ground, on the
    // same degrade-and-log rule (pinned in
    // `crates/tacsim/tests/stations.rs`). Shelved always, so a standing world takes a
    // newer station version by hand exactly as it takes a newer arena.
    match crate::definitions::content::station_packs() {
        Ok(stations) => shelf.extend(stations),
        Err(error) => tracing::error!(%error, "atlas: the station packs did not load"),
    }
    // Doc 18 cut IC2: the lane after the stations, on the same degrade-and-log
    // rule (pinned in `crates/tacsim/tests/lane.rs`) — a tacsim seed pack on the
    // shelf, as the arena is.
    match crate::definitions::content::lane_packs() {
        Ok(lane) => shelf.extend(lane),
        Err(error) => tracing::error!(%error, "atlas: the lane pack did not load"),
    }
    match crate::relay_content::packs() {
        Ok(packs) => shelf.extend(packs),
        Err(error) => tracing::error!(%error, "relay shelf pack refused"),
    }
    Box::new(mush_atlas::AtlasModule::with_config(cfg).with_shelf(shelf))
}

/// The **content module** (S1, 14-spatial-implementation-plan §3.1): the pack latch,
/// every entry kind's applier, and the first-boot apply of the starter area.
///
/// It composes **LAST** — after the NPC module and the rulesets, after the TacSim
/// layer — because its boot pass applies packs whose entries are made by other
/// modules' appliers (`mush-atlas` for places, `mush-npc` for people), and each of
/// those writes its own RAM slot as it goes; `on_world_loaded` runs in list order,
/// so every owner must have hydrated before this pass or the owner's own boot would
/// overwrite what the pass wrote. Registration order does not matter to it: it
/// registers one state slot and nothing else.
///
/// `starter_area` is `[game].starter_area`: `true` ships the three starter packs
/// as boot packs, applied once on a world that has never latched them (doc 14
/// §3.1 P9: zero LLM calls; the packs are bytes); `false` composes the module with
/// no boot packs, so the world stays Limbo and God.
///
/// # Errors
///
/// A kind two owners both claimed (a programming error at composition), or the
/// shipped packs failing to load (they are digest-pinned; see [`atlas_module`]).
pub fn content_module(starter_area: bool) -> Result<Box<dyn GameModule>, String> {
    content_module_with(starter_area, false)
}

/// The production boot list, in apply order (doc 18 §4.7): the starter area's three
/// when `starter_area`, then — when `game_content` — the proving ground's two, the
/// ten station packs (the corridor's four and doc 19 cut A.8's six) and (doc 18 cut
/// IC2) the lane: the game's **thirteen** boot packs.
///
/// The stations are game content: they ride `game_content` (`[tacsim].enabled`),
/// whatever `[game].starter_area` says, and come after the proving ground because
/// nothing earlier names them and the IC door's Limbo end needs no pack. The lane
/// comes after the stations (its places are the corridor's domain hosts, which
/// cut IC3 ties to the stations' berth galleries). A world latched on the old list
/// (dev1: the starter's three and the proving ground's two) takes the rest at its
/// next boot — the latch applies every boot pack it has never latched, at any boot
/// (`mush-content/src/module.rs:100-152`).
///
/// # Errors
///
/// A shipped pack that does not verify (every one is digest-pinned).
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
    }
    Ok(packs)
}

/// [`content_module`] with the game's own boot packs (**16b cut C.2, C3**; the
/// game's six boot packs since doc 18 cut IC1, seven since cut IC2 and thirteen
/// since doc 19 cut A.8 — [`boot_pack_list`]).
///
/// `game_content` is `[tacsim].enabled`: `true` appends the two packs that
/// instantiate the proving ground — the atlas region carrying its ground, then
/// the arena entry whose applier makes the muster room, its domain and the twelve
/// stock tanks — **after** the lot's three, since the latch applies boot packs in
/// order and the arena's own `requires` is empty by design (it names atlas
/// records, and the latch resolves `requires` against content latches only, so
/// an atlas dependency expressed there would be unsatisfiable from the tacsim
/// side; the applier checks the records instead and refuses naming the entry).
///
/// They are boot packs **and** shelf entries (**16b cut Z.3**; the shelf is
/// [`atlas_module`]'s). Boot packs, because doc 16 §3.3's tanks are *persistent
/// objects created once by the centre's content* — first-boot instantiation is
/// exactly what `mush_content`'s latch is, and it is what makes "a second boot
/// creates no thirteenth" a property of the latch rather than of the applier's
/// own bookkeeping; the boot pass is unchanged, so a first boot applies and a
/// latched world logs the one line saying a newer version is available. Shelf
/// entries, because that line is all a boot pass will ever do for a standing
/// world, and a pack whose new version adds tanks has to reach one: staff take
/// it with `@atlas/pack plan` and `@atlas/pack apply`, with the centre idle
/// (`deploy/README.md`), and the arena's applier refuses while a match holds a
/// roster tank. Z.3's version 3, which adds slots 5 and 6 a side, is the first
/// version that needs it.
///
/// # Errors
///
/// As [`content_module`], plus a shipped game pack that does not verify (they
/// are digest-pinned in `crates/tacsim/tests/content.rs`).
pub fn content_module_with(
    starter_area: bool,
    game_content: bool,
) -> Result<Box<dyn GameModule>, String> {
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).map_err(|e| format!("atlas appliers: {e}"))?;
    mush_npc::content::register_appliers(&mut appliers)
        .map_err(|e| format!("npc appliers: {e}"))?;
    // S9.1: a TacSim campaign-seed pack's places and routes are the atlas's
    // sites and connections, applied through the content latch like any pack.
    // 16b cut C.2 adds the `arena` kind to the same registration.
    crate::register_appliers(&mut appliers).map_err(|e| format!("tacsim appliers: {e}"))?;
    let mut module = mush_content::ContentModule::new().with_appliers(appliers);
    // Doc 18 cut IC1: then the station packs (ten since doc 19 cut A.8) after the
    // proving ground's two, and (cut IC2) the lane after them — the game's thirteen
    // boot packs — from the one list the tests read too.
    let packs = boot_pack_list(starter_area, game_content)?;
    // A kept Ceres v1 world must remain bootable so staff can explicitly take
    // Ceres v2 before installing the relay seed. Fresh/current worlds seed now.
    if game_content {
        for pack in crate::relay_content::packs()? {
            module = module.wait_for_dependency_upgrades(pack.manifest().id().clone());
        }
    }
    if !packs.is_empty() {
        module = module.with_boot_packs(packs);
    }
    Ok(Box::new(module))
}

/// The **disabled** recovery policy, for a composition that carries no TacSim layer.
///
/// It exists so a caller outside this crate can build the module list without naming
/// `mush_campaign`'s type — and therefore without taking an edge on `mush-campaign`
/// merely to say "no recovery here". Fail-closed by construction.
#[must_use]
pub fn no_recovery() -> mush_campaign::RecoveryConfig {
    mush_campaign::RecoveryConfig::default()
}

/// **E7 residue pass 1, R4 — the `[tacsim.recovery]` block, mapped for the campaign
/// layer.**
///
/// The composition root does the mapping so neither crate takes an edge on the other:
/// `tacsim` owns the frozen block, `mush-campaign` owns the provider, and
/// this is the one line between them (`IndustryModule::with_recipes`' exact shape).
#[must_use]
pub fn recovery_policy(cfg: &crate::TacsimRecoveryConfig) -> mush_campaign::RecoveryConfig {
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

/// The M7 worldgen module, backed by the shared [`LlmService`] handle.
///
/// `main.rs` inserts it into the boot list **after** [`server_modules`] (so it
/// registers after the built-ins) and before the config-selected
/// [`sheet_modules`]. It registers regardless of whether the LLM is enabled:
/// with `[llm].enabled = false` the service's kill switch is engaged, so
/// `@gen` still parses and defers, and the off-loop generator re-enters a
/// "disabled" outcome (a keyless clone boots and its worldgen commands answer
/// cleanly rather than 404). Its persistence namespace (`worldgen`) is disjoint
/// from every other module's, and it loads its drafts through the same
/// boot-data path as the rest (`mush_server::modules::boot_modules`).
#[must_use]
pub fn worldgen_module(llm: Arc<LlmService>) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::new(llm))
}

/// [`worldgen_module`] with an explicit worldgen config (`[worldgen]` knobs —
/// the M10 `@gen/building` room cap). `main.rs` uses this so `[worldgen]` in
/// `mushd.toml` reaches the module; tests take the crate defaults via
/// [`worldgen_module`].
#[must_use]
pub fn worldgen_module_with(
    llm: Arc<LlmService>,
    config: mush_worldgen::WorldgenConfig,
) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::with_config(llm, config))
}

/// The M8 NPC agent module. Registered after worldgen and before the rulesets.
/// `grounding` supplies the ruleset sheet summary + ability tools through the
/// generic [`RulesetAgent`] (whichever ruleset is enabled); `None` runs NPCs
/// dialogue-only.
#[must_use]
pub fn npc_module(grounding: Option<Arc<dyn mush_npc::AgentGrounding>>) -> Box<dyn GameModule> {
    npc_module_with(grounding, &mush_config::config::NpcCfg::default())
}

/// H2b: [`npc_module`] with the `[npc]` section applied (the scene-turn
/// decision deadline). `main.rs` and the forked server use this.
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

/// The NPC grounding adapter over the one game's sheet (16b cut R-b): every
/// production composition grounds its NPCs through the generic [`RulesetAgent`]
/// (sheet summary, the `check` ability, `@agent/sheet`). `main.rs` passes it to
/// [`npc_module`]. It is `Some` on every server; the `Option` is the seam's shape
/// for a host that deliberately composes dialogue-only NPCs.
#[must_use]
pub fn npc_agent_adapter() -> Option<Arc<dyn mush_npc::AgentGrounding>> {
    Some(Arc::new(RulesetAgent::new(
        crate::sheet::PilotRuleset::new(),
    )))
}

/// A generic NPC↔ruleset grounding adapter (treaty ruling 6) over any
/// [`Ruleset`], running **on the world loop**. This is the seam that carries the
/// milestone's "the plugin API generalises past WoD" claim end-to-end: the NPC
/// host dispatches grounding through the [`Ruleset`] trait, so any enabled
/// ruleset grounds its NPCs with no NPC-wiring change — the earlier
/// `WodAgent` hardcoded the concrete `WodRuleset`, which left a shadowrun-only
/// server's NPCs dialogue-only despite SR5 fully implementing the seam.
///
/// - [`profile`](mush_npc::AgentGrounding::profile) reads the NPC's `sheet:<id>`
///   document (via a [`SheetStore`](mush_ruleset::SheetStore) over the cloned
///   schema) and grounds it via [`Ruleset::agent_profile`].
/// - [`invoke_ability`](mush_npc::AgentGrounding::invoke_ability) executes a
///   validated ability (`wod_roll`, `sr_roll`, …) via
///   [`Ruleset::invoke_ability`], which reads the NPC's own sheet, rolls on the
///   shared server RNG, and emits to the room.
/// - [`init_sheet`](mush_npc::AgentGrounding::init_sheet) backs `@agent/sheet`
///   via [`Ruleset::init_sheet`].
///
/// The held ruleset is grounding-only — it reads each NPC's own sheet from the
/// world and rolls on the shared world state, carrying no per-character state.
pub struct RulesetAgent<R: Ruleset> {
    ruleset: R,
    schema: Arc<mush_ruleset::schema::SheetSchema>,
}

impl<R: Ruleset> RulesetAgent<R> {
    /// A grounding adapter over `ruleset`, capturing its sheet schema for reads.
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

/// Read and partition one namespace without invoking module-supplied boot
/// callbacks. Typed state must be validated globally before callbacks run.
/// How a namespace is fetched (J4, doc 12 §5.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// Honour the namespace's boot manifest, if it has one: skip the ledger
    /// chunks it names.
    Narrowed,
    /// The whole namespace, manifest ignored — the retry after a module
    /// answered [`mush_core::ModError::RetryFullFetch`], and every first boot's shape
    /// before J4.
    Whole,
}
