//! # mush-sim — the spatial simulation foundation (design 06, Phase B cut 1)
//!
//! Ordinary objects gain **spatial placement**: a durable, single-parent,
//! acyclic graph of [`SpatialParent`]s over passive **domains**, **interior
//! spaces**, and **mounts**. `Obj.location` becomes a maintained *projection*
//! of placement (design D1), never the truth. This cut ships the records,
//! persistence, boot validation with quarantine, derived indexes, the
//! `sim.*` behaviors, the movement/carry/destroy/detach gates, the ONE
//! transition primitive ([`transition::plan`] / [`transition::apply`]), and the
//! `@domain`/`@spatial` staff surface. Player-facing embark/egress/attach
//! transitions, stations, and domain ticking are later cuts.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(clippy::panic, clippy::unreachable, clippy::panic_in_result_fn)]

// The deny above has no test exemption, so a test or fixture helper that
// returns `Result` checks with `check!`/`check_eq!`/`check_ne!` — its asserts
// as early `Err` returns. Declared before every other module because
// `macro_rules!` is textually scoped. The module file gates itself to test
// builds; this file names no test cfg this early, because its source
// censuses read the production half as the text before the first one.
#[cfg_attr(test, macro_use)]
mod test_checks;

pub mod boot;
pub mod commands;
pub mod custody;
pub mod data;
/// 16b cut T.3: **units a simulation authority holds custody of** (doc 16 §3.1
/// "Custody").
///
/// The world-side index behind the one shared admission helper,
/// [`deployed::refuse_if_deployed`], and the one named error,
/// [`deployed::UnitDeployed`]. The durable marker lives on the unit's own
/// `mush-tactics` record; this is the projection every admission site reads.
pub mod deployed;
pub mod domain;
pub mod driver;
pub mod focus;
pub mod graph;
mod handoff;
pub mod ic_exclusion;
pub mod journey;
pub mod links;
mod locomotion;
mod locomotion_capture;
mod locomotion_composed;
mod locomotion_scenes;
mod locomotion_window;
pub mod movement_custody;
mod movement_reads;
pub use locomotion_composed::ComposedMovementFactory;
mod personnel;
mod personnel_capture;
pub mod physical;
pub mod relay;
pub mod route;
pub mod seat;
pub mod station;
pub mod tick;
pub mod transition;
pub mod window;
pub mod window_movement;
pub mod window_occupancy;

use std::sync::{Arc, OnceLock};

use std::time::Duration;

use mush_core::{
    AccountIdentity, AuthorityGuardKind, BehaviorId, BehaviorSpec, ControlLeaseGuardValidator,
    ControlLeaseStateCodec, Ctx, DefinitionKind, DestroyGate, DestroyPolicy, DetachGate, EventHook,
    GameModule, HookPoint, HookReadContext, ModError, ModuleBootData, ModuleCtx, ObjId, ObjKind,
    Registrar, Registry, Role, StateFamilyId, TickCadence, World, WorldEvent, WorldEventKind,
    control_lease_state_family_descriptor,
};
use mush_proto::RichText;

pub use commands::{
    Departure, MANUAL_ROUTE_STAFF_ONLY, ROUTE_NOT_FROM_HERE, depart, depart_ineligible,
};
pub use custody::InteriorCustodyProvider;
pub use data::{
    CapacitySpec, DEFAULT_RECOVERY_ROOM, Derived, DomainClock, DomainId, DomainRecord,
    DomainStatus, EgressSpec, GeometryFn, HeldPlacement, HostAssignments, HostMounts, HostStations,
    InteriorSpaceRecord, MAX_GRANTED_CHANNELS, MAX_SEATS_PER_UNIT, MAX_SPACE_ROOMS,
    MAX_SPATIAL_DEPTH, MetricGeometry, MountId, MountSpec, NS, PASSIVE_DRIVER, PlacementRecord,
    Pose, Presence, ResolveFn, RoomResolver, RulesValue, SlotId, SpaceAnchor, SpatialCounterError,
    SpatialParent, SpatialState, StationAssignment, StationId, StationSpec, UnplacedInfo,
    WindowBox, WindowGeometry, WindowOpener, WindowPin, WindowRec, WindowSurface, cgen_key,
    passive_definition_ref, passive_rules_value,
};
pub use deployed::{DeployedMark, DeployedUnits, UnitDeployed, refuse_if_deployed};
pub use domain::{
    ActivationBook, DomainActivation, DomainActivationWire, DomainSummary, DomainSummaryWire,
    DomainWorkflowError, MAX_ACTIVE_DOMAINS, PromotionTrigger, QUIET_BOUNDARY, activation_key,
    summary_key,
};
pub use focus::{
    AccountOracle, ActiveFocus, FocusState, MAX_BILLETS_LISTED, ProxyAct, ProxyDrive, ReachProof,
    driving_actor, proxy_drive_of, seat_restriction,
};
pub use graph::{
    anchored_space_of, effective_domain, presence, project, space_domain, verify_indexes,
    verify_projections,
};
pub use journey::{
    CarrierHooks, CarrierHooksState, CarrierJourney, CarrierJourneyError, CarrierJourneyWire,
    JourneyBook, JourneyId, JourneyState, MAX_CARRIER_JOURNEYS, MAX_CARRIER_SUBTREE,
    MAX_JOURNEY_ANOMALY_BYTES, MAX_JOURNEYS_VISITED_PER_FIRE, MAX_JOURNEYS_WORKED_PER_FIRE,
    carrier_journey_key, same_place, under_way_root_of,
};
pub use route::{
    CadenceId, DistanceLaw, DriveProfile, DriveProfileWire, LegDef, LegDefWire, MAX_HAZARD_BAND,
    MAX_LEGS, MAX_ROUTE_ID_BYTES, RouteCatalog, RouteCatalogState, RouteDef, RouteDefError,
    RouteDefWire, laden_leg_durations, route_duration, route_duration_at,
};
pub use seat::{
    ControlLeaseView, MAX_LEASE_VIEW_RECORDS, MAX_STATION_SERVICE_ROWS, SeatError, SeatRequest,
    SeatTicket, ServiceBook, SimFactProvider, StationService, channel_lease_holder,
    request_seat_activation, service_key,
};
pub use station::{
    MAX_STATION_FALLBACKS, StationFallback, StationFallbacks, StationTopologyError,
    declare_station, vacate_station,
};
pub use transition::{Plan, SpatialError, Transition, apply, move_carrier_subtree, plan, run};
pub use window::{
    MAX_WINDOWS, OpenWindow, arc_between, close_window, declare_compartment, metric_between,
    open_passive_domain, open_window, participants, place, place_interior, pose_of,
    raw_participants, register_room_resolver, register_window_arc, register_window_geometry,
    unplace, update_window, window_of, windows,
};

