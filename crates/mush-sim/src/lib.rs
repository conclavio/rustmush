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

pub mod boot;
pub mod commands;
pub mod data;
pub mod graph;
pub mod transition;

use std::sync::Arc;

use mush_core::{
    BehaviorId, BehaviorSpec, Ctx, DestroyGate, DestroyPolicy, DetachGate, EventHook, GameModule,
    HookPoint, ModError, ModuleBootData, ModuleCtx, ObjId, ObjKind, Registrar, Registry, Role,
    World, WorldEvent, WorldEventKind,
};
use mush_proto::RichText;

pub use data::{
    CapacitySpec, DEFAULT_RECOVERY_ROOM, Derived, DomainClock, DomainId, DomainRecord, DomainStatus,
    EgressSpec, HostMounts, InteriorSpaceRecord, MAX_SPATIAL_DEPTH, MountId, MountSpec, NS,
    PASSIVE_DRIVER, PlacementRecord, Presence, RulesValue, SlotId, SpaceAnchor, SpatialParent,
    SpatialState, UnplacedInfo,
};
pub use graph::{effective_domain, presence, project, verify_indexes, verify_projections};
pub use transition::{Plan, SpatialError, Transition, apply, plan, run};

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
#[derive(Debug, Default)]
pub struct SimulationModule {
    config: SimConfig,
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
        SimulationModule { config }
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
    s.help = "Hosts a simulation domain (@domain/create). Refuses @destroy; retire the domain first.".into();
    s
}

