//! M3 module wiring: which [`GameModule`]s the server boots with, their
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

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    ActionStateBootRecord, BOOT_MANIFEST_KEY, BootManifest, Effects, Engine, GameModule, Migration,
    ModError, ModuleBootData, Outbox, is_action_state_persistence_key,
};
use mush_llm::LlmService;
use mush_ruleset::Ruleset;
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, WriteBatch};
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
        &ruleset_wod::OnboardingConfig::default(),
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
    onboarding: &ruleset_wod::OnboardingConfig,
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
    if shadowrun.is_some_and(|s| s.enabled) {
        issuers.push(market_title_issuer());
    }
    // E7 cut 3: the claim's titled object is minted through
    // `mush_economy::issue_title` under this crate's own system principal, which
    // must be named in BOTH this list and `IndustryModule`'s own
    // `industry_title_issuer()` or the verb refuses loudly. It is unconditional:
    // `mush-industry` composes on every ruleset.
    issuers.push(mush_industry::industry_title_issuer());
    let issuer = housing_on.then(housing_title_issuer);
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
                .with_route_catalog(Arc::new(crate::sim_routes::TacsimRouteCatalog)),
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
            .with_canon_evidence(Arc::new(crate::econ_canon::CanonEvidenceReader))
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
                // `mush-inventory` and implemented in `crate::salvage_source`, the
                // one crate with edges to both — `sim_routes::TacsimRouteCatalog`'s
                // shape exactly, and for exactly the same reason. Naming it here is
                // what makes `@salvage` exist at all: an unwired composition
                // registers no salvage action and refuses byte-identically to a
                // wreck that does not exist.
                .with_salvage_source(Arc::new(crate::salvage_source::SimSalvageSource))
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
                .with_recipes(Arc::new(crate::industry_recipes::TacsimRecipeAuthority)),
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
                    crate::campaign_formations::TacsimFormationAuthority,
                ))
                // E7 cut 10 B3 (OPEN 4): the server-side, audit-only seat
                // attribution log, on the same composition seam. The row
                // linking a driver token to an account NEVER becomes a
                // world document, a journal line or a canon record; one
                // staff verb asks whether a token is attributed, and it
                // prints no id. Unwired is fail-closed.
                .with_seat_audit(Arc::new(crate::seat_audit::SeatAuditLog::new()))
                // E7 residue pass 1, R4/R3: `[tacsim.recovery]`, mapped from the
                // ruleset's frozen block by [`recovery_policy`] on this same
                // composition seam. The campaign crate takes no `ruleset-tacsim`
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
        Box::new(
            mush_market::MarketModule::new()
                .with_catalog_sources(market_catalog_sources(shadowrun))
                .with_availability(market_availability(shadowrun))
                .with_delivery(market_delivery(shadowrun))
                .with_issuer(market_title_issuer())
                // E7r2 P1 (§15.5 (b′)): `IndustryModule::with_inventory`'s shape
                // exactly. It carries a **capability**; the authority to write with it
                // is `with_quantity_delegates(["industry", "market"])` above, and
                // without that name the very first `settle_sale` would answer
                // `ActionStateError::OwnerMismatch`. Unwired is fail-closed: no lot
                // action registers and `@market/vendor stock` refuses a
                // lot-represented good byte-identically to E6c.
                .with_inventory(Arc::clone(&inventory_handles)),
        ),
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
        Arc::new(crate::econ_canon::CanonEvidenceBridge),
        Arc::new(mush_housing::HousingEconomyObserver),
        // E6a step 1 (DL-04): an organization's officers are told, kinds only, what
        // its contracts and obligations did.
        Arc::new(crate::econ_inbox::OrgInbox),
        // E6a step 1 (DL-05): the ladder's pages reach connected wizards on the
        // ladder's own re-notify schedule, deduped per step.
        Arc::new(crate::econ_alerts::StaffAlerts::default()),
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
/// `@market/vendor stock`'s own call — and `mush_economy::issue_title` refuses an
/// issuer that is not on the economy's list, so a typo is a loud refusal rather than
/// a silent capability. Registration, not code.
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
pub fn ruleset_modules(
    enabled: &[String],
    onboarding: &ruleset_wod::OnboardingConfig,
    wod: &ruleset_wod::WodEconomyConfig,
    shadowrun: &ruleset_shadowrun::SrEconomyConfig,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut mods: Vec<Box<dyn GameModule>> = Vec::new();
    for id in enabled {
        let key = id.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("[rulesets].enabled lists {id:?} more than once"));
        }
        match key.as_str() {
            "wod" => {
                mods.push(Box::new(
                    ruleset_wod::WodRuleset::new()
                        .onboarding(onboarding.clone())
                        .economy(wod.clone()),
                ));
            }
            "shadowrun" => {
                // SR5 (M10b). Single-ruleset-per-server: wod and shadowrun both
                // register +roll/+sheet/+chargen, so a server runs one of them.
                // SR5 has no onboarding gate wired yet.
                //
                // E6a: `[shadowrun.economy]` is frozen into the ruleset here, exactly
                // as `[wod.economy]` is above. `enabled = false` off-config, so an
                // unconfigured server keeps its pre-E6a behaviour.
                mods.push(Box::new(
                    ruleset_shadowrun::ShadowrunRuleset::new().economy(shadowrun.clone()),
                ));
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
pub fn tacsim_module(cfg: ruleset_tacsim::TacsimConfig) -> Box<dyn GameModule> {
    Box::new(ruleset_tacsim::TacsimRuleset::with_config(cfg))
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
/// `ruleset-tacsim` owns the frozen block, `mush-campaign` owns the provider, and
/// this is the one line between them (`IndustryModule::with_recipes`' exact shape).
#[must_use]
pub fn recovery_policy(
    cfg: &ruleset_tacsim::TacsimRecoveryConfig,
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
enum Fetch {
    /// Honour the namespace's boot manifest, if it has one: skip the ledger
    /// chunks it names.
    Narrowed,
    /// The whole namespace, manifest ignored — the retry after a module
    /// answered [`ModError::RetryFullFetch`], and every first boot's shape
    /// before J4.
    Whole,
}

/// What a namespace's boot manifest lets the loader skip (the chunk floors
/// and the key prefixes, doc 13 W1), or nothing: no manifest, an unreadable
/// one (said at `warn`), or a format this loader does not know.
async fn manifest_skip(store: &dyn ObjectStore, ns: &str) -> Result<mush_store::BootSkip, String> {
    let Some(doc) = store
        .module_json_get(ns, BOOT_MANIFEST_KEY)
        .await
        .map_err(|e| format!("reading {ns} boot manifest: {e}"))?
    else {
        return Ok(mush_store::BootSkip::default());
    };
    match serde_json::from_value::<BootManifest>(doc) {
        Ok(manifest) => {
            if manifest.format != mush_core::BOOT_MANIFEST_FORMAT {
                tracing::warn!(
                    ns,
                    format = manifest.format,
                    "boot manifest at an unknown format; fetching the namespace whole"
                );
            }
            Ok(mush_store::BootSkip {
                floors: manifest
                    .floors()
                    .into_iter()
                    .map(|(log, below)| (log.to_string(), below))
                    .collect(),
                prefixes: manifest
                    .prefixes()
                    .into_iter()
                    .map(|p| p.to_string())
                    .collect(),
            })
        }
        Err(error) => {
            tracing::warn!(ns, %error, "boot manifest unreadable; fetching the namespace whole");
            Ok(mush_store::BootSkip::default())
        }
    }
}

async fn load_one_namespace(
    store: &dyn ObjectStore,
    ns: &str,
    fetch: Fetch,
) -> Result<(ModuleBootData, Vec<ActionStateBootRecord>), String> {
    // J1 / doc 12 D-12-3: the fetch is timed and handed to the module beside
    // its data — the part of a boot the replay timer never saw.
    let started = std::time::Instant::now();
    // J4: the manifest first, then a listing that never fetches what it names.
    // A whole fetch (the retry) still honours the manifest's key prefixes
    // (doc 13 W1): those documents are read on demand and never at boot,
    // whatever the checkpoint's state.
    let mut skip = manifest_skip(store, ns).await?;
    if fetch == Fetch::Whole {
        skip.floors.clear();
    }
    let documents = store
        .module_docs_from(ns, &skip)
        .await
        .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
    let fetch_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut kv = Vec::new();
    let mut action_state = Vec::new();
    for (key, value) in documents.docs {
        // Loader-owned protocol documents never reach the module.
        if key == SCHEMA_KEY || key == BOOT_MANIFEST_KEY {
            continue;
        }
        if is_action_state_persistence_key(&key) {
            action_state.push(ActionStateBootRecord::new(ns, key, value));
        } else {
            kv.push((SmolStr::new(key), value));
        }
    }
    let ledger = documents
        .ledger
        .into_iter()
        .map(|(key, text)| (SmolStr::new(key), text))
        .collect();
    let omitted = documents
        .omitted
        .into_iter()
        .map(|(log, below)| (SmolStr::new(log), below))
        .collect();
    if !skip.is_empty() {
        tracing::info!(
            ns,
            floors = ?skip.floors,
            prefixes = ?skip.prefixes,
            fetch_ms,
            "namespace fetched narrowed to its boot manifest"
        );
    }
    Ok((
        ModuleBootData {
            kv,
            ledger,
            streams: HashMap::new(),
            fetch_ms,
            omitted,
        },
        action_state,
    ))
}

/// Only after typed state has hydrated, let modules select their bounded
/// stream tails from already-partitioned ordinary JSON data.
async fn load_requested_streams(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
    boot: &mut HashMap<SmolStr, ModuleBootData>,
) -> Result<(), String> {
    for module in modules {
        let data = boot
            .get_mut(module.name())
            .expect("every composed module namespace was loaded in the first pass");
        load_streams_for(store, module.as_ref(), data).await?;
    }
    Ok(())
}

/// [`load_requested_streams`] for one module's data (the J4 retry refetches
/// one namespace and needs its streams again).
async fn load_streams_for(
    store: &dyn ObjectStore,
    module: &dyn GameModule,
    data: &mut ModuleBootData,
) -> Result<(), String> {
    let ns = module.name();
    let mut streams = HashMap::new();
    let stream_requests = module
        .boot_stream_requests(&data.kv)
        .map_err(|e| format!("validating module stream requests for {ns:?}: {e}"))?;
    for (key, count) in stream_requests {
        let entries = store
            .stream_range(ns, &key, count)
            .await
            .map_err(|e| format!("loading module stream {ns}:{key}: {e}"))?;
        streams.insert(key, entries);
    }
    data.streams = streams;
    Ok(())
}

/// Load only ordinary module boot data. The full public boot path is
/// [`boot_modules`], which also hydrates the reserved typed-state partition.
#[cfg(test)]
async fn load_module_boot(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<HashMap<SmolStr, ModuleBootData>, String> {
    let (mut boot, _) = load_module_boot_with_action_state(store, modules).await?;
    load_requested_streams(store, modules, &mut boot).await?;
    Ok(boot)
}

async fn load_module_boot_with_action_state(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<(HashMap<SmolStr, ModuleBootData>, Vec<ActionStateBootRecord>), String> {
    let mut boot = HashMap::new();
    let mut action_state = Vec::new();
    for module in modules {
        let (module_boot, mut module_action_state) =
            load_one_namespace(store, module.name(), Fetch::Narrowed).await?;
        boot.insert(SmolStr::new(module.name()), module_boot);
        action_state.append(&mut module_action_state);
    }
    Ok((boot, action_state))
}

/// The schema version a namespace's [`SCHEMA_KEY`] doc claims, if any.
async fn ns_version(store: &dyn ObjectStore, ns: &str) -> Result<Option<u32>, String> {
    let doc = store
        .module_json_get(ns, SCHEMA_KEY)
        .await
        .map_err(|e| format!("reading {ns} schema doc: {e}"))?;
    Ok(doc
        .and_then(|d| d.get("version").and_then(serde_json::Value::as_u64))
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX)))
}

/// Write one batch of module ops through the store, ack awaited (boot runs
/// before the world task, so it writes directly).
async fn write_ops(store: &dyn ObjectStore, ops: Vec<ModuleOp>) -> Result<(), String> {
    let batch = WriteBatch {
        serial: BOOT_SERIAL,
        durable: true,
        module_ops: ops,
        ..WriteBatch::default()
    };
    write_batch(store, batch).await
}

/// Commit one boot-time batch, durably, checking the receipt's serial.
async fn write_batch(store: &dyn ObjectStore, batch: WriteBatch) -> Result<(), String> {
    let receipt = store
        .write(batch)
        .await
        .map_err(|e| format!("module boot write: {e}"))?;
    if receipt.serial != BOOT_SERIAL {
        return Err(format!(
            "module boot write: expected receipt serial {BOOT_SERIAL}, got {}",
            receipt.serial
        ));
    }
    Ok(())
}

/// A `JsonSet` stamping a namespace's schema version.
fn stamp(ns: &str, version: u32) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(ns),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new(SCHEMA_KEY),
            value: serde_json::json!({ "version": version }),
        },
    }
}

/// Run every module's registered per-namespace migrations (collected via
/// `Registrar::migrations`, exposed by
/// [`Registry::migrations`](mush_core::Registry::migrations)) against the
/// store, following the same rules as the core schema
/// ([`mush_store::run_migrations`]):
///
/// - a **fresh namespace** (no schema doc, no documents) is stamped to the
///   latest registered version without running anything;
/// - an unstamped namespace **with** documents is pre-versioning data at
///   v0: every migration runs;
/// - a namespace **newer than the build** refuses to boot;
/// - before any migration actually runs, `bgsave` is triggered once (the
///   mandatory pre-migrate snapshot);
/// - each step transforms every document in the namespace (minus the
///   schema doc) and commits them **with** the version bump in one batch,
///   so a crash between steps resumes exactly where it stopped.
pub async fn run_module_migrations(
    store: &dyn ObjectStore,
    migrations: &HashMap<SmolStr, Vec<Migration>>,
) -> Result<(), String> {
    let mut namespaces: Vec<&SmolStr> = migrations.keys().collect();
    namespaces.sort(); // deterministic order across boots
    let mut snapshotted = false;
    for ns in namespaces {
        let migs = &migrations[ns];
        let Some(latest) = migs.last().map(|m| m.version) else {
            continue;
        };
        let mut prev = 0u32;
        for m in migs {
            if m.version <= prev {
                return Err(format!(
                    "module {ns}: migration set not strictly ascending at v{}",
                    m.version
                ));
            }
            prev = m.version;
        }
        let current = match ns_version(store, ns).await? {
            Some(v) => v,
            None => {
                let docs = store
                    .module_json_list(ns)
                    .await
                    .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
                if docs
                    .iter()
                    .all(|(key, _)| key == SCHEMA_KEY || is_action_state_persistence_key(key))
                {
                    // Fresh namespace: stamp to latest, run nothing.
                    write_ops(store, vec![stamp(ns, latest)]).await?;
                    tracing::info!(ns = %ns, version = latest, "fresh module namespace stamped");
                    continue;
                }
                0 // pre-versioning data
            }
        };
        if current > latest {
            return Err(format!(
                "module {ns}: stored schema v{current} is newer than this build's \
                 v{latest}; refusing to run (restore from backup to downgrade)"
            ));
        }
        if current == latest {
            continue;
        }
        if !snapshotted {
            store
                .bgsave()
                .await
                .map_err(|e| format!("pre-migrate BGSAVE failed: {e}"))?;
            snapshotted = true;
        }
        for m in migs.iter().filter(|m| m.version > current) {
            tracing::info!(ns = %ns, to = m.version, desc = %m.description,
                "running module migration");
            let docs = store
                .module_json_list(ns)
                .await
                .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
            let mut ops = Vec::new();
            for (key, value) in docs {
                if key == SCHEMA_KEY || is_action_state_persistence_key(&key) {
                    continue;
                }
                let migrated = (m.apply)(&key, value).map_err(|e| {
                    format!("module {ns} migration to v{}: doc {key:?}: {e}", m.version)
                })?;
                ops.push(ModuleOp {
                    ns: ns.clone(),
                    op: ModuleWrite::JsonSet {
                        key: SmolStr::new(key),
                        value: migrated,
                    },
                });
            }
            // The version bump commits with the transformed docs: a crash
            // between steps re-runs from exactly the stored version.
            ops.push(stamp(ns, m.version));
            write_ops(store, ops).await?;
        }
    }
    Ok(())
}

/// The full module boot sequence over an already-constructed engine (see
/// the module docs): migrations → boot data → `modules_loaded` → durable
/// write-through of whatever boot persisted. Call between
/// [`Engine::from_composition`] (or the compatibility
/// [`Engine::with_modules`] wrapper) and spawning the world task.
pub async fn boot_modules(engine: &mut Engine, store: &dyn ObjectStore) -> Result<(), String> {
    run_module_migrations(store, engine.registry.migrations()).await?;
    let (mut boot, action_state) =
        load_module_boot_with_action_state(store, engine.modules()).await?;
    engine
        .hydrate_action_state(action_state)
        .map_err(|error| format!("action-state boot failed: {error}"))?;
    load_requested_streams(store, engine.modules(), &mut boot).await?;
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    // J4 (doc 12 §5.6): the modules are delivered one at a time, in order, so
    // a module that cannot boot from a namespace narrowed by its own manifest
    // (the checkpoint it stood for proved invalid or gone) gets that namespace
    // fetched whole — manifest ignored — and delivered once more, itself
    // alone; the retry's failure is the boot's.
    let names: Vec<SmolStr> = engine
        .modules()
        .iter()
        .map(|m| SmolStr::new(m.name()))
        .collect();
    for name in names {
        let data = boot.remove(&name).unwrap_or_default();
        match engine.module_loaded(&name, data, &mut out, &mut fx) {
            Ok(()) => {}
            Err(ModError::RetryFullFetch { reason, .. }) => {
                tracing::warn!(
                    module = %name,
                    %reason,
                    "module cannot boot from its narrowed namespace; fetching it whole and delivering again"
                );
                // The manifest is loader-owned and its floors stood for a
                // checkpoint that did not hold: clear the floors, so the next
                // boot fetches the ledger whole until the module writes a new
                // manifest beside its next checkpoint. The key prefixes stay
                // (doc 13 W1): they name documents the module never loads at
                // boot.
                let kept = manifest_skip(store, &name).await?;
                let cleared = BootManifest {
                    format: mush_core::BOOT_MANIFEST_FORMAT,
                    skip_log_chunks_below: std::collections::BTreeMap::new(),
                    skip_key_prefixes: kept.prefixes.iter().map(SmolStr::new).collect(),
                };
                write_ops(
                    store,
                    vec![ModuleOp {
                        ns: name.clone(),
                        op: ModuleWrite::JsonSet {
                            key: SmolStr::new(BOOT_MANIFEST_KEY),
                            value: serde_json::to_value(&cleared)
                                .expect("a boot manifest serializes"),
                        },
                    }],
                )
                .await?;
                let (mut data, _already_hydrated_action_state) =
                    load_one_namespace(store, &name, Fetch::Whole).await?;
                let module = engine
                    .modules()
                    .iter()
                    .find(|m| m.name() == name)
                    .expect("the module was just delivered");
                load_streams_for(store, module.as_ref(), &mut data).await?;
                engine
                    .module_loaded(&name, data, &mut out, &mut fx)
                    .map_err(|e| format!("module boot failed after the whole fetch: {e}"))?;
            }
            Err(e) => return Err(format!("module boot failed: {e}")),
        }
    }
    // Nothing is connected yet; boot-time output has nowhere to go.
    let events = out.take();
    if !events.is_empty() {
        tracing::debug!(
            count = events.len(),
            "module boot output dropped (no sessions yet)"
        );
    }
    debug_assert!(fx.is_empty(), "modules must not defer effects at boot");
    // First-boot seeding / orphan cleanup must be durable before serving.
    let ops = engine.take_module_ops();
    if let Some((namespace, key)) = ops.iter().find_map(|op| {
        let key = match &op.op {
            ModuleWrite::JsonSet { key, .. }
            | ModuleWrite::Del { key }
            | ModuleWrite::StreamAppend { key, .. } => key,
        };
        is_action_state_persistence_key(key).then_some((op.ns.as_str(), key.as_str()))
    }) {
        return Err(format!(
            "module boot attempted coordinator-reserved persistence key {namespace}:{key}"
        ));
    }
    // S1: a module boot pass may also make OBJECTS — the content module applies the
    // starter packs, which dig rooms and place NPCs — and those ride the SAME batch
    // as the ops, so the content latch and the objects it describes commit together
    // or not at all ("objects and the latch or neither"). A crash between two writes
    // would otherwise leave a latch that says the lot exists over a world without
    // it, and the next boot would believe the latch.
    let dirty = engine.world.take_dirty();
    if ops.is_empty() && dirty.is_empty() {
        return Ok(());
    }
    let mut batch = crate::persist::batch_from_dirty(&engine.world, dirty, BOOT_SERIAL, true);
    batch.module_ops = ops;
    write_batch(store, batch).await
}

/// The serial every boot-time batch carries: the world seed's, the module boot
/// write's, and the manifest reset's. Receipts echo it; the world task's own serials
/// start after boot.
const BOOT_SERIAL: u64 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use mush_core::{
        AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, CanonicalHash,
        CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind,
        DefinitionRegistryBuilder, GOD, IdempotencyKey, IdempotencyScope, ModError, OwnerId,
        PreparedCausalRef, PrincipalId, Registrar, ReplayWrite, RequestUid, RulesValue,
        ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, World,
    };
    use mush_store::{AuthoritativeStore, InMemoryStore};

    /// A module that persists one doc under ns "toy" and registers a v1→v2
    /// migration renaming field `a` to `b`.
    struct ToyMod {
        loaded: Vec<(SmolStr, serde_json::Value)>,
    }

    impl GameModule for ToyMod {
        fn name(&self) -> &'static str {
            "toy"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.migrations(
                "toy",
                vec![
                    Migration {
                        version: 1,
                        description: "initial".into(),
                        apply: Arc::new(|_key, v| Ok(v)),
                    },
                    Migration {
                        version: 2,
                        description: "rename a to b on doc:* only".into(),
                        // The key parameter routes heterogeneous
                        // namespaces: only doc:* documents are reshaped.
                        apply: Arc::new(|key: &str, mut v: serde_json::Value| {
                            if key.starts_with("doc:")
                                && let Some(obj) = v.as_object_mut()
                                && let Some(a) = obj.remove("a")
                            {
                                obj.insert("b".into(), a);
                            }
                            Ok(v)
                        }),
                    },
                ],
            )?;
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            self.loaded = data.kv;
            Ok(())
        }
    }

    fn toy_engine() -> Engine {
        Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ToyMod { loaded: Vec::new() })],
        )
        .unwrap()
    }

    #[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct BootState {
        amount: u8,
    }

    struct StateBootMod {
        handle: Arc<Mutex<Option<StateFamilyHandle<u64, BootState>>>>,
        streams_requested: Arc<AtomicBool>,
        loaded: Arc<AtomicBool>,
    }

    impl GameModule for StateBootMod {
        fn name(&self) -> &'static str {
            "state_boot_test"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let descriptor = StateFamilyDescriptor::new(
                StateFamilyId::new("boot").expect("static family id"),
                CanonicalId::new("u64.decimal.v1").expect("static codec id"),
                CanonicalId::new("boot_state.v1").expect("static schema id"),
                1,
                CanonicalId::new("boot_state.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("boot_state.range.v1").expect("static validator id"),
                CanonicalId::new("boot_state.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?;
            let handle = reg.state_family(
                descriptor,
                |key: &u64| Ok(key.to_string()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                },
                |_value: &BootState| Ok(()),
                |value: &BootState| Ok(serde_json::json!({ "amount": value.amount })),
            )?;
            *self.handle.lock().expect("handle mutex poisoned") = Some(handle);
            Ok(())
        }

        fn boot_stream_requests(
            &self,
            _kv: &[(SmolStr, serde_json::Value)],
        ) -> Result<Vec<(SmolStr, u32)>, ModError> {
            self.streams_requested.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            if data
                .kv
                .iter()
                .any(|(key, _)| is_action_state_persistence_key(key))
            {
                return Err(ModError::msg("reserved state leaked into module boot"));
            }
            self.loaded.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn state_boot_engine() -> (
        Engine,
        StateFamilyHandle<u64, BootState>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        let captured = Arc::new(Mutex::new(None));
        let streams_requested = Arc::new(AtomicBool::new(false));
        let loaded = Arc::new(AtomicBool::new(false));
        let engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(StateBootMod {
                handle: Arc::clone(&captured),
                streams_requested: Arc::clone(&streams_requested),
                loaded: Arc::clone(&loaded),
            })],
        )
        .unwrap();
        let handle = captured
            .lock()
            .expect("handle mutex poisoned")
            .clone()
            .expect("registration captured state handle");
        (engine, handle, streams_requested, loaded)
    }

    fn boot_state_op(key: &str, amount: u8) -> ModuleOp {
        ModuleOp {
            ns: "state_boot_test".into(),
            op: ModuleWrite::JsonSet {
                key: format!("action-state/v1/boot/{key}").into(),
                value: serde_json::json!({
                    "family": { "owner": "state_boot_test", "id": "boot" },
                    "key": key,
                    "schema": { "id": "boot_state.v1", "version": 1 },
                    "value": { "amount": amount },
                }),
            },
        }
    }

    /// Seed a module document through the same authoritative prefix owner that
    /// production uses. Reserved `action-state/*` fixtures must never bypass
    /// the store guard through the legacy module-write API.
    async fn seed_authoritative_json(
        store: &InMemoryStore,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
    ) {
        let owner = OwnerId::new(namespace).unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("boot_fixture").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("boot_fixture_written").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([0xA1; 16]);
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([0xA2; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: PrincipalId::Object { id: GOD },
                    owner: owner.clone(),
                    namespace: CanonicalId::new("boot_fixture").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([0xA3; 32]),
            request_hash: CanonicalHash::from_bytes([0xA4; 32]),
            plan_hash: CanonicalHash::from_bytes([0xA5; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome.clone()],
            writes: vec![ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: AuthoritativeKey::parse_canonical(key).unwrap(),
                value,
            }],
            outcome: RulesValue::new(outcome.clone(), serde_json::json!({"seeded": true})).unwrap(),
            resolution: None,
            semantic_events: vec![
                mush_core::AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &action,
                    RulesValue::new(outcome, serde_json::json!({"seeded": true})).unwrap(),
                    vec![PrincipalId::Object { id: GOD }],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![PrincipalId::Object { id: GOD }],
            durable_outbox: vec![],
        })
        .unwrap();
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store.reserve_authoritative_intent(bundle).await.unwrap();
        store.reconcile_authoritative(token, hash).await.unwrap();
        store.checkpoint_authoritative(token, hash).await.unwrap();
    }

    async fn seed_authoritative_op(store: &InMemoryStore, op: ModuleOp) {
        let ModuleWrite::JsonSet { key, value } = op.op else {
            panic!("authoritative boot fixture must be a JSON set");
        };
        seed_authoritative_json(store, &op.ns, &key, value).await;
    }

    struct ReservedBootWrite;

    impl GameModule for ReservedBootWrite {
        fn name(&self) -> &'static str {
            "reserved_boot_write"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), ModError> {
            ctx.persist(ModuleOp {
                ns: self.name().into(),
                op: ModuleWrite::JsonSet {
                    key: "action-state/v1/forged/1".into(),
                    value: serde_json::json!(true),
                },
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn module_boot_cannot_write_the_reserved_action_state_keyspace() {
        let store = InMemoryStore::new();
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ReservedBootWrite)],
        )
        .unwrap();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("coordinator-reserved"), "{error}");
        assert!(
            store
                .module_json_get("reserved_boot_write", "action-state/v1/forged/1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn boot_hydrates_typed_state_before_module_callbacks() {
        let store = InMemoryStore::new();
        seed_authoritative_op(&store, boot_state_op("7", 3)).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        boot_modules(&mut engine, &store).await.unwrap();

        assert!(streams_requested.load(Ordering::SeqCst));
        assert!(loaded.load(Ordering::SeqCst));
        assert_eq!(
            engine.action_state().get(&handle, &7).unwrap(),
            Some(&BootState { amount: 3 })
        );
    }

    #[tokio::test]
    async fn malformed_typed_state_aborts_before_any_module_callback() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { value, .. } = &mut op.op else {
            unreachable!()
        };
        value["schema"]["version"] = serde_json::json!(2);
        seed_authoritative_op(&store, op).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("schema.version"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
    }

    #[tokio::test]
    async fn future_action_state_encoding_is_reserved_and_fails_closed() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { key, .. } = &mut op.op else {
            unreachable!()
        };
        *key = "action-state/v2/boot/7".into();
        seed_authoritative_op(&store, op).await;
        let (mut engine, _, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("unsupported action-state"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn fresh_namespace_is_stamped_without_running() {
        let store = InMemoryStore::new();
        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();
        let v = ns_version(&store, "toy").await.unwrap();
        assert_eq!(v, Some(2));
    }

    #[tokio::test]
    async fn preversioned_docs_migrate_and_stamp() {
        let store = InMemoryStore::new();
        // Seed a pre-versioning doc (no schema key).
        write_ops(
            &store,
            vec![ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "doc:1".into(),
                    value: serde_json::json!({ "a": 42 }),
                },
            }],
        )
        .await
        .unwrap();

        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();

        let doc = store
            .module_json_get("toy", "doc:1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc, serde_json::json!({ "b": 42 }));
        assert_eq!(ns_version(&store, "toy").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn module_migrations_never_transform_reserved_action_state() {
        let store = InMemoryStore::new();
        let reserved = serde_json::json!({ "a": "must remain byte-equivalent" });
        seed_authoritative_json(&store, "toy", "action-state/v1/example/1", reserved.clone()).await;
        write_ops(
            &store,
            vec![ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "doc:1".into(),
                    value: serde_json::json!({ "a": 42 }),
                },
            }],
        )
        .await
        .unwrap();
        let engine = toy_engine();

        run_module_migrations(&store, engine.registry.migrations())
            .await
            .unwrap();

        assert_eq!(
            store
                .module_json_get("toy", "action-state/v1/example/1")
                .await
                .unwrap(),
            Some(reserved)
        );
        assert_eq!(
            store.module_json_get("toy", "doc:1").await.unwrap(),
            Some(serde_json::json!({ "b": 42 }))
        );
    }

    #[tokio::test]
    async fn newer_store_refuses_to_boot() {
        let store = InMemoryStore::new();
        write_ops(&store, vec![stamp("toy", 9)]).await.unwrap();
        let mut engine = toy_engine();
        let err = boot_modules(&mut engine, &store).await.unwrap_err();
        assert!(err.contains("newer than this build"), "{err}");
    }

    #[tokio::test]
    async fn schema_key_never_reaches_module_boot_data() {
        let store = InMemoryStore::new();
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];
        let boot = load_module_boot(&store, &modules).await.unwrap();
        let kv = &boot[&SmolStr::new("toy")].kv;
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "doc:1");
    }

    /// J4 (doc 12 §5.6): the loader reads the namespace's boot manifest first
    /// and fetches nothing it names; the omission reaches the module as the
    /// min-present marker; the manifest itself never does; a whole fetch
    /// ignores it.
    #[tokio::test]
    async fn j4_the_loader_narrows_the_fetch_to_the_boot_manifest() {
        let store = InMemoryStore::new();
        let chunk = |n: u64, sealed: bool| {
            serde_json::json!({
                "chunk": n, "sealed": sealed, "entries": [{ "seq": n, "value": { "n": n } }]
            })
        };
        let set = |key: &str, value: serde_json::Value| ModuleOp {
            ns: "toy".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
        };
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                set("doc:1", serde_json::json!({ "b": 1 })),
                set("log:s:0", chunk(0, true)),
                set("log:s:1", chunk(1, true)),
                set("log:s:2", chunk(2, false)),
                set("log:t:0", chunk(0, false)),
                set("idem:aa", serde_json::json!({ "tx": 1 })),
                set(
                    BOOT_MANIFEST_KEY,
                    serde_json::to_value(BootManifest {
                        format: mush_core::BOOT_MANIFEST_FORMAT,
                        skip_log_chunks_below: [(SmolStr::new("s"), 2u64)].into_iter().collect(),
                        skip_key_prefixes: vec![SmolStr::new("idem:")],
                    })
                    .unwrap(),
                ),
            ],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(
            data.kv.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["doc:1"],
            "the manifest is loader-owned and never a module document; the prefixed key is not fetched"
        );
        let mut ledger: Vec<&str> = data.ledger.iter().map(|(k, _)| k.as_str()).collect();
        ledger.sort_unstable();
        assert_eq!(ledger, vec!["log:s:2", "log:t:0"]);
        assert_eq!(
            data.omitted,
            [(SmolStr::new("s"), 2u64)].into_iter().collect()
        );
        let (whole, _) = load_one_namespace(&store, "toy", Fetch::Whole)
            .await
            .unwrap();
        assert_eq!(
            whole.ledger.len(),
            4,
            "a whole fetch ignores the manifest's floors"
        );
        assert!(whole.omitted.is_empty());
        assert_eq!(
            whole.kv.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["doc:1"],
            "a whole fetch still honours the manifest's key prefixes"
        );
        // An unreadable manifest is a whole fetch, not a failed boot.
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::json!({ "format": "nine" }),
            )],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(data.ledger.len(), 4);
        assert!(data.omitted.is_empty());
        // A manifest at a format this loader does not know: the same.
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::json!({ "format": 99, "skip_log_chunks_below": { "s": 2 } }),
            )],
        )
        .await
        .unwrap();
        let (data, _) = load_one_namespace(&store, "toy", Fetch::Narrowed)
            .await
            .unwrap();
        assert_eq!(data.ledger.len(), 4);
    }

    /// A module that refuses a narrowed namespace once (J4's
    /// `RetryFullFetch`) and records what each delivery carried.
    struct PickyMod {
        deliveries: Arc<Mutex<Vec<(usize, usize)>>>,
        refuse_narrowed: bool,
    }

    impl GameModule for PickyMod {
        fn name(&self) -> &'static str {
            "picky"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            self.deliveries
                .lock()
                .unwrap()
                .push((data.ledger.len(), data.omitted.len()));
            if self.refuse_narrowed && !data.omitted.is_empty() {
                return Err(ModError::RetryFullFetch {
                    module: SmolStr::new("picky"),
                    reason: "the checkpoint the manifest stood for is gone".into(),
                });
            }
            Ok(())
        }
    }

    /// J4 (lens G): a module that cannot boot from its narrowed namespace is
    /// delivered the whole one, itself alone, and the boot goes on; a module
    /// that refuses the whole one too fails the boot; the retry is not
    /// offered to a whole fetch.
    #[tokio::test]
    async fn j4_boot_modules_refetches_whole_for_a_module_that_asks() {
        let store = InMemoryStore::new();
        let chunk = |n: u64, sealed: bool| {
            serde_json::json!({
                "chunk": n, "sealed": sealed, "entries": [{ "seq": n, "value": { "n": n } }]
            })
        };
        let set = |key: &str, value: serde_json::Value| ModuleOp {
            ns: "picky".into(),
            op: ModuleWrite::JsonSet {
                key: key.into(),
                value,
            },
        };
        write_ops(
            &store,
            vec![
                set("log:s:0", chunk(0, true)),
                set("log:s:1", chunk(1, false)),
                set(
                    BOOT_MANIFEST_KEY,
                    serde_json::to_value(BootManifest {
                        format: mush_core::BOOT_MANIFEST_FORMAT,
                        skip_log_chunks_below: [(SmolStr::new("s"), 1u64)].into_iter().collect(),
                        skip_key_prefixes: vec![SmolStr::new("idem:")],
                    })
                    .unwrap(),
                ),
            ],
        )
        .await
        .unwrap();
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let picky = PickyMod {
            deliveries: Arc::clone(&deliveries),
            refuse_narrowed: true,
        };
        let modules: Vec<Box<dyn GameModule>> =
            vec![Box::new(picky), Box::new(ToyMod { loaded: Vec::new() })];
        let mut engine =
            Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
                .unwrap();
        boot_modules(&mut engine, &store).await.unwrap();
        assert_eq!(
            *deliveries.lock().unwrap(),
            vec![(1, 1), (2, 0)],
            "narrowed first (one chunk, one floor), then whole (two chunks, no floor)"
        );
        assert_eq!(
            store
                .module_json_get("picky", BOOT_MANIFEST_KEY)
                .await
                .unwrap()
                .map(|doc| serde_json::from_value::<BootManifest>(doc).unwrap()),
            Some(BootManifest {
                format: mush_core::BOOT_MANIFEST_FORMAT,
                skip_log_chunks_below: std::collections::BTreeMap::new(),
                skip_key_prefixes: vec![SmolStr::new("idem:")],
            }),
            "the floors that stood for nothing are cleared by the loader; the key prefixes are kept"
        );

        // The whole fetch refused too: the boot fails with the module's reason.
        // (The first retry cleared the manifest; write it back for this one.)
        write_ops(
            &store,
            vec![set(
                BOOT_MANIFEST_KEY,
                serde_json::to_value(BootManifest {
                    format: mush_core::BOOT_MANIFEST_FORMAT,
                    skip_log_chunks_below: [(SmolStr::new("s"), 1u64)].into_iter().collect(),
                    skip_key_prefixes: Vec::new(),
                })
                .unwrap(),
            )],
        )
        .await
        .unwrap();
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        struct AlwaysRetry(Arc<Mutex<Vec<(usize, usize)>>>);
        impl GameModule for AlwaysRetry {
            fn name(&self) -> &'static str {
                "picky"
            }
            fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
                Ok(())
            }
            fn on_world_loaded(
                &mut self,
                _ctx: &mut mush_core::ModuleCtx<'_>,
                data: ModuleBootData,
            ) -> Result<(), ModError> {
                self.0
                    .lock()
                    .unwrap()
                    .push((data.ledger.len(), data.omitted.len()));
                Err(ModError::RetryFullFetch {
                    module: SmolStr::new("picky"),
                    reason: "never satisfied".into(),
                })
            }
        }
        let modules: Vec<Box<dyn GameModule>> =
            vec![Box::new(AlwaysRetry(Arc::clone(&deliveries)))];
        let mut engine =
            Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
                .unwrap();
        let err = boot_modules(&mut engine, &store).await.unwrap_err();
        assert!(err.contains("after the whole fetch"), "{err}");
        assert!(err.contains("never satisfied"), "{err}");
        assert_eq!(
            *deliveries.lock().unwrap(),
            vec![(1, 1), (2, 0)],
            "exactly one retry"
        );
    }

    /// J1: ledger chunks reach boot data as text, apart from the ordinary
    /// documents, and the namespace fetch is timed.
    #[tokio::test]
    async fn j1_ledger_chunks_reach_boot_data_as_text() {
        let store = InMemoryStore::new();
        let chunk = serde_json::json!({
            "chunk": 0, "sealed": false, "entries": [{ "seq": 0, "value": { "n": 1 } }]
        });
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "log:s:0".into(),
                        value: chunk.clone(),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];
        let boot = load_module_boot(&store, &modules).await.unwrap();
        let data = &boot[&SmolStr::new("toy")];
        assert_eq!(data.kv.len(), 1);
        assert_eq!(data.kv[0].0, "doc:1");
        assert_eq!(data.ledger.len(), 1);
        assert_eq!(data.ledger[0].0, "log:s:0");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&data.ledger[0].1).unwrap(),
            chunk
        );
        assert!(data.fetch_ms < 60_000, "a timed fetch: {}", data.fetch_ms);
    }

    #[tokio::test]
    async fn action_state_keys_never_reach_module_boot_data() {
        let store = InMemoryStore::new();
        seed_authoritative_json(
            &store,
            "toy",
            "action-state/v99/future/1",
            serde_json::json!({ "opaque": true }),
        )
        .await;
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];

        let boot = load_module_boot(&store, &modules).await.unwrap();

        assert_eq!(boot[&SmolStr::new("toy")].kv.len(), 1);
        assert_eq!(boot[&SmolStr::new("toy")].kv[0].0, "doc:1");
    }

    #[tokio::test]
    async fn server_modules_boot_and_seed_durably() {
        // The real module list boots against an empty store; comsys'
        // first-boot seeding (the Public channel) must be written through.
        let store = InMemoryStore::new();
        let mut registry = crate::world_task::base_registry();
        let staff = std::sync::Arc::new(crate::staff::StaffQueue::new());
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine, &store).await.unwrap();

        let chans = store.module_json_list("comsys").await.unwrap();
        assert!(
            chans.iter().any(|(k, v)| k.starts_with("chan:")
                && v.get("name").and_then(serde_json::Value::as_str) == Some("Public")),
            "first boot must seed the Public channel durably: {chans:?}"
        );

        // A second boot over the same store must NOT re-seed (the member
        // docs and channel survive; God's membership from boot one holds).
        let mut registry = crate::world_task::base_registry();
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine2 =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine2, &store).await.unwrap();
        let chans2 = store.module_json_list("comsys").await.unwrap();
        let count = |cs: &[(String, serde_json::Value)]| {
            cs.iter().filter(|(k, _)| k.starts_with("chan:")).count()
        };
        assert_eq!(count(&chans), count(&chans2), "reboot must not re-seed");
    }

    #[test]
    fn m5_modules_opt_into_the_testkit_cleanly() {
        // The testkit's `with_modules` is the module-authoring on-ramp: the
        // M5 scripting + weather modules must register (commands, hooks,
        // state slots), first-boot, and surface their seams there — the
        // "opt into the modules" confirmation for both.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(vec![
            Box::new(mush_weather::WeatherModule::new()),
            Box::new(mush_script::ScriptModule::new()),
        ]);
        // Weather filled its reserved query seam...
        let (owner, _) = tw.registry().weather_query().expect("weather query seam");
        assert_eq!(owner.as_str(), "weather");
        // ...and script installed its post-command-table `$`-command
        // fallback (the object-command matcher).
        assert!(
            tw.registry().command_fallback_count() >= 1,
            "script registers a command fallback"
        );
        // Both modules are present under the one registry, no collision.
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(
            names.contains(&"weather") && names.contains(&"script"),
            "{names:?}"
        );
    }

    #[test]
    fn k8_5_production_compositions_publish_command_compatibility() {
        for ruleset in ["wod", "shadowrun"] {
            let onboarding = ruleset_wod::OnboardingConfig::default();
            let wod = ruleset_wod::WodEconomyConfig::default();
            let shadowrun = ruleset_shadowrun::SrEconomyConfig::default();
            let mut modules = server_modules_with(
                mush_weather::WeatherConfig::default(),
                mush_economy::EconomyConfig::default(),
                mush_atlas::AtlasConfig::default(),
                &onboarding,
                (ruleset == "wod").then_some(&wod),
                (ruleset == "shadowrun").then_some(&shadowrun),
                mush_campaign::RecoveryConfig::default(),
            );
            modules.extend(
                ruleset_modules(&[ruleset.to_owned()], &onboarding, &wod, &shadowrun)
                    .expect("one production ruleset"),
            );
            // E7 cut 1: the TacSim layer is part of the production composition, so its
            // `reg.classify_commands(..)` declaration is enforced here too.
            modules.push(tacsim_module(ruleset_tacsim::fixture::config()));
            let composition = Engine::compose_modules(crate::world_task::base_registry(), modules)
                .expect("production composition declares every command lane");

            let registrations = composition
                .registry()
                .command_registrations()
                .collect::<Vec<_>>();
            assert!(
                registrations.len() > 50,
                "unexpectedly small {ruleset} command surface"
            );
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@tacsim"
                        && *owner == Some("tacsim")
                        && *class == mush_core::CommandCompatibility::LegacyPostImage
                }),
                "the TacSim layer must publish its compatibility lane"
            );
            // E7 residue pass 1, R2: `@corridor` is a SECOND registration group in the
            // same module, declared `ReadOnly` before its own `reg.command` and with
            // `LegacyPostImage` restored immediately after — the K8.5 discipline that
            // failed `mush-market`'s 140 tests at the K merge, applied per group.
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@corridor"
                        && *owner == Some("tacsim")
                        && *class == mush_core::CommandCompatibility::ReadOnly
                }),
                "the corridor read surface must publish `ReadOnly`"
            );
            // E7 cut 2: the FIRST `Authoritative` command registration on the
            // tree. `@inv` is one `CommandSpec`, so it is one registration and
            // takes the ceiling class of its nine switches.
            assert!(
                registrations.iter().any(|(spec, owner, class)| {
                    spec.name == "@inv"
                        && *owner == Some("inventory")
                        && *class == mush_core::CommandCompatibility::Authoritative
                }),
                "the inventory lane must publish `Authoritative`"
            );
            // E7 cut 3: five lanes in THREE registration groups, each with its
            // own `reg.classify_commands(..)` before its first `reg.command`.
            // `@deposit` is `Authoritative` rather than `ReadOnly` because a
            // body's reserve IS cut 2's pool and a pool can only be credited
            // inside an authoritative bundle — conservative over-declaration,
            // and the two read switches simply never submit.
            for (name, class) in [
                ("@deposit", mush_core::CommandCompatibility::Authoritative),
                ("@claim", mush_core::CommandCompatibility::Authoritative),
                ("@extract", mush_core::CommandCompatibility::Authoritative),
                ("@survey", mush_core::CommandCompatibility::LegacyPostImage),
                ("@licence", mush_core::CommandCompatibility::ReadOnly),
                // E7 cut 4: the work-order lane (ceiling Authoritative) and the
                // facility lane (LegacyPostImage — open writes a document).
                ("@wo", mush_core::CommandCompatibility::Authoritative),
                (
                    "@facility",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("industry") && *published == class
                    }),
                    "the industry lane {name} must publish {class:?}"
                );
            }
            // E7 cut 5: `@station` is one CommandSpec, so it takes the CEILING
            // class of its switches — `/handoff` and `/cancel` ride cut 2's
            // authoritative control-lease ingress, so the sim lane publishes
            // `Authoritative` (`/show` reads and `/fallback` writes the exempt
            // `sim.station_fallback` record under the same lane).
            //
            // E7 cut 6 (fold B10): `@journey` is the sim lane's other E7 verb
            // and had no row here — it is covered by the allowlist tripwire and
            // by `e7c6_p4`, but `e7c6_p4` builds a two-module `TestWorld`, so
            // nothing caught a composition-order change that re-classified it.
            // A ROW in this loop, not a new stanza: `PINNED_BASELINE` is
            // untouched.
            //
            // E7 cut 8 (B6): the eight tactical verbs join `@station`'s
            // authoritative group — every one stages `tactical_intent` through
            // cut 2's seam, which is the ceiling criterion cut 5 set and cut 7
            // reused for `@prop`.
            //
            // E7 cut 8 independent verify fold (B1): `@domain` publishes
            // `Authoritative`. Its cut-6 note here — "the durable cursor is
            // armed by the `sim.domain_pulse` Coarse tick, never by the
            // command" — was factually false and is deleted: `@domain/tick`
            // runs `tick::run_maintenance`, which reaches
            // `Ctx::submit_authoritative(domain_pulse_arm, ..)` from the
            // command handler, and cut 8's own `e7c8_x3_*` proved it by
            // draining that submission. Its allowlist stanza moves
            // `baseline` → `authoritative` in the same commit, which lowers
            // `PINNED_BASELINE` 158 → 157 — the retirement the tripwire itself
            // documents as legal.
            for (name, class) in [
                ("@station", mush_core::CommandCompatibility::Authoritative),
                ("@journey", mush_core::CommandCompatibility::LegacyPostImage),
                ("@domain", mush_core::CommandCompatibility::Authoritative),
                ("+fire", mush_core::CommandCompatibility::Authoritative),
                ("+launch", mush_core::CommandCompatibility::Authoritative),
                ("+heading", mush_core::CommandCompatibility::Authoritative),
                ("+thrust", mush_core::CommandCompatibility::Authoritative),
                ("+posture", mush_core::CommandCompatibility::Authoritative),
                ("+dc", mush_core::CommandCompatibility::Authoritative),
                ("+dock", mush_core::CommandCompatibility::Authoritative),
                ("+eject", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("sim") && *published == class
                    }),
                    "the sim lane {name} must publish {class:?}"
                );
            }
            // E7 cut 7: FIVE verbs in THREE registration groups, each with its
            // own `reg.classify_commands(..)` before its first `reg.command`.
            // `+contacts`/`+contact` are pure reads over the leak-safe ledger;
            // `+order` writes propagation's own exempt documents, and `+orders`
            // takes the CEILING class of its switches because `/ack` writes the
            // acknowledgement envelope. `@prop` takes the ceiling of ITS
            // switches too, and `/pass` stages the tactics-owned `sensor_arm`
            // through `Ctx::submit_authoritative` — an ingress adapter for the
            // authoritative action coordinator, which is `Authoritative`'s own
            // definition and exactly the criterion cut 5 used for `@station`
            // (E7 cut 7 independent verify fold, B8).
            for (name, class) in [
                ("+contacts", mush_core::CommandCompatibility::ReadOnly),
                ("+contact", mush_core::CommandCompatibility::ReadOnly),
                ("+order", mush_core::CommandCompatibility::LegacyPostImage),
                ("+orders", mush_core::CommandCompatibility::LegacyPostImage),
                ("@prop", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("propagation") && *published == class
                    }),
                    "the propagation lane {name} must publish {class:?}"
                );
            }
            // E7 cut 9: THREE verbs in TWO registration groups, each with its own
            // `reg.classify_commands(..)` before its first `reg.command`, and
            // `LegacyPostImage` restored last so the module ends in the class its
            // own documents ride. A `CommandSpec` is one PRIMARY VERB, so each
            // takes the CEILING class of its switches — the rule the tree already
            // states for `@station`, `@journey`, `@prop` and `@domain`.
            // `@formation` reads only. `@op` and `@force` write the exempt
            // `campaign.operation` / `campaign.formation` documents, and
            // `@op/activate` additionally posts an E4 `HoldOp::Place` through
            // `mush_economy::post` — the economy's OWN carve-out lane, never
            // `Ctx::submit_authoritative` — so the ceiling is `LegacyPostImage`
            // and NOT `Authoritative`.
            for (name, class) in [
                ("@formation", mush_core::CommandCompatibility::ReadOnly),
                ("@op", mush_core::CommandCompatibility::LegacyPostImage),
                ("@force", mush_core::CommandCompatibility::LegacyPostImage),
                // E7 residue pass 1, R3: `@recovery` is its own registration group,
                // classified before its own `reg.command` and restored after. Its
                // ceiling is `LegacyPostImage` for `@op`'s exact reason — the three
                // `recovery.queue` documents are exempt workflow records and the
                // outfitting stipend rides `mush_economy::post`'s carve-out lane
                // from the command, never `Ctx::submit_authoritative`.
                (
                    "@recovery",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("campaign") && *published == class
                    }),
                    "the campaign lane {name} must publish {class:?}"
                );
            }
            // ---- E7r2: P1 — the market lane moves to `Authoritative` ----
            //
            // Both verbs, by the CEILING rule a `CommandSpec` takes over its switches
            // (`@station` / `@inv` / `@wo` / `@prop` / `@domain`'s precedent):
            // `@market/vendor stock`, `@market/vendor withdraw` and `+market/accept`
            // each stage exactly one `Ctx::submit_authoritative` of the market-owned
            // `market_lot` action, which reserves, settles and releases on cut 2's
            // delegated `InventoryFamilies`. `crate::commands::register` classifies the
            // group before its first `reg.command` and restores `LegacyPostImage`
            // after, so the module still ends in the class its own `market:*` documents
            // ride. Both allowlist categories move in the same commit and
            // `PINNED_BASELINE` is unmoved at 157 — neither stanza was baseline.
            for (name, class) in [
                ("+market", mush_core::CommandCompatibility::Authoritative),
                ("@market", mush_core::CommandCompatibility::Authoritative),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("market") && *published == class
                    }),
                    "the market lane {name} must publish {class:?}"
                );
            }
            // E7 cut 10: the seat lane, in TWO further groups on the same
            // module. `+seat` and `@seat` publish `Authoritative` because
            // `/activate`, `/leave` and `/revoke` reach K6's control lease
            // through cut 5's ingress (`@station`'s exact precedent — the
            // ingress publishes the class, whoever stages it); `@crew`,
            // `+crewcall` and `+muster` write only the exempt `campaign.crew` /
            // `campaign.muster` workflow documents, and `LegacyPostImage` is
            // restored last so the module still ends in the class its own
            // records ride.
            for (name, class) in [
                ("+seat", mush_core::CommandCompatibility::Authoritative),
                ("@seat", mush_core::CommandCompatibility::Authoritative),
                (
                    "+crewcall",
                    mush_core::CommandCompatibility::LegacyPostImage,
                ),
                ("+muster", mush_core::CommandCompatibility::LegacyPostImage),
                ("@crew", mush_core::CommandCompatibility::LegacyPostImage),
            ] {
                assert!(
                    registrations.iter().any(|(spec, owner, published)| {
                        spec.name == name && *owner == Some("campaign") && *published == class
                    }),
                    "the campaign seat lane {name} must publish {class:?}"
                );
            }
            // ---- E7r2: P0c — `+nuyen` is `ReadOnly`, in its own group ----
            //
            // E7 residue pass 1's fold recorded `+nuyen` in the class tripwire's
            // `KNOWN_CLASS_DRIFT`: the allowlist declared `read-only` while
            // `ShadowrunRuleset::register`'s module-wide
            // `classify_commands(LegacyPostImage)` published a post-image writer.
            // `nuyen_cmd` writes nothing, so pass 2 fixed the CLASS: its own
            // registration group, `ReadOnly` before its `reg.command`, with
            // `LegacyPostImage` restored after. `KNOWN_CLASS_DRIFT` is now empty and
            // this row is what keeps the group from being folded back in.
            if ruleset == "shadowrun" {
                assert!(
                    registrations.iter().any(|(spec, owner, class)| {
                        spec.name == "+nuyen"
                            && *owner == Some("shadowrun")
                            && *class == mush_core::CommandCompatibility::ReadOnly
                    }),
                    "the shadowrun nuyen read must publish `ReadOnly`"
                );
                // …and the group is restored, so the ruleset's own documents keep
                // riding the class they always did.
                assert!(
                    registrations.iter().any(|(spec, owner, class)| {
                        spec.name == "@sr"
                            && *owner == Some("shadowrun")
                            && *class == mush_core::CommandCompatibility::LegacyPostImage
                    }),
                    "the shadowrun lane restores `LegacyPostImage` after the read group"
                );
            }
            assert!(registrations.iter().any(|(_, owner, class)| {
                *owner == Some("economy")
                    && *class == mush_core::CommandCompatibility::EconomyCarveOut
            }));
            assert!(registrations.iter().all(|(_, owner, class)| {
                *owner != Some("economy")
                    || *class == mush_core::CommandCompatibility::EconomyCarveOut
            }));
            assert!(
                composition
                    .registry()
                    .command_fallback_compatibilities()
                    .any(|(owner, class)| {
                        owner == Some("script")
                            && class == mush_core::CommandCompatibility::LegacyPostImage
                    })
            );
        }
    }

    /// E7 cut 3: the claim's titled object is minted by
    /// `mush_economy::issue_title` under `industry.claims`, which `issue_title`
    /// refuses unless it is on the economy's own issuer list. The two names
    /// have to agree, so a typo is a loud refusal at the verb rather than a
    /// silent capability — this pins that they do.
    #[test]
    fn e7c3_wiring_the_industry_title_issuer_is_registered_with_the_economy() {
        use mush_testkit::TestWorld;
        let onboarding = ruleset_wod::OnboardingConfig::default();
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            None,
            None,
            mush_campaign::RecoveryConfig::default(),
        ));
        // The economy composed at all, and the industry store came with it —
        // which is the composition this test is really about.
        assert!(
            tw.world()
                .module_state::<mush_industry::IndustryStore>()
                .is_some(),
            "mush-industry composes on every ruleset"
        );
        assert_eq!(
            mush_industry::industry_title_issuer().as_str(),
            "industry.claims"
        );
        // And the composition root passed exactly that principal to
        // `with_title_issuers`: a `@claim/file` that reached `issue_title` with
        // an unregistered issuer would answer `Unauthorized`, so the pin is the
        // source line itself.
        let source = include_str!("modules.rs");
        assert!(
            source.contains("issuers.push(mush_industry::industry_title_issuer());"),
            "the industry issuer must be on the economy's list"
        );
    }

    /// E7 cut 3 B1: the cross-owner write bridge. `mush-inventory` delegates
    /// its typed families to `"industry"` at composition, which is what lets one
    /// extraction bundle carry both owners' post-images. It is never implicit:
    /// no other owner is a grantee, and the delegation is not symmetric.
    ///
    /// **E7r2 P1 adds the second grantee and nothing else.** `"market"` joins on
    /// exactly the same terms — a market-owned action's prepare drives the same
    /// exported verbs under `mush_market::listing_holder` — and the two properties this
    /// test exists for are unchanged: the delegation is still **one-directional** (a
    /// delegate cannot write back), and the whole production composition still carries
    /// **only** the delegations named here, so a third one cannot arrive unnoticed.
    #[test]
    fn e7c3_wiring_the_quantity_delegation_is_declared_and_narrow() {
        use mush_testkit::TestWorld;
        let onboarding = ruleset_wod::OnboardingConfig::default();
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            None,
            None,
            mush_campaign::RecoveryConfig::default(),
        ));
        let families = tw
            .engine()
            .registry
            .state_families()
            .expect("the composed state-family registry");
        let inventory = mush_core::OwnerId::new("inventory").expect("owner");
        let industry = mush_core::OwnerId::new("industry").expect("owner");
        let market = mush_core::OwnerId::new("market").expect("owner");
        assert!(
            families.delegates(&inventory, &industry),
            "inventory delegates its families to industry"
        );
        assert!(
            families.delegates(&inventory, &market),
            "E7r2 P1: …and to market, which is what lets one sale carry the pool, the \
             reservation and the buyer's lot in one bundle"
        );
        assert!(
            !families.delegates(&industry, &inventory),
            "and the delegation is NOT symmetric"
        );
        assert!(
            !families.delegates(&market, &inventory),
            "…in either direction: the market grants the quantity authority nothing"
        );
        assert!(
            !families.delegates(&market, &industry) && !families.delegates(&industry, &market),
            "and the two grantees are strangers to each other"
        );
        assert_eq!(
            families.delegations(),
            vec![(inventory.clone(), industry), (inventory, market)],
            "exactly two delegations in the whole production composition, both from \
             `inventory` outward"
        );
    }

    /// E5c-C231 (look-back fold): the `[wod.economy].reserved_rooms` →
    /// `server_modules_with` → `HousingModule::with_reserved_rooms` chain, pinned
    /// before Shadowrun copies the plumbing — housing's store is told every room the
    /// config names, plus `[onboarding]`'s two start rooms and Limbo.
    #[test]
    fn lb_e5cc231_the_wod_reserved_rooms_reach_housing_through_server_modules_with() {
        use mush_testkit::TestWorld;
        let onboarding = ruleset_wod::OnboardingConfig::default();
        let wod = ruleset_wod::WodEconomyConfig {
            reserved_rooms: vec![mush_core::ObjId(4_242), mush_core::ObjId(4_243)],
            ..ruleset_wod::WodEconomyConfig::default()
        };
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            Some(&wod),
            None,
            mush_campaign::RecoveryConfig::default(),
        ));
        let store = tw
            .world()
            .module_state::<mush_housing::HousingStore>()
            .expect("housing is wired");
        for room in [
            mush_core::ObjId(4_242),
            mush_core::ObjId(4_243),
            onboarding.start_room,
            onboarding.start_room_unapproved,
            mush_core::LIMBO,
        ] {
            assert!(
                store.is_reserved(room),
                "#{} did not reach housing's reserved list",
                room.0
            );
        }
        assert!(
            !store.is_reserved(mush_core::ObjId(9_999)),
            "an unnamed room was reserved"
        );
        assert!(
            store.issuer().is_some(),
            "the title issuer is wired with housing on"
        );
        // A server without the WoD ruleset reserves the host's rooms alone and wires
        // no issuer.
        let tw = TestWorld::with_modules(server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            None,
            None,
            mush_campaign::RecoveryConfig::default(),
        ));
        let store = tw
            .world()
            .module_state::<mush_housing::HousingStore>()
            .expect("housing is wired");
        assert!(store.is_reserved(mush_core::LIMBO) && store.is_reserved(onboarding.start_room));
        assert!(!store.is_reserved(mush_core::ObjId(4_242)));
        assert!(store.issuer().is_none());
    }

    /// SP1 (D-LBK-7): a cadence declared in `[economy.cadences]` reaches the
    /// composition-frozen clock registry through `economy_config` →
    /// `server_modules_with` → `EconomyModule::register`, is created at boot, and
    /// is read through `Engine::clock_now` as `Registered { economy, cadence.<name> }`.
    /// The WoD layer, booting after the economy, finds the declared campaign clock
    /// by name instead of minting its own manual one.
    #[test]
    fn sp1_a_declared_cadence_reaches_the_registry_through_server_modules_with() {
        use mush_testkit::TestWorld;
        let cfg: mush_config::config::Config = toml::from_str(
            "[economy.cadences.\"chronicle.period\"]\nclock = \"real_utc\"\n\
             period_secs = 2592000\n",
        )
        .expect("parses");
        let economy = crate::config_ext::economy_config(&cfg.economy).expect("converts");
        let onboarding = ruleset_wod::OnboardingConfig::default();
        let wod = ruleset_wod::WodEconomyConfig::default();
        assert_eq!(wod.cadence, "chronicle.period");
        let mut mods = server_modules_with(
            mush_weather::WeatherConfig::default(),
            economy,
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            Some(&wod),
            None,
            mush_campaign::RecoveryConfig::default(),
        );
        mods.extend(
            ruleset_modules(
                &["wod".to_string()],
                &onboarding,
                &wod,
                &ruleset_shadowrun::SrEconomyConfig::default(),
            )
            .expect("wod"),
        );
        let tw = TestWorld::with_modules(mods);
        let clock = mush_core::ClockRef::Registered {
            owner: mush_core::OwnerId::new("economy").expect("owner"),
            id: mush_core::CanonicalId::new("cadence.chronicle.period").expect("id"),
            generation: 1,
        };
        let stamp = tw
            .engine()
            .clock_now(&clock)
            .expect("the declared cadence is registered and readable");
        assert_eq!(stamp.at, 0);
        assert_eq!(stamp.clock, clock);
        let store = tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .expect("economy");
        let c = mush_economy::cadence_by_name(store.snapshot(), "chronicle.period")
            .expect("created at boot");
        assert_eq!(
            (c.schema, c.source, c.period),
            (
                mush_economy::CADENCE_SCHEMA_DECLARED,
                mush_economy::CadenceSource::RealUtc,
                2_592_000
            )
        );
        assert_eq!(
            store.snapshot().cadences().count(),
            1,
            "the WoD layer minted its own"
        );
        // An undeclared id has no provider.
        let other = mush_core::ClockRef::Registered {
            owner: mush_core::OwnerId::new("economy").expect("owner"),
            id: mush_core::CanonicalId::new("cadence.sr.month").expect("id"),
            generation: 1,
        };
        assert!(matches!(
            tw.engine().clock_now(&other),
            Err(mush_core::ClockReadError::ProviderMissing { .. })
        ));
    }

    #[test]
    fn server_modules_register_without_collision() {
        // The full production list (including org/economy and sim/tactics) shares one
        // registry: registration must not collide, and every module must be present
        // in the load-bearing boot order.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(server_modules());
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                // E7 cut 7: the propagation layer, immediately after
                // `mush-tactics`. Its boot reads no tactics state, but its
                // POLICY reads the leak-safe contact ledger tactics publishes
                // and its contexts are built off `mush-sim`'s placement graph,
                // both of which are hydrated by then (`on_world_loaded` runs in
                // list order).
                "propagation",
                "economy",
                // E7 cut 2: immediately after the economy (its boot asks about
                // vault status) and before the market (whose listing quantity is
                // its own advertised counter, never a `Pool` claim).
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "market",
                "script"
            ]
        );
    }

    /// E7 cut 1 (10b §3.1, doc 10 §20): **the TacSim layer composes after the one
    /// character ruleset**, and its order position is what makes that true.
    ///
    /// `on_world_loaded` runs in list order, so registering last is what lets TacSim's
    /// boot read the character ruleset's facts while the ruleset reads TacSim's
    /// environmental facts through the generic seam. `[rulesets].enabled` is untouched
    /// by any of this: it still selects exactly one of `wod` | `shadowrun`, and TacSim
    /// is switched by `[tacsim].enabled` alone.
    #[test]
    fn e7c1_wiring_tacsim_composes_after_the_character_ruleset() {
        use mush_testkit::TestWorld;
        for ruleset in ["wod", "shadowrun"] {
            let onboarding = ruleset_wod::OnboardingConfig::default();
            let wod = ruleset_wod::WodEconomyConfig::default();
            let shadowrun = ruleset_shadowrun::SrEconomyConfig::default();
            let mut mods = server_modules_with(
                mush_weather::WeatherConfig::default(),
                mush_economy::EconomyConfig::default(),
                mush_atlas::AtlasConfig::default(),
                &onboarding,
                (ruleset == "wod").then_some(&wod),
                (ruleset == "shadowrun").then_some(&shadowrun),
                mush_campaign::RecoveryConfig::default(),
            );
            mods.extend(
                ruleset_modules(&[ruleset.to_owned()], &onboarding, &wod, &shadowrun)
                    .expect("one production ruleset"),
            );
            mods.push(tacsim_module(ruleset_tacsim::fixture::config()));
            let tw = TestWorld::with_modules(mods);
            let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
            assert_eq!(
                names,
                [
                    "comsys",
                    "canon",
                    "privileges",
                    "org",
                    "faction",
                    "scenes",
                    "sim",
                    "atlas",
                    "tactics",
                    "propagation",
                    "economy",
                    "inventory",
                    // E7 cut 3: immediately after `mush-inventory`, whose handles
                    // and family delegation it needs, and before `mush-market`,
                    // which it does not touch.
                    "industry",
                    // E7 cut 9: the campaign layer, after economy/org/canon.
                    "campaign",
                    "mail",
                    "jobs",
                    "housing",
                    "weather",
                    "party",
                    "market",
                    "script",
                    ruleset,
                    "tacsim"
                ],
                "the TacSim layer must be last, after the character ruleset"
            );
        }
    }

    /// E7 cut 1: `[rulesets].enabled` is untouched — TacSim is not one of its ids, and
    /// asking for it as a ruleset is still an unknown-ruleset error.
    #[test]
    fn e7c1_wiring_tacsim_is_not_a_ruleset_id() {
        let err = match ruleset_modules(
            &["tacsim".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("tacsim is a layer, not a ruleset id"),
        };
        assert!(err.contains("unknown ruleset"), "{err}");
        // And the single-ruleset guard is unchanged by cut 1.
        let err = match ruleset_modules(
            &["wod".to_string(), "shadowrun".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("two rulesets must be rejected"),
        };
        assert!(err.contains("one ruleset per server"), "{err}");
    }

    #[test]
    fn the_org_capability_vocabulary_covers_every_economy_capability() {
        // E3b2 §4.10: the wiring vocabulary is `mush_economy::CAPABILITIES` ∪
        // `mush_housing::CAPABILITIES` (E5b) ∪ the two org-administrative ids. If a
        // module starts asking about a capability the server does not wire,
        // `@org/grant` would refuse the very grant that module needs — so this is a
        // real coupling, pinned here.
        let vocab = org_capability_vocabulary();
        // E7 cut 9 (OPEN 5): `mush_campaign::CAPABILITIES` joins them. A `force.*`
        // id missing here fails at REGISTRATION — `@org/grant` refuses it — rather
        // than silently at use, which is the whole point of the typo catcher.
        for c in mush_economy::CAPABILITIES
            .iter()
            .chain(mush_housing::CAPABILITIES)
            .chain(mush_campaign::CAPABILITIES)
        {
            let id = mush_org::CapabilityId::new(c).expect("well-formed");
            assert!(vocab.contains(&id), "the wiring vocabulary omits {c}");
        }
        for c in ["org.role.manage", "org.grant.delegate"] {
            assert!(
                vocab.contains(&mush_org::CapabilityId::new(c).expect("well-formed")),
                "{c}"
            );
        }
        assert_eq!(
            vocab.len(),
            mush_economy::CAPABILITIES.len()
                + mush_housing::CAPABILITIES.len()
                + mush_campaign::CAPABILITIES.len()
                + 2
        );
        // …and a `force.*` id that is NOT declared is absent, so a typo'd grant
        // still fails loudly rather than becoming an inert record.
        assert!(
            !vocab.contains(&mush_org::CapabilityId::new("force.comand").expect("well-formed"))
        );
        assert!(
            !vocab.contains(
                &mush_org::CapabilityId::new("economy.acount.post").expect("well-formed")
            )
        );
    }

    #[test]
    fn ruleset_modules_selects_wod() {
        // "wod" resolves to the WoD ruleset module; the id survives boot.
        let mods = ruleset_modules(
            &["wod".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        )
        .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name(), "wod");
        // Case/whitespace-insensitive.
        let mods = ruleset_modules(
            &["  WoD ".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        )
        .unwrap();
        assert_eq!(mods[0].name(), "wod");
        // Empty list is fine (a ruleset-less server).
        assert!(
            ruleset_modules(
                &[],
                &ruleset_wod::OnboardingConfig::default(),
                &ruleset_wod::WodEconomyConfig::default(),
                &ruleset_shadowrun::SrEconomyConfig::default(),
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn ruleset_modules_selects_shadowrun() {
        // "shadowrun" (M10b) resolves to the SR5 ruleset module.
        let mods = ruleset_modules(
            &["shadowrun".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        )
        .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name(), "shadowrun");
    }

    #[test]
    fn ruleset_modules_rejects_two_rulesets_at_once() {
        // Single-ruleset-per-server: enabling wod AND shadowrun (both register
        // +roll/+sheet/+chargen) is caught here with a clear message, not left
        // to surface as an opaque command collision at registration.
        let err = match ruleset_modules(
            &["wod".to_string(), "shadowrun".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("enabling two rulesets must be rejected"),
        };
        assert!(err.contains("one ruleset per server"), "{err}");
    }

    #[test]
    fn ruleset_modules_rejects_unknown_and_duplicate() {
        // `unwrap_err` needs `Debug` on the Ok value; `dyn GameModule` has
        // none, so match the error out by hand.
        //
        // E7 cut 1: the unknown id used to be `"tacsim"`, which is now a real thing —
        // a *layer* switched by `[tacsim].enabled`, deliberately not a `[rulesets]` id
        // (doc 10 §20). Keeping it here would have read as "tacsim is not a ruleset yet",
        // which is the opposite of the decision. `"nosuch"` names nothing and never will.
        let err = match ruleset_modules(
            &["nosuch".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("unknown ruleset must be rejected"),
        };
        assert!(err.contains("unknown ruleset"), "{err}");
        let err = match ruleset_modules(
            &["wod".to_string(), "wod".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("duplicate ruleset must be rejected"),
        };
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn worldgen_registers_in_the_full_boot_list() {
        // main.rs's assembly: built-ins THEN worldgen THEN the config-selected
        // ruleset, sharing one registry. Worldgen registers with a
        // MockProvider-backed service (no network, no key); its @gen/@theme/@llm
        // commands and the worldgen.draft panel feed must all wire in cleanly.
        use mush_llm::{LlmService, MockProvider};
        use mush_testkit::TestWorld;

        let mock: Arc<dyn mush_llm::LlmProvider> = Arc::new(MockProvider::new("openrouter"));
        let llm = Arc::new(
            LlmService::builder()
                .provider("openrouter", mock)
                .global_enabled(false)
                .build(),
        );
        let mut mods = server_modules();
        mods.push(worldgen_module(Arc::clone(&llm)));
        mods.extend(
            ruleset_modules(
                &["wod".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
                &ruleset_wod::WodEconomyConfig::default(),
                &ruleset_shadowrun::SrEconomyConfig::default(),
            )
            .unwrap(),
        );
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                "propagation",
                "economy",
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "market",
                "script",
                "worldgen",
                "wod"
            ]
        );
        assert!(tw.registry().spec_of("@gen").is_some());
        // S2b: the theme moved onto the atlas (`@atlas/theme`); `@theme` is gone.
        assert!(tw.registry().spec_of("@theme").is_none());
        assert!(tw.registry().spec_of("@atlas").is_some());
        assert!(tw.registry().spec_of("@llm").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "worldgen.draft")
        );
    }

    #[test]
    fn npc_module_registers_and_joins_the_panel_manifest() {
        // main.rs builds the web hello manifest from `registry.panel_feeds()`,
        // so registering `npc.thinking` is what puts the thinking indicator in
        // the manifest. This guards that (and that the npc module registers
        // cleanly alongside the built-ins). Dialogue-only grounding is enough.
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.push(npc_module(None));
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(names.contains(&"npc"), "{names:?}");
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "npc.thinking"),
            "npc.thinking must be a registered panel feed (it joins the web manifest)"
        );
        // The @agent staff surface is live.
        assert!(tw.registry().spec_of("@agent").is_some());
    }

    #[test]
    fn ruleset_agent_grounds_a_shadowrun_npc_end_to_end() {
        // The generic grounding adapter carries the API-scaling claim end-to-end:
        // on a shadowrun-only world, an NPC grounds through the Ruleset trait
        // (sheet bootstrap + capability profile), not the concrete WodAgent.
        use mush_npc::AgentGrounding;
        use mush_testkit::TestWorld;
        let mut mods = server_modules(); // carries the jobs module the ruleset needs
        mods.extend(
            ruleset_modules(
                &["shadowrun".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
                &ruleset_wod::WodEconomyConfig::default(),
                &ruleset_shadowrun::SrEconomyConfig::default(),
            )
            .unwrap(),
        );
        let mut tw = TestWorld::with_modules(mods);
        let npc = tw.spawn_player("Ganger");
        let agent = RulesetAgent::new(ruleset_shadowrun::ShadowrunRuleset::new());
        // No sheet yet → dialogue-only (no profile).
        assert!(
            agent.profile(tw.world(), npc).is_none(),
            "an NPC with no sheet does not ground"
        );
        // The `@agent/sheet` bootstrap gives it a real SR5 sheet via Ruleset::init_sheet.
        agent
            .init_sheet(tw.world_mut(), npc, "", npc)
            .expect("SR5 init_sheet writes a sheet");
        // Now it grounds: a capability profile carrying the sr_roll tool.
        let prof = agent
            .profile(tw.world(), npc)
            .expect("SR5 NPC grounds through the generic Ruleset seam after init_sheet");
        assert!(
            prof.tools.iter().any(|t| t.name == "sr_roll"),
            "the grounded SR5 NPC exposes the sr_roll ability: {:?}",
            prof.tools
        );
    }

    #[test]
    fn npc_agent_adapter_grounds_every_enabled_ruleset() {
        // The generic grounding adapter is built for any ruleset that grounds
        // NPCs — wod AND shadowrun (each dispatches through its Ruleset trait
        // impl). A ruleset-less config, or one whose ruleset has no grounding,
        // runs NPCs dialogue-only (None).
        assert!(npc_agent_adapter(&["wod".to_string()]).is_some());
        assert!(npc_agent_adapter(&["  WoD ".to_string()]).is_some());
        assert!(
            npc_agent_adapter(&["shadowrun".to_string()]).is_some(),
            "SR5 NPCs must ground through the generic Ruleset seam, not just WoD"
        );
        assert!(npc_agent_adapter(&["  Shadowrun ".to_string()]).is_some());
        assert!(npc_agent_adapter(&[]).is_none());
        assert!(npc_agent_adapter(&["tacsim".to_string()]).is_none());
    }

    #[test]
    fn full_boot_list_registers_without_collision() {
        // The production concatenation main.rs builds — built-ins THEN the
        // config-selected wod ruleset — shares one registry with no
        // collision, and wod registers last (after jobs, so its approval
        // payload handlers see a loaded jobs module).
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.extend(
            ruleset_modules(
                &["wod".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
                &ruleset_wod::WodEconomyConfig::default(),
                &ruleset_shadowrun::SrEconomyConfig::default(),
            )
            .unwrap(),
        );
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys",
                "canon",
                "privileges",
                "org",
                "faction",
                "scenes",
                "sim",
                "atlas",
                "tactics",
                "propagation",
                "economy",
                "inventory",
                // E7 cut 3: immediately after `mush-inventory`, whose handles
                // and family delegation it needs, and before `mush-market`,
                // which it does not touch.
                "industry",
                // E7 cut 9: the campaign layer, after economy/org/canon — its
                // bond, its `force.*` authority and its root canon record are
                // theirs — and before the player-facing subsystems.
                "campaign",
                "mail",
                "jobs",
                "housing",
                "weather",
                "party",
                "market",
                "script",
                "wod"
            ]
        );
        // The ruleset's surface is live: +roll / +sheet registered, the
        // wod.roll panel feed present, and the chargen/xp job queues too.
        assert!(tw.registry().spec_of("+roll").is_some());
        assert!(tw.registry().spec_of("+sheet").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "wod.roll")
        );
    }
}