/// Module configuration.
#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Where quarantined and unplaced objects are projected (design D4 item 3).
    pub recovery_room: ObjId,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            recovery_room: DEFAULT_RECOVERY_ROOM,
        }
    }
}

/// The spatial simulation module (design §3.2). Phase C adds domain drivers
/// through a builder on this struct.
#[derive(Default)]
pub struct SimulationModule {
    config: SimConfig,
    /// The account-distinctness oracle (E7 cut 5 B1), installed at the
    /// composition root through [`SimulationModule::with_accounts`] — the
    /// `EconomyModule::with_facts` / `IndustryModule::with_recipes` shape. It is
    /// carried into world state at `register` as a [`focus::AccountOracle`] so
    /// command handlers reach it synchronously; `None` leaves the oracle
    /// unwired and every consumer fails closed.
    accounts: Option<Arc<dyn AccountIdentity>>,
    /// The composition-installed route catalog (E7 cut 6 fold, B6; landed with
    /// cut 7). `None` leaves `@journey/plan|depart <route id>` on the manual
    /// fixture-route fallback, which is exactly what it did before.
    routes: Option<Arc<dyn route::RouteCatalog>>,
    /// Doc 18 cut IC9 (lead ruling L8): the composition-installed carrier hooks
    /// (the manual-departure refusal and the two narration points). `None`
    /// leaves today's behaviour: no refusal, the generic aboard lines only.
    carrier_hooks: Option<Arc<dyn journey::CarrierHooks>>,
    /// E7c8 B1: the live pulse cadence (§7's `GROUND_PULSE_MS`/`SPACE_PULSE_MS`
    /// by default). Carried into world state at `register`, where the
    /// composition root may upgrade it from `[tacsim]`.
    pulse: tick::PulseConfig,
    ic_custody: Option<ic_exclusion::IcExclusionConfig>,
    atomic_scene_movement:
        Option<Arc<OnceLock<mush_core::locomotion::scene_transition::SceneLocomotionReader>>>,
    composed_movement: Option<ComposedMovementFactory>,
    window_movement: Option<(window_movement::WindowSelector, ComposedMovementFactory)>,
    tactics_movement_reads: Option<movement_custody::CustodyReadFactory>,
}

impl std::fmt::Debug for SimulationModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationModule")
            .field("config", &self.config)
            .field("accounts", &self.accounts.is_some())
            .field("routes", &self.routes.is_some())
            .field("carrier_hooks", &self.carrier_hooks.is_some())
            .field(
                "atomic_scene_movement",
                &self.atomic_scene_movement.is_some(),
            )
            .field("composed_movement", &self.composed_movement.is_some())
            .field("window_movement", &self.window_movement.is_some())
            .field(
                "tactics_movement_reads",
                &self.tactics_movement_reads.is_some(),
            )
            .finish()
    }
}

impl SimulationModule {
    /// With default configuration.
    #[must_use]
    pub fn new() -> SimulationModule {
        SimulationModule::default()
    }

    /// With explicit configuration.
    #[must_use]
    pub fn with_config(config: SimConfig) -> SimulationModule {
        SimulationModule {
            config,
            accounts: None,
            routes: None,
            carrier_hooks: None,
            pulse: tick::PulseConfig::default(),
            ic_custody: None,
            atomic_scene_movement: None,
            composed_movement: None,
            window_movement: None,
            tactics_movement_reads: None,
        }
    }

    /// Opt into Sim's dedicated IC exclusion owner. The actual four-owner
    /// union and mandatory final joins remain required before any activation.
    #[must_use]
    pub fn with_ic_mind_custody(mut self, config: ic_exclusion::IcExclusionConfig) -> Self {
        self.ic_custody = Some(config);
        self
    }

    /// Compose the closed captured-locomotion action with Scenes' real atomic
    /// transcript participant. The Scenes module must opt in as well. This
    /// installs no command route and does not certify personnel history.
    #[must_use]
    pub fn with_atomic_scene_movement(
        mut self,
        reader: Arc<OnceLock<mush_core::locomotion::scene_transition::SceneLocomotionReader>>,
    ) -> Self {
        self.atomic_scene_movement = Some(reader);
        self
    }

    /// Join composition-supplied actual observer adapters to the closed Sim
    /// movement primary. This registers no command route or legal-action offer.
    #[must_use]
    pub fn with_composed_movement(mut self, factory: ComposedMovementFactory) -> Self {
        self.composed_movement = Some(factory);
        self
    }

    /// Add the closed v2 profile with actual observer adapters. The selector
    /// proposes data only; Atlas independently attests it. No live route,
    /// offer, completed output or personnel eligibility is granted.
    #[must_use]
    pub fn with_window_movement(
        mut self,
        selector: window_movement::WindowSelector,
        factory: ComposedMovementFactory,
    ) -> Self {
        self.window_movement = Some((selector, factory));
        self
    }

    /// Require the fixed actual Tactics custody read in every closed movement
    /// preparation. The Tactics module must register the matching service; no
    /// caller callback can stand in for the retained Core read observation.
    #[must_use]
    pub fn with_tactics_movement_reads<S>(mut self) -> Self
    where
        S: mush_core::read_service::OwnerReadService<
                Request = movement_custody::MovementCustodyRequest,
                Output = movement_custody::MovementCustodyFacts,
            >,
    {
        self.tactics_movement_reads = Some(movement_custody::factory::<S>());
        self
    }

    /// Install the pulse cadence from `[tacsim].ground_pulse_ms` /
    /// `space_pulse_ms` (E7c8 B1) — the `with_accounts`/`with_route_catalog`
    /// shape. Unwired leaves §7's constants, which is what cut 7 did.
    #[must_use]
    pub fn with_pulse(mut self, pulse: tick::PulseConfig) -> SimulationModule {
        self.pulse = pulse;
        self
    }

