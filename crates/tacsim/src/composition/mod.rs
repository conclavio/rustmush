//! The production composition root (16b cut R-a, R2; formerly `mush-server/src/modules.rs`):
//! which [`GameModule`]s the server boots with, their
//! persisted-data load path, and the per-namespace module migrations.
//!
//! Boot order (driven from `main.rs` / test rigs via [`boot_modules`]):
//!
//! 1. [`Engine::compose_modules`] has already run every module's `register`
//!    (commands, hooks, ticks, state slots) against the one registry and
//!    frozen mechanics before world hydration; [`Engine::from_composition`]
//!    then installed the result on the loaded world.
//! 2. [`run_module_migrations`] brings each registered namespace's
//!    persisted documents up to its module's current schema (BGSAVE first —
//!    the mandatory pre-migrate snapshot), **before** any module parses
//!    them.
//! 3. The boot loader partitions the coordinator-reserved `action-state/*`
//!    documents away from ordinary module data and atomically hydrates them
//!    through their registered typed decoders.
//! 4. The loader supplies every remaining kv doc (minus the
//!    reserved [`SCHEMA_KEY`]) plus the streams the module names via
//!    [`GameModule::boot_stream_requests`].
//! 5. [`Engine::modules_loaded`] hands each module its
//!    [`ModuleBootData`]; whatever the modules persist during boot
//!    (first-boot seeding, orphan-doc cleanup) is drained and written
//!    through **durably, acked** before the server starts serving.
//!
//! The module list itself is [`server_modules`]: comsys first (its
//! [`AliasStage`](mush_core::AliasStage) is the earliest alias stage in
//! dispatch), then canon, org, faction, scenes, simulation, tactics, economy,
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
pub mod sim_routes;
pub mod industry_recipes;
pub mod campaign_formations;
pub mod salvage_source;
pub mod seat_audit;
pub mod econ_inbox;
pub mod econ_canon;
pub mod econ_alerts;

use std::sync::Arc;

use mush_core::GameModule;
use mush_llm::LlmService;
use mush_ruleset::Ruleset;
use mush_worldgen::WorldgenModule;
use smol_str::SmolStr;

/// Reserved kv key per module namespace carrying `{"version": N}` — the
/// namespace's schema version for [`run_module_migrations`]. Filtered out
/// of [`ModuleBootData::kv`] so modules never see it.
pub const SCHEMA_KEY: &str = "schema";

/// The built-in game modules a production `mushd` boots with, in
/// registration order, with **default** module config. Config-selected
/// [`ruleset_modules`] are appended **after** these (see `main.rs`); hand the
/// concatenation to [`Engine::with_modules`].
#[must_use]
pub fn server_modules() -> Vec<Box<dyn GameModule>> {
    server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        // S1: default `[atlas]` limits, an empty shelf but for the shipped packs.
        mush_atlas::AtlasConfig::default(),
        &mush_core::OnboardingConfig::default(),
        // `[rulesets].enabled` defaults to `["wod"]`, so the default composition is a
        // WoD server and the housing title issuer stays wired exactly as E5b wired
        // it. A host that enables no WoD ruleset passes `None` (see
        // [`server_modules_with`]).
        Some(&ruleset_wod::WodEconomyConfig::default()),
        None,
        // E7r1 R4: the default composition composes no TacSim layer, so the provider
        // is disabled — `@recovery` registers and answers "not enabled".
        mush_campaign::RecoveryConfig::default(),
    )
}