fn interior_spec() -> BehaviorSpec {
    let mut s = BehaviorSpec::new(bid("sim.interior"));
    s.allowed_kinds = vec![ObjKind::Room];
    s.destroy = DestroyPolicy::Refuse;
    s.min_role = Role::Wizard;
    s.help = "An interior space (a hangar, a bay) that can hold placed units (@spatial/space).".into();
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

fn placed_or_quarantined(w: &World, obj: ObjId) -> Option<String> {
    let st = state(w)?;
    if st.is_quarantined(obj) {
        return Some(format!(
            "{} is quarantined; staff recover it with @spatial/recover.",
            mush_core::output::display_name(w, obj)
        ));
    }
    match st.parent_of(obj) {
        SpatialParent::Direct { domain, .. } => Some(format!(
            "{} is placed in domain #{domain}; it moves only through the simulation, or staff recovery (@spatial).",
            mush_core::output::display_name(w, obj)
        )),
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
    fn move_attempt(&self, w: &mut World, actor: ObjId, _exit: Option<ObjId>, _dest: ObjId) -> Result<(), RichText> {
        match placed_or_quarantined(w, actor) {
            Some(reason) => Err(RichText::tag("error", reason)),
            None => Ok(()),
        }
    }
    fn carry_attempt(&self, w: &mut World, _mover: ObjId, obj: ObjId, _dest: ObjId) -> Result<(), RichText> {
        match placed_or_quarantined(w, obj) {
            Some(reason) => Err(RichText::tag("error", reason)),
            None => Ok(()),
        }
    }
}

/// The structural destroy veto (design D4/D8), independent of bindings.
struct StructuralDestroyGate;

impl DestroyGate for StructuralDestroyGate {
    fn check(&self, w: &World, _reg: &Registry, _actor: ObjId, target: ObjId) -> Result<(), String> {
        let Some(st) = state(w) else { return Ok(()) };
        if target == st.recovery_room && target != mush_core::LIMBO {
            return Err(format!("{} is the spatial recovery room; reconfigure it first.", name(w, target)));
        }
        if let Some(d) = st.domain_of_host(target) {
            return Err(format!("{} hosts domain #{}; retire it first (@domain/retire).", name(w, target), d.id));
        }
        if let Some(ds) = st.derived.presentation_of.get(&target) {
            let live: Vec<String> = ds.iter().filter(|d| st.domains.get(d).is_some_and(|r| r.status != DomainStatus::Retired)).map(|d| format!("#{d}")).collect();
            if !live.is_empty() {
                return Err(format!("{} is the presentation room of domain(s) {}; re-point them first (@domain/present).", name(w, target), live.join(", ")));
            }
        }
        if st.spaces.contains_key(&target) {
            return Err(format!("{} is an interior space; @spatial/unspace it first.", name(w, target)));
        }
        if let Some(spaces) = st.derived.anchored_spaces.get(&target) {
            let occupied: Vec<String> = spaces.iter().filter(|s| !st.children_of(**s).is_empty()).map(|s| name(w, *s)).collect();
            if !occupied.is_empty() {
                return Err(format!("{} carries occupied interior(s) {}; empty them first.", name(w, target), occupied.join(", ")));
            }
        }
        let deps = st.children_of(target).len();
        if deps > 0 {
            return Err(format!("{} still has {deps} placed dependent(s); move them first.", name(w, target)));
        }
        Ok(())
    }
}

/// The detach veto: a `sim.*` binding cannot be stripped while records
/// depend on it.
struct SimDetachGate;

impl DetachGate for SimDetachGate {
    fn check(&self, w: &World, _reg: &Registry, _actor: ObjId, obj: ObjId, behavior: &BehaviorId) -> Result<(), String> {
        let Some(st) = state(w) else { return Ok(()) };
        match behavior.as_str() {
            "sim.domain" => {
                if let Some(d) = st.domain_of_host(obj) {
                    return Err(format!("{} still hosts domain #{}; @domain/retire it first.", name(w, obj), d.id));
                }
            }
            "sim.interior" => {
                if st.spaces.contains_key(&obj) {
                    return Err(format!("{} is still an interior space; @spatial/unspace it first.", name(w, obj)));
                }
            }
            "sim.participant" if st.is_quarantined(obj) => {
                return Err(format!("{} is quarantined; @spatial/recover it first.", name(w, obj)));
            }
            "sim.participant" if st.is_placed(obj) => {
                return Err(format!("{} is still placed; @spatial/unplace it first.", name(w, obj)));
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
    let WorldEvent::ObjDestroyed { obj } = ev else { return };
    let obj = *obj;
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
    let riding_spaces: Vec<ObjId> = st.derived.anchored_spaces.get(&obj).map(|s| s.iter().copied().collect()).unwrap_or_default();
    let dependents: Vec<ObjId> = st.children_of(obj).iter().copied().collect();
    let was_placed = st.placements.contains_key(&obj);
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
        .flat_map(|d| st.derived.domain_direct.get(d).into_iter().flatten().copied())
        .collect();

    // Quarantine dependents (occupants/attachments) and direct participants of
    // a lost domain; interior/attached records under OTHER hosts stay intact.
    for root in dependents
        .iter()
        .chain(direct_in_hosted.iter())
        .chain(directs_of_orphaned.iter())
    {
        let _ = run(ctx, &Transition::Quarantine { root: *root, reason: format!("{obj} was destroyed") });
    }
    // Spaces fixed in the lost domain, or riding the husk, become unanchored.
    for sp in fixed_spaces.iter().chain(riding_spaces.iter()) {
        let _ = run(ctx, &Transition::Space { space: *sp, anchor: SpaceAnchor::Unanchored });
    }
    // A domain whose presentation room died is paused: it cannot project.
    for d in &orphaned_domains {
        let paused = ctx.w.module_state_mut::<SpatialState>().and_then(|s| {
            let rec = s.domains.get_mut(d)?;
            (rec.status == DomainStatus::Active).then(|| {
                rec.status = DomainStatus::Paused;
                rec.revision += 1;
                rec.clone()
            })
        });
        if let Some(rec) = paused {
            data::persist_domain(ctx.out, &rec);
        }
    }
    // Retire the records the husk owned.
    let s = ctx.w.module_state_mut::<SpatialState>().expect("sim state slot installed at registration");
    let mut dels: Vec<smol_str::SmolStr> = Vec::new();
    if let Some(d) = hosted
        && let Some(rec) = s.domains.get_mut(&d)
    {
        rec.status = DomainStatus::Retired;
        rec.revision += 1;
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
    s.derived = graph::rebuild(s);
    for k in dels {
        data::persist_del(ctx.out, k);
    }
}

impl GameModule for SimulationModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(SpatialState::new(self.config.recovery_room));
        reg.behavior(domain_spec())?;
        reg.behavior(interior_spec())?;
        reg.behavior(participant_spec())?;
        reg.hook(HookPoint::MoveAttempt, Arc::new(PlacementGate));
        reg.hook(HookPoint::CarryAttempt, Arc::new(PlacementGate));
        reg.destroy_gate(Arc::new(StructuralDestroyGate));
        reg.detach_gate(Arc::new(SimDetachGate));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(&mut self, ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
        let q = boot::load_boot(ctx, data, self.config.recovery_room)?;
        if q > 0 {
            tracing::warn!(quarantined = q, "sim: boot quarantined placements; see @spatial/quarantine");
        }
        Ok(())
    }
}