    /// Install the account-distinctness oracle (E7 cut 5 B1, OPEN 4). Called at
    /// `server_modules_with` with an `Arc<dyn AccountIdentity>` the server built
    /// from a boot-hydrated character → account-id map; the world consults it
    /// synchronously inside a command. Mirrors `EconomyModule::with_facts`.
    #[must_use]
    pub fn with_accounts(mut self, accounts: Arc<dyn AccountIdentity>) -> SimulationModule {
        self.accounts = Some(accounts);
        self
    }

    /// Install the route catalog (E7 cut 6 fold B6, discharged by cut 7).
    ///
    /// `tacsim` depends on `mush-sim`, so resolving a pack-authored
    /// route id by an edge would be a cycle; the implementation is handed in at
    /// `server_modules_with` instead, exactly as [`Self::with_accounts`] and
    /// `IndustryModule::with_recipes` are. Unwired leaves the manual
    /// fixture-route path, which is the pre-cut-7 behaviour verbatim.
    #[must_use]
    pub fn with_route_catalog(mut self, routes: Arc<dyn route::RouteCatalog>) -> SimulationModule {
        self.routes = Some(routes);
        self
    }

    /// Install the carrier hooks (doc 18 cut IC9, lead ruling L8) — the
    /// [`Self::with_route_catalog`] shape, for the same reason: the game layer
    /// depends on `mush-sim`, so its passage rules are handed in at the
    /// composition root. Unwired leaves the pre-IC9 behaviour verbatim.
    #[must_use]
    pub fn with_carrier_hooks(mut self, hooks: Arc<dyn journey::CarrierHooks>) -> SimulationModule {
        self.carrier_hooks = Some(hooks);
        self
    }

    /// These audits read registered handles and already-hydrated typed history,
    /// not live placement projections or other modules' loaded state.
    fn audit_movement_history(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        physical::loaded(ctx)?;
        locomotion::loaded(ctx)?;
        if self.atomic_scene_movement.is_some() {
            locomotion_scenes::loaded(ctx)?;
        }
        if self.composed_movement.is_some() {
            locomotion_composed::loaded(ctx)?;
        }
        if self.window_movement.is_some() {
            locomotion_window::audit_typed(ctx)?;
        }
        Ok(())
    }

    fn audit_native_movement_history(&self, data: &ModuleBootData) -> Result<(), ModError> {
        // Registration of passive history handles alone does not configure a
        // publisher or its recovery join. Never restore this journal through
        // the generic legacy recovery loader while that profile is unavailable.
        let log = window_movement::WINDOW_MOVEMENT_LOG;
        let key = format!("log:{log}");
        let prefix = format!("{key}:");
        let belongs = |candidate: &str| candidate == key || candidate.starts_with(&prefix);
        if data.ledger.iter().any(|(key, _)| belongs(key))
            || data.kv.iter().any(|(key, _)| belongs(key))
            || data.omitted.get(log).is_some_and(|floor| *floor > 0)
        {
            return Err(ModError::msg(
                "window movement journal requires a configured native publisher and recovery audit",
            ));
        }
        Ok(())
    }
}

fn bid(s: &str) -> BehaviorId {
    BehaviorId::new(s).expect("static behavior id")
}

fn domain_spec() -> BehaviorSpec {
    let mut s = BehaviorSpec::new(bid("sim.domain"));
    s.allowed_kinds = vec![ObjKind::Room, ObjKind::Thing];
    s.destroy = DestroyPolicy::Refuse;
    s.min_role = Role::Wizard;
    s.help =
        "Hosts a simulation domain (@domain/create). Refuses @destroy; retire the domain first."
            .into();
    s
}

fn interior_spec() -> BehaviorSpec {
    let mut s = BehaviorSpec::new(bid("sim.interior"));
    s.allowed_kinds = vec![ObjKind::Room];
    s.destroy = DestroyPolicy::Refuse;
    s.min_role = Role::Wizard;
    s.help =
        "An interior space (a hangar, a bay) that can hold placed units (@spatial/space).".into();
    s
}

fn participant_spec() -> BehaviorSpec {
    let mut s = BehaviorSpec::new(bid("sim.participant"));
    s.allowed_kinds = vec![ObjKind::Thing, ObjKind::Player, ObjKind::Npc];
    s.help = "A spatial participant: may be placed in a domain, an interior, or on a mount.".into();
    s
}

fn state(w: &World) -> Option<&SpatialState> {
    w.module_state::<SpatialState>()
}

/// The movement/carry gate: a placed (direct, interior, or attached) or
/// quarantined object moves only through the simulation, or staff recovery.
/// Player-driven egress is a later cut. Staff are not exempt (design §7.5).
struct PlacementGate;

/// The held and quarantined arms of the gate, which win before every other
/// (the member arm of [`move_reason`] included).
fn held_or_quarantined(st: &SpatialState, w: &World, obj: ObjId) -> Option<String> {
    if let Some(held) = st.held_placement(obj) {
        return Some(format!(
            "{} has a held unreadable placement ({}); staff must repair or migrate it.",
            mush_core::output::display_name(w, obj),
            held.key
        ));
    }
    if st.is_quarantined(obj) {
        return Some(format!(
            "{} is quarantined; staff recover it with @spatial/recover.",
            mush_core::output::display_name(w, obj)
        ));
    }
    None
}

/// The carrier a multi-room space rides (its `MovingObject` host), else the
/// space object itself — the name the aboard sentence and the egress refusal
/// give (cut IC4).
pub(crate) fn aboard_host(st: &SpatialState, space: ObjId) -> ObjId {
    match st.spaces.get(&space).map(|s| &s.anchor) {
        Some(crate::data::SpaceAnchor::MovingObject { host }) => *host,
        _ => space,
    }
}

/// Whether `space` is a multi-room interior (lead ruling L3: `rooms.len() >
/// 1`; a legacy empty `rooms` is single-room).
pub(crate) fn is_multi_room(st: &SpatialState, space: ObjId) -> bool {
    st.spaces.get(&space).is_some_and(|s| s.rooms.len() > 1)
}

fn placed_or_quarantined(w: &World, obj: ObjId) -> Option<String> {
    let st = state(w)?;
    if let Some(reason) = held_or_quarantined(st, w, obj) {
        return Some(reason);
    }
    parent_reason(st, w, obj)
}