/// [`server_modules`] with explicit module configs: the `[weather]` knobs (the M10
/// flavor-variant sizing), the `[economy]` knobs (E4's mint bound on a ruleset's
/// faucet-funded obligations), `[onboarding]`'s start rooms (E5b's reserved-room
/// list), and `[wod.economy]` (E5c: whether the housing title issuer is registered at
/// all, plus any extra reserved rooms the ruleset names). `main.rs` uses this so
/// those sections in `mushd.toml` actually reach their modules; every other caller
/// (tests) takes the crate defaults via [`server_modules`].
///
/// `wod` is `None` when `[rulesets].enabled` does not select the WoD ruleset. That
/// is what 08d §2's "the server registers `system:wod.housing` **only when the WoD
/// ruleset is configured**" means in code, and it replaces E5b's unconditional
/// wiring: a Shadowrun-only server no longer carries a title issuer for a ruleset it
/// does not load.
#[must_use]
pub fn server_modules_with(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    atlas: mush_atlas::AtlasConfig,
    onboarding: &mush_core::OnboardingConfig,
    wod: Option<&ruleset_wod::WodEconomyConfig>,
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
    recovery: mush_campaign::RecoveryConfig,
) -> Vec<Box<dyn GameModule>> {
    // The issuer is registered when the WoD ruleset is configured **and** its housing
    // knob is on. `issue_title` refuses an issuer that is not on both the economy's
    // list and `HousingModule::with_issuer`, so the two are computed once, here, and
    // a mismatch is impossible by construction.
    let housing_on = wod.is_some_and(|w| w.housing);
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
    let market_on = shadowrun.is_some_and(|s| s.enabled);
    if market_on {
        issuers.push(market_title_issuer());
    }
    // E7 cut 3: the claim's titled object is minted through
    // `mush_economy::issue_title` under this crate's own system principal, which
    // must be named in BOTH this list and `IndustryModule`'s own
    // `industry_title_issuer()` or the verb refuses loudly. It is unconditional:
    // `mush-industry` composes on every ruleset.
    issuers.push(mush_industry::industry_title_issuer());
    let issuer = housing_on.then(housing_title_issuer);
    let market_issuer = market_on.then(market_title_issuer);
    // E7 cut 3 B1: the composition root's own wire between two modules whose
    // order it already fixes. `mush-inventory` fills it while registering;
    // `mush-industry`, composed after it, takes the handles out.
    let inventory_handles: mush_inventory::InventoryHandleCell = Arc::default();
    // `[onboarding]`'s two start rooms plus Limbo (E5b fold H4), extended by whatever
    // the ruleset knows is never somebody's living room (E5b's handoff to E5c).
    let reserved: Vec<mush_core::ObjId> = [
        onboarding.start_room,
        onboarding.start_room_unapproved,
        mush_core::LIMBO,
    ]
    .into_iter()
    .chain(
        wod.into_iter()
            .flat_map(|w| w.reserved_rooms.iter().copied()),
    )
    .collect();
    vec![
        Box::new(mush_comsys::ComsysModule::new()),
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
        Box::new(
            mush_sim::SimulationModule::new()
                .with_route_catalog(Arc::new(self::sim_routes::TacsimRouteCatalog)),
        ),
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
        Box::new(
            mush_tactics::TacticsModule::new()
                .with_sensor_policy(Arc::new(mush_propagation::PropagationSensorPolicy)),
        ),
        // E7 cut 7 (10b §3.7): the propagation layer itself. It composes AFTER
        // `mush-sim` (whose `SpatialState` every context is built from) and
        // after `mush-tactics` (whose leak-safe contact ledger its policy
        // reads); `on_world_loaded` runs in list order, so both are hydrated
        // before propagation's boot. It registers NO contact family and NO
        // contact guard validator — `mush-tactics` is the sole `ContactState`
        // authority (08i answer 1, D-E7-36).
        Box::new(mush_propagation::PropagationModule::new()),
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
        Box::new(
            // E7 cut 3: `mush-industry` registers a `CustodyProvider` for its own
            // `CustodyRef::Registered { provider: "industry", .. }` backings, on
            // E3's rules unchanged (exactly one claiming provider,
            // validate-then-mutate). It needs no module ordering of its own:
            // a registered backing has no world object, so `backing_valid`
            // answers without reading any state and cannot boot `Frozen` the way
            // an interior would.
            mush_economy::EconomyModule::with_custody(vec![
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
            .with_facts(vec![
                Arc::new(mush_jobs::JobsFactProvider),
                Arc::new(mush_inventory::InventoryFactProvider),
                Arc::new(mush_industry::IndustryFactProvider),
                Arc::new(mush_campaign::CampaignFactProvider),
                Arc::new(mush_sim::SimFactProvider),
            ])
            // E5b: housing's exact-mode mirror joins the canon bridge. It hears the
            // obligation and contract transitions a lease is made of; abstract mode
            // does not need it at all.
            .with_observers(econ_observers(shadowrun))
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
            .with_party_caps(vendor_caps(shadowrun)),
        ),
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
        Box::new(
            mush_inventory::InventoryModule::new()
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
                .with_quantity_delegates(["industry", "market"])
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
                .with_handle_cell(Arc::clone(&inventory_handles)),
        ),
        // E7 cut 3 (10b §3.3, D-E7-3, doc 10 §14): the neutral industry
        // authority. It composes **immediately after `mush-inventory`** — whose
        // handles and delegation it needs — and **before `mush-market`**, which
        // it does not touch at all: a `Representation::Lot` listing is still
        // refused at creation and lifting that refusal is cut 11's.
        Box::new(
            mush_industry::IndustryModule::new()
                .with_inventory(Arc::clone(&inventory_handles))
                // E7c4 B2: the recipe authority reaches industry by composition,
                // not by an edge — the impl reads `TacsimStore` at the root.
                .with_recipes(Arc::new(self::industry_recipes::TacsimRecipeAuthority)),
        ),
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
        Box::new(mush_mail::MailModule),
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
                .with_catalog_sources(market_catalog_sources(shadowrun))
                .with_availability(market_availability(shadowrun))
                .with_delivery(market_delivery(shadowrun))
                // E7r2 P1 (§15.5 (b′)): `IndustryModule::with_inventory`'s shape
                // exactly. It carries a **capability**; the authority to write with it
                // is `with_quantity_delegates(["industry", "market"])` above, and
                // without that name the very first `settle_sale` would answer
                // `ActionStateError::OwnerMismatch`. Unwired is fail-closed: no lot
                // action registers and `@market/vendor stock` refuses a
                // lot-represented good byte-identically to E6c.
                .with_inventory(Arc::clone(&inventory_handles));
            // WS04-S7: housing's shape at the top of this list, for housing's reason.
            // Wired unconditionally, `sr.market` was a name the market held and the
            // economy did not list on every non-Shadowrun server, so the one arm that
            // could reach `issue_title` answered `Unauthorized` — a permission
            // refusal for a capability the operator never asked for — instead of the
            // market's own "this server registers no market title issuer".
            match market_issuer {
                Some(i) => m.with_issuer(i),
                None => m,
            }
        }),
        Box::new(mush_script::ScriptModule::new()),
    ]
}