/// The by-parent arm of the gate (after held and quarantined).
fn parent_reason(st: &SpatialState, w: &World, obj: ObjId) -> Option<String> {
    match st.parent_of(obj) {
        SpatialParent::Direct { domain, .. } => {
            // S4: a domain with a window is walkable ground — its participants
            // leave by the same exits as anyone, and the window's owner re-poses
            // or releases them on the move (doc 14 §3.4).
            if st.domains.get(&domain).is_some_and(|d| d.window.is_some()) {
                return None;
            }
            Some(format!(
                "{} is placed in domain #{domain}; it moves only through the simulation, or staff recovery (@spatial).",
                mush_core::output::display_name(w, obj)
            ))
        }
        // Cut IC4 (doc 18 §4.2 (a)): aboard a multi-room interior the
        // sentence says where the occupant MAY go; a single-room space keeps
        // the original sentence byte for byte.
        SpatialParent::Interior { space, .. } if is_multi_room(st, space) => {
            let host = mush_core::output::display_name(w, aboard_host(st, space));
            Some(format!(
                "{} is aboard {host}; it moves only between {host}'s rooms, or leaves through the simulation or staff recovery (@spatial).",
                mush_core::output::display_name(w, obj),
            ))
        }
        SpatialParent::Interior { space, .. } => Some(format!(
            "{} is inside {}; it leaves only through the simulation, or staff recovery (@spatial).",
            mush_core::output::display_name(w, obj),
            mush_core::output::display_name(w, space)
        )),
        SpatialParent::Attached { host, .. } => Some(format!(
            "{} is mounted on {}; detach it through the simulation, or staff recovery (@spatial).",
            mush_core::output::display_name(w, obj),
            mush_core::output::display_name(w, host)
        )),
        SpatialParent::Unplaced => None,
    }
}

impl EventHook for PlacementGate {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        if ctx
            .physical_transition_attempt()
            .is_some_and(|attempt| attempt.matches_actor_projection(actor, dest))
        {
            return Ok(());
        }
        match move_reason(ctx.world(), actor, exit, dest) {
            Some(reason) => Err(RichText::tag("error", reason)),
            None => Ok(()),
        }
    }
    fn carry_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        mover: ObjId,
        obj: ObjId,
        dest: ObjId,
    ) -> Result<(), RichText> {
        // Only Core can supply this attribution, after preparing the exact Sim
        // participant for this action/commit. Other modules' hooks still run.
        if ctx.physical_transition_attempt().is_some_and(|attempt| {
            attempt.matches_projection(mover, obj, dest)
                && attempt.action().key().owner.as_str() == NS
                && attempt.action().key().id.as_str() == mush_core::physical_transition::ACTION
        }) {
            return Ok(());
        }
        match gate_reason(ctx.world(), obj) {
            Some(reason) => Err(RichText::tag("error", reason)),
            None => Ok(()),
        }
    }

    /// **Doc 16 §3.6's world-side exclusion, on the `look` surface** (16b cut S.2).
    ///
    /// A deployed unit's world placement is frozen in the arena's muster room and
    /// its `Obj.location` still points there, so `look` would list it by name beside
    /// the pods. `mush-core`'s room renderer cannot ask `mush-sim` anything directly
    /// — it is the layer below — but it already consults every `Perceive` hook for
    /// exactly this kind of per-observer projection, and that gate is **not waived
    /// for staff**, which is what doc 16 asks for. So the exclusion arrives on the
    /// one seam built for it, and it arrives through
    /// [`crate::deployed::excluded`] like every other listing site.
    ///
    /// `is_concealed` is deliberately left at its default `false`: a unit an arena
    /// holds is not *hiding*, it is somewhere else, and the ruleset-agnostic
    /// notability question is about concealment effects.
    fn perceives(
        &self,
        ctx: &HookReadContext<'_>,
        _observer: ObjId,
        subject: ObjId,
    ) -> mush_core::module::Perceive {
        if crate::deployed::excluded(ctx.world(), crate::deployed::Shown::Unit(subject)) {
            return mush_core::module::Perceive::Conceal;
        }
        mush_core::module::Perceive::Abstain
    }
}

/// The full movement veto: a traveling object (route.rs), a carrier under way
/// (journey.rs) OR a placed/quarantined sim object moves only through its
/// sanctioned mover.
///
/// **E7c6:** the journey arm is load-bearing, not decoration. A cruising carrier
/// root is deliberately *not* placed (only its subtree is), so
/// `placed_or_quarantined` says nothing about it — without this arm the
/// move/carry hooks would let anyone pick a ship up mid-passage, and the ONE
/// checked `move_carrier_subtree` would stop being the only way a carrier moves.
fn gate_reason(w: &World, obj: ObjId) -> Option<String> {
    w.module_state::<journey::JourneyBook>()
        .and_then(|book| journey::journeying_reason(book, w, obj))
        .or_else(|| placed_or_quarantined(w, obj))
}

/// **The destination-aware movement veto** (doc 06 §7.5's boundary rule; doc
/// 18 §4.2 (a), D-18-22, cut IC4). In order: (1) the journeying arm; (2) a
/// held placement; (3) quarantine; (4) **the member arm** — an actor placed
/// `Interior` in a space that walks an EXIT (`exit.is_some()`) into one of
/// that space's member rooms is allowed, and writes no placement; (5) the
/// by-parent refusal. `home`, `@teleport` and every other exit-less move pass
/// `exit: None`, so they stay refused for every occupant, staff included
/// (design §7.5: staff are not exempt). No condition on the actor's current
/// room or on the space's size: an exit into a member is the whole test.
/// [`gate_reason`] (and so `carry_attempt`) has no member arm: carrying a
/// placed object is not walking.
fn move_reason(w: &World, actor: ObjId, exit: Option<ObjId>, dest: ObjId) -> Option<String> {
    if let Some(reason) = w
        .module_state::<journey::JourneyBook>()
        .and_then(|book| journey::journeying_reason(book, w, actor))
    {
        return Some(reason);
    }
    let st = state(w)?;
    if let Some(reason) = held_or_quarantined(st, w, actor) {
        return Some(reason);
    }
    if exit.is_some()
        && let SpatialParent::Interior { space, .. } = st.parent_of(actor)
        && st
            .spaces
            .get(&space)
            .is_some_and(|s| s.rooms.contains(&dest))
    {
        return None;
    }
    parent_reason(st, w, actor)
}

/// The structural destroy veto (design D4/D8), independent of bindings.
struct StructuralDestroyGate;

impl DestroyGate for StructuralDestroyGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        _actor: ObjId,
        target: ObjId,
    ) -> Result<(), String> {
        let Some(st) = state(w) else { return Ok(()) };
        if let Some(held) = st.held_placement(target) {
            return Err(format!(
                "{} has a held unreadable placement ({}); repair or migrate it before destruction.",
                name(w, target),
                held.key
            ));
        }
        if target == st.recovery_room && target != mush_core::LIMBO {
            return Err(format!(
                "{} is the spatial recovery room; reconfigure it first.",
                name(w, target)
            ));
        }
        if let Some(d) = st.domain_of_host(target) {
            return Err(format!(
                "{} hosts domain #{}; retire it first (@domain/retire).",
                name(w, target),
                d.id
            ));
        }
        if let Some(ds) = st.derived.presentation_of.get(&target) {
            let live: Vec<String> = ds
                .iter()
                .filter(|d| {
                    st.domains
                        .get(d)
                        .is_some_and(|r| r.status != DomainStatus::Retired)
                })
                .map(|d| format!("#{d}"))
                .collect();
            if !live.is_empty() {
                return Err(format!(
                    "{} is the presentation room of domain(s) {}; re-point them first (@domain/present).",
                    name(w, target),
                    live.join(", ")
                ));
            }
        }
        if st.spaces.contains_key(&target) {
            return Err(format!(
                "{} is an interior space; @spatial/unspace it first.",
                name(w, target)
            ));
        }
        // Cut IC4 (doc 18 §4.2 (c)): a member room is part of its space's
        // interior; destroying it would strand whoever walks it. The space
        // object's self-entry (L4) is the arm above.
        if let Some(space) = st.derived.member_of.get(&target)
            && *space != target
        {
            return Err(format!(
                "{} is a room of the interior space {}; @spatial/unspace that space first.",
                name(w, target),
                name(w, *space)
            ));
        }
        if let Some(spaces) = st.derived.anchored_spaces.get(&target) {
            let occupied: Vec<String> = spaces
                .iter()
                .filter(|s| !st.children_of(**s).is_empty())
                .map(|s| name(w, *s))
                .collect();
            if !occupied.is_empty() {
                return Err(format!(
                    "{} carries occupied interior(s) {}; empty them first.",
                    name(w, target),
                    occupied.join(", ")
                ));
            }
        }
        if let Some(a) = st.assignments.get(&target)
            && !a.seats.is_empty()
        {
            return Err(format!(
                "{} has {} crewed station(s); @spatial/unassign first.",
                name(w, target),
                a.seats.len()
            ));
        }
        let deps = st.children_of(target).len();
        if deps > 0 {
            return Err(format!(
                "{} still has {deps} placed dependent(s); move them first.",
                name(w, target)
            ));
        }
        Ok(())
    }
}

/// The detach veto: a `sim.*` binding cannot be stripped while records
/// depend on it.
struct SimDetachGate;