/// The wiring-frozen party capabilities (E6b, DL-11 / M15).
///
/// Empty unless the Shadowrun economic layer is **enabled** and the operator listed
/// organization ids in `[shadowrun.economy].vendor_orgs` — the fail-closed default is
/// that no principal is programmatically acceptable, which is exactly E6a.
fn vendor_caps(
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
) -> Vec<(mush_org::PrincipalId, mush_economy::PartyCap)> {
    shadowrun
        .filter(|s| s.enabled)
        .map(|s| {
            s.vendor_orgs
                .iter()
                .map(|id| {
                    (
                        mush_org::PrincipalId::Organization(mush_org::OrgId(*id)),
                        mush_economy::PartyCap::Accept,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The market's content-pack sources (E6c B9). Empty unless the Shadowrun economic
/// layer is enabled: a server with no ruleset content loads no pack, which is the
/// fail-closed default.
fn market_catalog_sources(
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
) -> Vec<Arc<dyn mush_market::CatalogSource>> {
    shadowrun
        .filter(|s| s.enabled)
        .map(|s| {
            let source: Arc<dyn mush_market::CatalogSource> =
                Arc::new(ruleset_shadowrun::SrCatalogSource::new(s));
            vec![source]
        })
        .unwrap_or_default()
}

/// The market's availability engines (E6c B7). `mush-market` holds the rule shape and
/// the recorded outcome and **rolls nothing**; the dice are the ruleset's.
fn market_availability(
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
) -> Vec<Arc<dyn mush_market::AvailabilityProvider>> {
    shadowrun
        .filter(|s| s.enabled)
        .map(|_| {
            let p: Arc<dyn mush_market::AvailabilityProvider> =
                Arc::new(ruleset_shadowrun::SrAvailability::new());
            vec![p]
        })
        .unwrap_or_default()
}

/// The market's delivery providers (E6c B6). A `Representation::Asset` needs none —
/// its title moves as an ordinary E4b2 deliverable inside the settlement.
fn market_delivery(
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
) -> Vec<Arc<dyn mush_market::DeliveryProvider>> {
    shadowrun
        .filter(|s| s.enabled)
        .map(|_| {
            let p: Arc<dyn mush_market::DeliveryProvider> = Arc::new(ruleset_shadowrun::SrDelivery);
            vec![p]
        })
        .unwrap_or_default()
}

/// The economy observer table this server freezes at wiring.
///
/// The Shadowrun board's mirror joins it **only when the Shadowrun economic layer is
/// configured** (08f §2's `issuers` pattern): an observer that fires on every
/// contract transition on a WoD server would be a table entry doing nothing, and the
/// board it mirrors would not exist.
fn econ_observers(
    shadowrun: Option<&ruleset_shadowrun::SrEconomyConfig>,
) -> Vec<Arc<dyn mush_economy::EconomyObserver>> {
    let mut out: Vec<Arc<dyn mush_economy::EconomyObserver>> = vec![
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
        // projection folds, a cancellation or expiry back into stock. It is
        // unconditional because `mush-market` is game-neutral and a WoD or TacSim
        // server's market is the same market.
        Arc::new(mush_market::MarketObserver),
    ];
    if let Some(sr) = shadowrun.filter(|s| s.enabled) {
        // E6a (D-E6-5): mirrors `ContractChanged` onto the run board and releases the
        // crewed party through `mush_party`'s own seam; drives the lifestyle
        // miss/downgrade rule from `ObligationChanged`.
        out.push(Arc::new(ruleset_shadowrun::SrBoardObserver::new(
            sr.clone(),
        )));
    }
    out
}

/// The system principal housing mints property titles under (08d §2, D-E3-13).
///
/// It is named in exactly two places — `EconomyModule::with_title_issuers` and
/// `HousingModule::with_issuer` — and `mush_economy::issue_title` refuses an issuer
/// that is not on the economy's list, so a typo in either one is a loud refusal at
/// `@property/create` rather than a silent capability.
#[must_use]
pub fn housing_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("wod.housing")
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
    mush_org::SystemPrincipalId::new("sr.market")
        .expect("a built-in system principal id is well-formed")
}

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
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| mush_org::CapabilityId::new(c).expect("a built-in capability id is well-formed"))
        .collect()
}

/// The ruleset module(s) named in `[rulesets].enabled`, in listed order.
///
/// Every ruleset crate is compiled into `mushd`; this maps the configured
/// ids to their modules. Registered **after** [`server_modules`] so a
/// ruleset's commands, panel feeds, and job queues wire cleanly and its
/// approval payload handlers (registered in `on_world_loaded`) run after the
/// jobs module's state slot exists.
///
/// A ruleset's sheets ride the object attr write-through (treaty ruling 7),
/// so they load with the world — a ruleset registers no boot namespace and
/// needs no separate boot-data load here.
///
/// An unknown id is a hard error (consistent with the rest of config: a
/// silently-ignored typo is worse than not starting). The ids available as of
/// M10b are `"wod"` (oWoD20) and `"shadowrun"` (SR5); duplicates are rejected.
///
/// **Single-ruleset-per-server.** Every ruleset registers the same core
/// command surface (`+roll`/`+sheet`/`+chargen`/…), so two of them cannot share
/// one registry — enabling more than one is a configuration error caught here
/// with a clear message (rather than surfacing downstream as an opaque
/// command-collision at registration). Enable exactly one ruleset, or none (a
/// bare comsys/mail server). Per-character ruleset dispatch across systems is a
/// future concern.
/// S6: the rules-interface module for `rs`: its display name and its chargen
/// stages, snapshotted at composition.
fn rules_interface_for<R: mush_ruleset::Ruleset>(
    rs: &R,
) -> mush_ruleset::interface::RulesInterfaceModule {
    let stages = rs.chargen().map_or_else(Vec::new, |c| c.stages());
    mush_ruleset::interface::RulesInterfaceModule::new(rs.display_name().to_string(), stages)
}

pub fn ruleset_modules(
    enabled: &[String],
    onboarding: &mush_core::OnboardingConfig,
    wod: &ruleset_wod::WodEconomyConfig,
    shadowrun: &ruleset_shadowrun::SrEconomyConfig,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut mods: Vec<Box<dyn GameModule>> = Vec::new();
    // S6 (doc 14 §4.6): beside the ruleset, the small module that fronts its
    // chargen stages and the help topics as the off-duty gamemaster's
    // interface; the stages are snapshotted here, before the ruleset is boxed.
    let mut rules_interface: Option<Box<dyn GameModule>> = None;
    for id in enabled {
        let key = id.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("[rulesets].enabled lists {id:?} more than once"));
        }
        match key.as_str() {
            "wod" => {
                let rs = ruleset_wod::WodRuleset::new()
                    .onboarding(onboarding.clone())
                    .economy(wod.clone());
                rules_interface = Some(Box::new(rules_interface_for(&rs)));
                mods.push(Box::new(rs));
            }
            "shadowrun" => {
                // SR5 (M10b). Single-ruleset-per-server: wod and shadowrun both
                // register +roll/+sheet/+chargen, so a server runs one of them.
                // SR5 has no onboarding gate wired yet.
                //
                // E6a: `[shadowrun.economy]` is frozen into the ruleset here, exactly
                // as `[wod.economy]` is above. `enabled = false` off-config, so an
                // unconfigured server keeps its pre-E6a behaviour.
                let rs = ruleset_shadowrun::ShadowrunRuleset::new().economy(shadowrun.clone());
                rules_interface = Some(Box::new(rules_interface_for(&rs)));
                mods.push(Box::new(rs));
            }
            _ => {
                return Err(format!(
                    "[rulesets].enabled: unknown ruleset {id:?} (available: \"wod\", \"shadowrun\")"
                ));
            }
        }
        seen.push(key);
    }
    // Single ruleset per server: two rulesets would collide on the shared
    // +roll/+sheet/+chargen command names. Fail here with an explanation
    // instead of letting Engine::with_modules report an opaque collision.
    if mods.len() > 1 {
        return Err(format!(
            "[rulesets].enabled selects {} rulesets ({enabled:?}), but rustmush runs one \
             ruleset per server — they all register +roll/+sheet/+chargen, so more than one \
             collides. Enable exactly one (e.g. [\"shadowrun\"]) or none.",
            mods.len()
        ));
    }
    if let Some(m) = rules_interface {
        mods.push(m);
    }
    Ok(mods)
}

/// The **TacSim layer** (E7 cut 1, 10b §3.1), composed **after** the character
/// ruleset.
///
/// It is deliberately **not** a `[rulesets].enabled` id. Doc 10 §20: *TacSim supplies
/// physical and civil context, not ruleset metaphysics … a ruleset decides what those
/// facts mean to its characters.* So [`ruleset_modules`] is untouched — it still knows
/// `"wod"` and `"shadowrun"` and still refuses more than one — and this layer composes
/// beside whichever single ruleset is enabled, switched by `[tacsim].enabled` alone.
///
/// **Order position matters and is pinned** (`tacsim_composes_after_the_character_ruleset`).
/// `on_world_loaded` runs in list order, so registering last is what lets TacSim read
/// the character ruleset's facts at boot while that ruleset reads TacSim's
/// environmental facts through the generic seam.
///
/// The module is composed **unconditionally**, and `enabled` gates behaviour rather
/// than existence — E5c deviation 1 and `[shadowrun.economy]`'s precedent: a disabled
/// layer answers "not enabled on this server" instead of hiding a verb, which is also
/// what keeps `@tacsim` visible to the rule-1 allowlist tripwire in a default
/// composition.
#[must_use]
pub fn tacsim_module(cfg: crate::TacsimConfig) -> Box<dyn GameModule> {
    Box::new(crate::TacsimRuleset::with_config(cfg))
}

/// The **atlas** (S1, 14-spatial-implementation-plan §3.1): the spatial records, the
/// rasterizer, the `+atlas` views and `@atlas/pack`, with `[atlas]`'s limits and the
/// operator's packs, and the shipped starter packs on the shelf whatever
/// `[game].starter_area` says — the shelf is what staff plan and apply from, and
/// listing a pack applies nothing.
#[must_use]
pub fn atlas_module(cfg: mush_atlas::AtlasConfig) -> Box<dyn GameModule> {
    let starter = mush_atlas::starter::packs().unwrap_or_else(|error| {
        // The shipped packs are digest-pinned in `mush-atlas`'s own tests; this
        // cannot fail on a tree that passed them, and a shelf without them is the
        // honest degradation if it ever did.
        tracing::error!(%error, "atlas: the shipped starter packs did not load");
        Vec::new()
    });
    Box::new(mush_atlas::AtlasModule::with_config(cfg).with_shelf(starter))
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
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).map_err(|e| format!("atlas appliers: {e}"))?;
    mush_npc::content::register_appliers(&mut appliers)
        .map_err(|e| format!("npc appliers: {e}"))?;
    // S9.1: a TacSim campaign-seed pack's places and routes are the atlas's
    // sites and connections, applied through the content latch like any pack.
    crate::register_appliers(&mut appliers)
        .map_err(|e| format!("tacsim appliers: {e}"))?;
    let mut module = mush_content::ContentModule::new().with_appliers(appliers);
    if starter_area {
        let packs = mush_atlas::starter::packs().map_err(|e| format!("starter packs: {e}"))?;
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
pub fn recovery_policy(
    cfg: &crate::TacsimRecoveryConfig,
) -> mush_campaign::RecoveryConfig {
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
/// [`ruleset_modules`]. It registers regardless of whether the LLM is enabled:
/// with `[llm].enabled = false` the service's kill switch is engaged, so
/// `@gen` still parses and defers, and the off-loop generator re-enters a
/// "disabled" outcome (a keyless clone boots and its worldgen commands answer
/// cleanly rather than 404). Its persistence namespace (`worldgen`) is disjoint
/// from every other module's, and it loads its drafts through the same
/// boot-data path as the rest ([`boot_modules`]).
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

/// Build the NPC↔ruleset grounding adapter for the enabled ruleset, if one
/// grounds NPC agents. `main.rs` passes it to [`npc_module`]. Grounding
/// dispatches through the generic [`RulesetAgent`], so **every** ruleset — not
/// just WoD — grounds its NPCs: a shadowrun-only server grounds SR5 NPCs
/// (sheet summary + `sr_roll` + `@agent/sheet`) exactly as a wod server grounds
/// WoD NPCs. A ruleset-less config leaves NPCs dialogue-only (`None`).
///
/// Single-ruleset-per-server (see [`ruleset_modules`]), so at most one entry
/// matches; the first recognized id wins.
#[must_use]
pub fn npc_agent_adapter(enabled: &[String]) -> Option<Arc<dyn mush_npc::AgentGrounding>> {
    for id in enabled {
        match id.trim().to_ascii_lowercase().as_str() {
            "wod" => {
                return Some(Arc::new(RulesetAgent::new(ruleset_wod::WodRuleset::new())));
            }
            "shadowrun" => {
                return Some(Arc::new(RulesetAgent::new(
                    ruleset_shadowrun::ShadowrunRuleset::new(),
                )));
            }
            _ => {}
        }
    }
    None
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
    /// answered [`ModError::RetryFullFetch`], and every first boot's shape
    /// before J4.
    Whole,
}