impl DetachGate for SimDetachGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        _actor: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String> {
        let Some(st) = state(w) else { return Ok(()) };
        match behavior.as_str() {
            "sim.domain" => {
                if let Some(d) = st.domain_of_host(obj) {
                    return Err(format!(
                        "{} still hosts domain #{}; @domain/retire it first.",
                        name(w, obj),
                        d.id
                    ));
                }
            }
            "sim.interior" => {
                if st.spaces.contains_key(&obj) {
                    return Err(format!(
                        "{} is still an interior space; @spatial/unspace it first.",
                        name(w, obj)
                    ));
                }
            }
            "sim.participant" if st.held_placement(obj).is_some() => {
                return Err(format!(
                    "{} has a held unreadable placement; repair or migrate it first.",
                    name(w, obj)
                ));
            }
            "sim.participant" if st.is_quarantined(obj) => {
                return Err(format!(
                    "{} is quarantined; @spatial/recover it first.",
                    name(w, obj)
                ));
            }
            "sim.participant" if st.is_placed(obj) => {
                return Err(format!(
                    "{} is still placed; @spatial/unplace it first.",
                    name(w, obj)
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

fn name(w: &World, id: ObjId) -> String {
    mush_core::output::display_name(w, id)
}

/// `ObjDestroyed` backstop for programmatic `World::destroy` (the `@destroy`
/// path is refused by the gates above): retire what the husk hosted and
/// quarantine what depended on it, per the domain-loss rule.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    // **16b cut F.4**: the account map's other half. A destroyed character is
    // forgotten, so the id (never recycled) cannot answer an account question
    // for a husk. The oracle behind this holder is the same instance the engine
    // holds, so one call clears both readers; an unwired oracle and a character
    // that was never mapped both answer `false` and cost one lookup.
    if let Some(oracle) = ctx.w.module_state::<focus::AccountOracle>()
        && oracle.unlink(obj)
    {
        tracing::info!(
            character = obj.0,
            "sim: a destroyed character left the account map"
        );
    }
    // E7c6: a destroyed carrier root drops its journey, so the seal lifts with
    // the husk and no ghost record keeps a dead subtree "under way".
    journey::on_root_destroyed(ctx, obj);
    let Some(st) = state(ctx.w) else { return };
    // Snapshot what to do from the records BEFORE mutating.
    let hosted: Option<DomainId> = st.domain_of_host(obj).map(|d| d.id);
    let direct_in_hosted: Vec<ObjId> = hosted
        .and_then(|d| st.derived.domain_direct.get(&d))
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let fixed_spaces: Vec<ObjId> = hosted
        .and_then(|d| st.derived.spaces_in_domain.get(&d))
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let was_space = st.spaces.contains_key(&obj);
    let riding_spaces: Vec<ObjId> = st
        .derived
        .anchored_spaces
        .get(&obj)
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let dependents: Vec<ObjId> = st.children_of(obj).iter().copied().collect();
    let was_placed = st.placements.contains_key(&obj);
    // Cut IC4: the space this husk was a member room of (never its own
    // self-entry), handled last by `drop_destroyed_member`.
    let member_space: Option<ObjId> = st
        .derived
        .member_of
        .get(&obj)
        .copied()
        .filter(|space| *space != obj);
    // Units this actor (if it is one) crews — its seats die with it.
    let actor_seats: Vec<ObjId> = st
        .derived
        .assignments_by_actor
        .get(&obj)
        .map(|us| us.iter().copied().collect())
        .unwrap_or_default();
    // Live domains whose presentation room IS the husk (excluding one this husk
    // also hosts, handled above): their directs now project into a dead room.
    let orphaned_domains: Vec<DomainId> = st
        .derived
        .presentation_of
        .get(&obj)
        .map(|ds| ds.iter().copied().filter(|d| Some(*d) != hosted).collect())
        .unwrap_or_default();
    let directs_of_orphaned: Vec<ObjId> = orphaned_domains
        .iter()
        .flat_map(|d| {
            st.derived
                .domain_direct
                .get(d)
                .into_iter()
                .flatten()
                .copied()
        })
        .collect();
    // Every actor whose crew link must be re-derived: the FULL subtree of every
    // quarantine root (Quarantine unplaces subtrees, so a crewed grandchild is
    // disturbed too), plus the husk's own crew (its seat record dies).
    let crew_refresh: std::collections::BTreeSet<ObjId> = {
        let mut set = std::collections::BTreeSet::new();
        for root in dependents
            .iter()
            .chain(direct_in_hosted.iter())
            .chain(directs_of_orphaned.iter())
        {
            for unit in st.subtree(*root) {
                if let Some(a) = st.assignments.get(&unit) {
                    set.extend(a.seats.values().map(|seat| seat.actor));
                }
            }
        }
        if let Some(a) = st.assignments.get(&obj) {
            set.extend(a.seats.values().map(|seat| seat.actor));
        }
        set
    };

    // Quarantine dependents (occupants/attachments) and direct participants of
    // a lost domain; interior/attached records under OTHER hosts stay intact.
    for root in dependents
        .iter()
        .chain(direct_in_hosted.iter())
        .chain(directs_of_orphaned.iter())
    {
        let _ = run(
            ctx,
            &Transition::Quarantine {
                root: *root,
                reason: format!("{obj} was destroyed"),
            },
        );
    }
    // Spaces fixed in the lost domain, or riding the husk, become unanchored.
    for sp in fixed_spaces.iter().chain(riding_spaces.iter()) {
        let _ = run(
            ctx,
            &Transition::Space {
                space: *sp,
                anchor: SpaceAnchor::Unanchored,
            },
        );
    }
    // A domain whose presentation room died is paused: it cannot project.
    for d in &orphaned_domains {
        let paused = ctx.w.module_state_mut::<SpatialState>().and_then(|s| {
            let rec = s.domains.get_mut(d)?;
            (rec.status == DomainStatus::Active).then(|| {
                // Losing the presentation room is an infallible safety
                // backstop. At exhaustion, retain MAX and leave the domain
                // permanently paused rather than keeping it executable.
                let revision = rec.revision.saturating_add(1);
                if revision == rec.revision {
                    tracing::error!(
                        domain = %d,
                        "sim: domain revision exhausted; pausing at terminal revision"
                    );
                }
                rec.status = DomainStatus::Paused;
                rec.revision = revision;
                rec.clone()
            })
        });
        if let Some(rec) = paused {
            data::persist_domain(ctx.out, &rec);
        }
    }
    // Retire the records the husk owned.
    let s = ctx
        .w
        .module_state_mut::<SpatialState>()
        .expect("sim state slot installed at registration");
    let mut dels: Vec<smol_str::SmolStr> = Vec::new();
    if let Some(d) = hosted
        && let Some(rec) = s.domains.get_mut(&d)
    {
        // Retirement on destruction is terminal and cannot be allowed to
        // fail. Saturating at MAX closes the domain while making every future
        // non-terminal revision change refuse.
        let revision = rec.revision.saturating_add(1);
        if revision == rec.revision {
            tracing::error!(
                domain = %d,
                "sim: domain revision exhausted; retiring at terminal revision"
            );
        }
        rec.status = DomainStatus::Retired;
        rec.revision = revision;
        let rec = rec.clone();
        data::persist_domain(ctx.out, &rec);
    }
    if was_space {
        s.spaces.remove(&obj);
        dels.push(data::space_key(obj));
    }
    if s.mounts.remove(&obj).is_some() {
        dels.push(data::mounts_key(obj));
    }
    if was_placed {
        s.placements.remove(&obj);
        dels.push(data::placement_key(obj));
    }
    // The husk's own control surface dies with it.
    if s.stations.remove(&obj).is_some() {
        dels.push(data::stations_key(obj));
    }
    if s.assignments.remove(&obj).is_some() {
        dels.push(data::assignment_key(obj));
    }
    // The husk's own control epoch dies with it (a reused ObjId starts clean;
    // the epoch is only a SECONDARY guard behind the link/channel re-check).
    if s.control_gen.remove(&obj).is_some() {
        dels.push(data::cgen_key(obj));
    }
    // If the husk was an ACTOR, drop its seat from every unit it crewed and bump
    // each such unit's control epoch (its crew changed).
    for unit in &actor_seats {
        if let Some(ha) = s.assignments.get_mut(unit) {
            ha.seats.retain(|_, seat| seat.actor != obj);
            let rec = ha.clone();
            if rec.seats.is_empty() {
                s.assignments.remove(unit);
            }
            data::persist_assignments(ctx.out, &rec);
        }
        match s.bump_control_gen(*unit) {
            Ok(generation) => data::persist_control_gen(ctx.out, *unit, generation),
            Err(error) => {
                // Revocation is already enforced by the assignment/link
                // removal and execution-time channel check. Keep MAX so no
                // later grant can reuse this epoch, and complete destruction.
                let generation = s.control_generation(*unit);
                tracing::error!(
                    unit = %unit,
                    %error,
                    "sim: control generation exhausted while revoking destroyed actor"
                );
                data::persist_control_gen(ctx.out, *unit, generation);
            }
        }
    }
    s.derived = graph::rebuild(s);
    for k in dels {
        data::persist_del(ctx.out, k);
    }
    // Re-derive control links for every affected actor (World::destroy already
    // scrubbed links whose actor/subject is the husk; a merely-quarantined unit
    // must also stop granting control).
    for actor in crew_refresh {
        links::refresh_actor_links(ctx.w, actor);
    }
    if let Some(space) = member_space {
        drop_destroyed_member(ctx, obj, space);
    }
}

/// **The programmatic-destroy backstop for a member room** (doc 18 §4.2 (c),
/// cut IC4). The destroy gate refuses `@destroy` of a member, but a
/// programmatic destroy skips it: the husk leaves its space's `rooms` (the
/// record re-persisted, the indexes rebuilt), and then the walkers fail closed
/// into the space. `World::destroy` has already sent the dead room's contents
/// to their homes (or Limbo), so a walker who stood in it is outside the ship
/// while still placed `Interior` in the space; every such occupant whose
/// location now disagrees with its projection (the space object) is moved
/// there, as boot reconcile does. Afterwards `verify_projections` is empty.
fn drop_destroyed_member(ctx: &mut Ctx<'_>, room: ObjId, space: ObjId) {
    let Some(s) = ctx.w.module_state_mut::<SpatialState>() else {
        return;
    };
    let Some(rec) = s.spaces.get_mut(&space) else {
        return;
    };
    if !rec.rooms.remove(&room) {
        return;
    }
    let rec = rec.clone();
    s.derived = graph::rebuild(s);
    data::persist_space(ctx.out, &rec);
    tracing::warn!(
        room = %room,
        space = %space,
        "sim: a destroyed member room left its interior space"
    );
    let moves: Vec<(ObjId, ObjId)> = match state(ctx.w) {
        Some(st) => st
            .children_of(space)
            .iter()
            .copied()
            .filter(|&c| {
                matches!(st.parent_of(c), SpatialParent::Interior { space: sp, .. } if sp == space)
            })
            .filter_map(|c| {
                let dest = graph::project(st, ctx.w, c)?;
                (ctx.w.get(c).and_then(|o| o.location) != Some(dest)).then_some((c, dest))
            })
            .collect(),
        None => Vec::new(),
    };
    for (c, dest) in moves {
        match ctx.w.move_to(c, dest) {
            Ok(_) => tracing::warn!(
                object = %c,
                space = %space,
                destroyed = %room,
                "sim: an occupant of a destroyed member room failed closed into its space"
            ),
            Err(error) => tracing::error!(
                object = %c,
                space = %space,
                destroyed = %room,
                %error,
                "sim: could not return an occupant of a destroyed member room to its space"
            ),
        }
    }
}

impl GameModule for SimulationModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.definition_validator(
            DefinitionKind::new("schema").map_err(|error| ModError::msg(error.to_string()))?,
            data::validate_schema_definition,
        )?;
        let passive = reg.definition(data::passive_definition_descriptor())?;
        debug_assert_eq!(passive, data::passive_definition_ref());
        let lease_codec = ControlLeaseStateCodec;
        let control_leases = reg.state_family(
            control_lease_state_family_descriptor(
                StateFamilyId::new("control_leases")
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            lease_codec,
            lease_codec,
            lease_codec,
            lease_codec,
        )?;
        reg.authority_guard_validator(
            AuthorityGuardKind::ControlLease,
            ControlLeaseGuardValidator::new(control_leases.clone()),
        )?;
        // ---- E7c8: B1/B3/B4 — the minimal deterministic pulse driver ----
        // `handoff.rs` owns this module's ONE `"action"`/`"outcome"` definition
        // validator, so the pulse contributes its four descriptors there and
        // registers its families, its one `DomainClockProvider`, its two action
        // providers and its one `domaintick` schedule source here.
        let control_leases_view = control_leases.clone();
        let tick_definitions =
            handoff::register(reg, control_leases, tick::definition_descriptors()?)?;
        tick::register(reg, tick_definitions)?;
        reg.state(SpatialState::new(self.config.recovery_room));
        let tactics_read = self
            .tactics_movement_reads
            .as_ref()
            .map(|factory| factory(reg))
            .transpose()?;
        let movement_reads = movement_reads::register(reg, tactics_read)?;
        physical::register(reg, self.ic_custody.is_some(), movement_reads.clone())?;
        locomotion::register(reg, movement_reads.clone())?;
        if let Some(reader) = &self.atomic_scene_movement {
            locomotion_scenes::register(reg, reader, movement_reads.clone())?;
        }
        if let Some(factory) = &self.composed_movement {
            locomotion_composed::register(reg, factory, movement_reads.clone())?;
        }
        if let Some((selector, factory)) = &self.window_movement {
            locomotion_window::register(reg, selector.clone(), factory, movement_reads)?;
        }
        // 16b cut T.3 (doc 16 §3.1 "Custody"): the world-side index of units an
        // arena holds. Always composed, even with no game layer above — the
        // admission sites read it unconditionally, and a missing slot would be
        // an admission site that silently admits.
        reg.state(deployed::DeployedUnits::new());
        reg.state(ic_exclusion::IcExclusions::default());
        if let Some(config) = &self.ic_custody {
            config.register(reg)?;
        }
        // E7c5 B1: carry the composition-installed account oracle into world
        // state so command handlers consult it synchronously; unwired = fail
        // closed.
        reg.state(match self.accounts.clone() {
            Some(oracle) => focus::AccountOracle::new(oracle),
            None => focus::AccountOracle::unwired(),
        });
        // E7c7 (cut 6 fold B6): the composition-installed route catalog, on the
        // same shape. Unwired = the manual fixture route, which is cut 6's own
        // behaviour unchanged.
        reg.state(match self.routes.clone() {
            Some(catalog) => route::RouteCatalogState::new(catalog),
            None => route::RouteCatalogState::unwired(),
        });
        // Doc 18 cut IC9 (lead ruling L8): the carrier hooks, on the same
        // shape. Unwired = no manual-departure refusal and the generic aboard
        // lines only, which is the pre-IC9 behaviour.
        reg.state(match self.carrier_hooks.clone() {
            Some(hooks) => journey::CarrierHooksState::new(hooks),
            None => journey::CarrierHooksState::unwired(),
        });
        // E7c5 B2: the world-owned ephemeral focus map — an in-RAM slot, never
        // persisted, never an `Outbox` write, never hydrated at boot. Boot
        // starts empty and a reconnecting player re-claims (D-E7-34).
        reg.state(focus::FocusState::default());
        // E7c5 B6: the RAM mirror of the declared station fallbacks the immediate
        // controller reads on a clean switch. Persisted as `sim.station_fallback`
        // docs; boot re-hydration of the slot is a follow-up (§3.5 as-built).
        reg.state(station::StationFallbacks::default());
        // E7c10 B1: the world-owned mirror of the `control_leases` family the
        // `channel_lease_holder` seam answers from. The family handle is a
        // capability minted only here; the mirror itself is RAM, never
        // persisted, never authority (see `seat.rs`'s head for why the seam is
        // not a direct owner-scoped read).
        reg.state(seat::ControlLeaseView::new(control_leases_view));
        // E7c10 B8: the applied-marker book `sim.station.served` answers from.
        // Persisted as exempt `sim.station_service` docs, hydrated at boot.
        reg.state(seat::ServiceBook::default());
        // E7c6 B2: the carrier-journey book (one `cjourney:<root>` document per
        // carrier) and B5's domain-workflow book (`activation:`/`summary:`).
        // Both are RAM mirrors of exempt `sim.journey` / `sim.domain` records,
        // hydrated at boot.
        reg.state(journey::JourneyBook::default());
        reg.state(domain::ActivationBook::default());
        // E7c8 B1: the live pulse cadence. Defaults to §7's constants; the
        // composition root upgrades it from `[tacsim].ground_pulse_ms` /
        // `space_pulse_ms` after composition (cut 5's `AccountOracle` device).
        reg.state(self.pulse);
        // S4: the room resolver a window's owner registers at its boot.
        reg.state(crate::data::RoomResolver::default());
        // S5: the window geometry the atlas registers, read by propagation.
        reg.state(crate::data::WindowGeometry::default());
        // S7: the domain drivers modules register at their boot.
        reg.state(crate::driver::DomainDrivers::default());
        reg.behavior(domain_spec())?;
        reg.behavior(interior_spec())?;
        reg.behavior(participant_spec())?;
        reg.hook(HookPoint::MoveAttempt, Arc::new(PlacementGate));
        // E7c10 B6: the tree's FIRST `HookPoint::Disconnected` hook — cut 5's
        // deferred unclean-loss PRODUCTION trigger. A disconnect now returns
        // every focus (proxy seat or ordinary billet) to its declared fallback
        // at a safe boundary, on the same path a clean release uses.
        reg.hook(HookPoint::Disconnected, Arc::new(seat::SeatDisconnectHook));
        reg.hook(HookPoint::CarryAttempt, Arc::new(PlacementGate));
        // 16b cut S.2: doc 16 §3.6's world-side exclusion on the `look` surface —
        // see `PlacementGate::perceives`.
        reg.hook(HookPoint::Perceive, Arc::new(PlacementGate));
        reg.destroy_gate(Arc::new(StructuralDestroyGate));
        reg.detach_gate(Arc::new(SimDetachGate));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        // E7c6 B1+B2+B3: ONE `Coarse` tick advances every carrier journey on the
        // world's IC clock — the housing assessment tick's "pull, not a
        // subscription" shape. It is the whole driver: the leg cursor makes it
        // idempotent, so its re-run at boot IS the crash-gap heal and there is no
        // second reconcile pass to disagree with it.
        reg.tick(
            journey::TICK_TAG,
            TickCadence::Coarse(Duration::from_secs(journey::TICK_SECS)),
            Arc::new(journey::tick),
        )?;
        // E7c8 B1/B6: the world-side applying authority. A `schedule_source`-fired
        // action has no `&World`, no `Ctx` and no `Outbox` (10b §4.0d), so the
        // half of the pulse that reads cut 6's activation book, writes the `sim`
        // domain document and arms the durable cursor is a Coarse tick — the same
        // split RESOLUTION A gave cuts 6 and 7. The DETERMINISTIC half is the
        // prepared authoritative commit the `domaintick` source fires.
        reg.tick(
            tick::PULSE_TICK_TAG,
            TickCadence::Coarse(Duration::from_secs(tick::PULSE_TICK_SECS)),
            Arc::new(tick::maintenance_tick),
        )?;
        commands::register(reg)?;
        relay::register(reg)?;
        Ok(())
    }

    fn audit_typed_history(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.audit_movement_history(ctx)
    }

    fn audit_boot_history(
        &self,
        ctx: &ModuleCtx<'_>,
        data: &ModuleBootData,
    ) -> Result<(), ModError> {
        if self.window_movement.is_some() {
            locomotion_window::audit_native(ctx, data)
        } else {
            self.audit_native_movement_history(data)
        }
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        mut data: ModuleBootData,
    ) -> Result<(), ModError> {
        // A missing/mismatched receipt pair must refuse before Sim's legacy
        // loaders can quarantine, reconcile projections, or advance journeys.
        // The server hydrates the typed-state partition before any load hook.
        self.audit_movement_history(ctx)?;
        self.audit_boot_history(ctx, &data)?;
        ic_exclusion::load(ctx, &mut data, self.ic_custody.is_some())?;
        if let Some(config) = &self.ic_custody {
            config.loaded(ctx)?;
        }
        relay::load(ctx, &mut data)?;
        let carriers = journey::extract_carrier_journeys(&mut data);
        ic_exclusion::check_boot_journeys(ctx.w, &carriers)?;
        let workflow = domain::extract_domain_workflow(&mut data);
        let q = boot::load_boot(ctx, data, self.config.recovery_room)?;
        domain::install_domain_workflow(ctx, workflow);
        // Installing the carrier journeys runs the same idempotent advance the
        // tick runs (B3): an arrival already applied is a no-op, and one whose
        // durable cursor says it arrived but whose carrier is not there is
        // repaired here.
        journey::install_carrier_journeys(ctx, carriers);
        if q > 0 {
            tracing::warn!(
                quarantined = q,
                "sim: boot quarantined placements; see @spatial/quarantine"
            );
        }
        Ok(())
    }

    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        // Retain the final check for drivers invoking final composition
        // directly and for changes made by intervening module load hooks.
        self.audit_movement_history(ctx)?;
        // The coordinator hydrated typed leases before the load callbacks.
        // Read them through Sim's real owner lens before the first command:
        // a restarted pilot may reclaim before any periodic tick has fired.
        // The existing bounded scan keeps an unavailable or partial view
        // incomplete, so the seat gate still refuses unknown occupancy.
        seat::refresh_control_lease_view(ctx);
        Ok(())
    }
}

pub use data::ArcFn;
pub use driver::{
    DomainDriver, DomainDrivers, MAX_DOMAIN_DRIVERS, driver_named, driver_names, driver_of,
    register_domain_driver,
};
