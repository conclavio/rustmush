//! # The tactics module (M33) — attach + persist + read
//!
//! The cut where M31 `MobilityProfile` and M32 `TacticalUnit` (pure libraries)
//! become LIVE: attached to real objects, persisted, surviving reboot, and
//! finally READ (a show command, and a panel that consumes the M32 damage
//! [`Journal`](crate::unit::Journal)). This module owns NS `tactics`; the
//! seam-test stub moved to `tacseam` so both can load in one Engine.
//!
//! ## Contracts (from the M33 design review)
//! - **The in-RAM slot [`UnitStore`] is the single source of truth.** Every
//!   mutation reads the slot, replaces the entry with the returned unit, THEN
//!   persists the whole `unit:<obj>` doc from that entry — never a detached
//!   clone (monolithic doc: a whole-unit write clobbers the same key).
//! - **Two orthogonal states on one object.** A tactical unit's section tree is
//!   entirely in mush-tactics; the same object may independently be a mush-sim
//!   placed participant, a journey traveler, or neither. Tactics boot consults
//!   ONLY `w.get(obj)` (never `SpatialState`) — create-eligibility and
//!   boot-eligibility use the same World-only predicate (the M30 lesson).
//! - **The `tactics.unit` binding ↔ the `unit:<obj>` doc are reconciled both
//!   ways at boot** (bound husk with no doc → detach; doc with no binding →
//!   re-attach), the symmetric invariant M30 hardening baked in.
//! - **Panel: staff/self only, always [`Audience::Actor`].** No crew fan-out and
//!   no `SpatialState` read; every payload routes through [`project`], whose M33
//!   body ignores the observer and returns full state, so the epistemic
//!   projector later swaps the body without touching emit topology.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind};
use mush_core::output::display_name;
use mush_core::{
    ArgShape, Audience, BehaviorBinding, BehaviorId, BehaviorSpec, CmdError, CommandSpec, Ctx, Deferred,
    DetachGate, Flags, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, PanelFeedHandle, Powers, Registry,
    Role, World, match_object, perm,
};
use smol_str::SmolStr;

use crate::unit::{self, TacticalUnit, UnitCatalog, UnitMutationOp, UnitMutationPlan};

/// The module id and persistence namespace.
pub const NS: &str = "tactics";
/// The marker behavior: an object carrying it IS a tactical unit.
pub const UNIT_BEHAVIOR: &str = "tactics.unit";
/// The panel feed id (the reserved `tactics.unit` view).
pub const UNIT_FEED: &str = "tactics.unit";

/// The in-RAM unit slot — the single source of truth; each unit rides a durable
/// `unit:<obj>` doc.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitStore {
    /// Live tactical units by object.
    pub units: BTreeMap<ObjId, TacticalUnit>,
}

/// The tactics [`GameModule`]. Construct with a frozen [`UnitCatalog`] (composed
/// at wiring time; empty by default — the engine seeds no concrete spec).
#[derive(Debug, Default)]
pub struct TacticsModule {
    catalog: Arc<UnitCatalog>,
}

impl TacticsModule {
    /// A module with an empty catalog (no unit schemas registered).
    #[must_use]
    pub fn new() -> TacticsModule {
        TacticsModule { catalog: Arc::new(UnitCatalog::default()) }
    }
    /// A module with a frozen catalog (a ruleset / dev wiring supplies it).
    #[must_use]
    pub fn with_catalog(catalog: UnitCatalog) -> TacticsModule {
        TacticsModule { catalog: Arc::new(catalog) }
    }
}

impl GameModule for TacticsModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(UnitStore::default());

        let mut spec = BehaviorSpec::new(BehaviorId::new(UNIT_BEHAVIOR).unwrap());
        spec.allowed_kinds = vec![ObjKind::Thing, ObjKind::Player, ObjKind::Npc];
        reg.behavior(spec)?;
        reg.detach_gate(Arc::new(TacticsDetachGate));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));

        let feed = reg.panel_feed(UNIT_FEED)?;
        let catalog = self.catalog.clone();
        reg.command(
            CommandSpec {
                name: "@unit".into(),
                shape: ArgShape::EqPair,
                switches: ["create", "show", "damage", "remove"].iter().copied().map(SmolStr::new).collect(),
                help: "@unit/create <obj>=<schema-id>   make an object a tactical unit\n\
                       @unit/show <obj>                 inspect it\n\
                       @unit/damage <obj>={json plan}   (staff/debug) apply a UnitMutationPlan\n\
                       @unit/remove <obj>               strip the tactical unit"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| cmd_unit(&catalog, &feed, ctx)),
        )?;
        Ok(())
    }

    fn on_world_loaded(&mut self, ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
        boot_units(ctx, data);
        Ok(())
    }
}

// ---- persistence ------------------------------------------------------

fn unit_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("unit:{}", obj.0))
}
fn persist_unit(out: &mut Outbox, unit: &TacticalUnit) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet { key: unit_key(unit.object()), value: serde_json::to_value(unit).expect("unit serializes") },
    }));
}
fn persist_unit_del(out: &mut Outbox, obj: ObjId) {
    persist_key_del(out, unit_key(obj));
}
/// Delete a RAW doc key — used at boot to purge the *actual* offending doc (a
/// corrupt or mis-keyed one whose key must NOT be reconstructed from a
/// possibly-wrong embedded/parsed id), mirroring route.rs `persist_key_del`.
fn persist_key_del(out: &mut Outbox, key: SmolStr) {
    out.defer(Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op: ModuleWrite::Del { key } }));
}

// ---- the panel projection (epistemic seam) ----------------------------

/// Project `unit` for `observer` at `now` into a panel payload. The M33 body
/// ignores `observer`/`now` and returns full state; the future epistemic
/// projector swaps this body per-observer without touching emit topology.
fn project(unit: &TacticalUnit, _observer: ObjId, _now: i64) -> serde_json::Value {
    serde_json::to_value(unit).unwrap_or(serde_json::Value::Null)
}

// ---- lifecycle --------------------------------------------------------

/// A destroyed unit-object drops its unit + doc — **and** every OTHER unit that
/// pointed a component at the husk as its economic asset drops that pointer
/// (E3c, doc 08b §5.2).
///
/// The M32/M33 deferral is over: `mush-economy` now titles objects, so a
/// component whose `asset` names a dead `ObjId` is a dangling reference into a
/// namespace that will never resolve again. Nulling goes through a real
/// [`UnitMutationPlan`], so the unit's revision bumps and a queued order that
/// captured `(subject, gen)` sees the change; the repaired doc rides the same
/// outbox as everything else this event produced.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else { return };
    let obj = *obj;
    let removed = ctx.w.module_state_mut::<UnitStore>().is_some_and(|s| s.units.remove(&obj).is_some());
    if removed {
        persist_unit_del(ctx.out, obj);
    }
    let now = ctx.w.now();
    let repaired = {
        let Some(store) = ctx.w.module_state::<UnitStore>() else { return };
        scrub_component_assets(&store.units, now, &|a| a == obj)
    };
    for u in &repaired {
        tracing::warn!(unit = %u.object(), dead = %obj, "tactics: component asset pointer scrubbed (object destroyed)");
        persist_unit(ctx.out, u);
    }
    if let Some(store) = ctx.w.module_state_mut::<UnitStore>() {
        for u in repaired {
            store.units.insert(u.object(), u);
        }
    }
}

/// The one scrub: for every unit holding a component whose `asset` satisfies
/// `dead`, null those pointers through a single [`UnitMutationPlan`] and return
/// the repaired units (the caller persists and installs them).
///
/// Pure with respect to the store — it clones and folds, so a plan the engine
/// refuses (it cannot, but the type says it may) leaves the live unit exactly as
/// it was, pointer and all, with a loud log instead of a half-edit.
fn scrub_component_assets(
    units: &BTreeMap<ObjId, TacticalUnit>,
    now: i64,
    dead: &dyn Fn(ObjId) -> bool,
) -> Vec<TacticalUnit> {
    let mut repaired = Vec::new();
    for (id, unit) in units {
        let ops: Vec<UnitMutationOp> = unit
            .components()
            .filter(|(_, c)| c.asset.is_some_and(dead))
            .map(|(cid, _)| UnitMutationOp::SetComponentAsset { component: cid.clone(), asset: None })
            .collect();
        if ops.is_empty() {
            continue;
        }
        let plan = UnitMutationPlan {
            ops,
            expected_revision: None,
            cause: Some(SmolStr::new_static("asset-scrub")),
        };
        match unit::apply(unit, &plan, now) {
            Ok((mut post, _journal)) => {
                post.settle_mobility(now);
                repaired.push(post);
            }
            Err(e) => {
                tracing::error!(unit = %id, error = %e, "tactics: component asset scrub refused; pointer left dangling");
            }
        }
    }
    repaired
}

/// Refuse `@behavior/remove tactics.unit` while the object is a live unit —
/// `@unit/remove` is the sanctioned teardown (which detaches programmatically).
struct TacticsDetachGate;
impl DetachGate for TacticsDetachGate {
    fn check(&self, w: &World, _reg: &Registry, _actor: ObjId, obj: ObjId, behavior: &BehaviorId) -> Result<(), String> {
        if behavior.as_str() == UNIT_BEHAVIOR
            && w.module_state::<UnitStore>().is_some_and(|s| s.units.contains_key(&obj))
        {
            return Err("That object is a live tactical unit; @unit/remove it first.".into());
        }
        Ok(())
    }
}

/// Rebuild the unit slot from `unit:<obj>` docs and reconcile the binding both
/// ways. World-only eligibility, mirroring route.rs `install_journeys`.
fn boot_units(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) {
    let unit_behavior = BehaviorId::new(UNIT_BEHAVIOR).unwrap();
    let mut store = UnitStore::default();

    for (key, value) in data.kv {
        let Some(rest) = key.strip_prefix("unit:") else {
            tracing::warn!(%key, "tactics: unknown doc ignored");
            continue;
        };
        let unit: TacticalUnit = match serde_json::from_value(value) {
            Ok(u) => u,
            Err(e) => {
                // Delete the ACTUAL key — the suffix may be non-numeric.
                tracing::error!(%key, error = %e, "tactics: unparseable unit doc deleted");
                persist_key_del(ctx.out, key.clone());
                continue;
            }
        };
        let obj = unit.object();
        // Key ↔ embedded-id consistency (parity with sim boot). Delete the doc's
        // OWN key — never the embedded id, which is precisely what is in question.
        if rest.parse::<u32>() != Ok(obj.0) {
            tracing::error!(%key, %obj, "tactics: unit doc key/object mismatch deleted");
            persist_key_del(ctx.out, key.clone());
            continue;
        }
        // Object gone during downtime → tombstone the doc (route's traveler-gone path).
        if ctx.w.get(obj).is_none_or(|o| o.flags.contains(Flags::GOING)) {
            tracing::warn!(%obj, "tactics: unit doc for a missing object deleted");
            persist_key_del(ctx.out, key.clone());
            continue;
        }
        // Doc-with-no-binding → re-attach the marker (the doc is the truth).
        if !ctx.w.has_behavior(obj, &unit_behavior) {
            let _ = ctx.w.attach_behavior(
                obj,
                unit_behavior.clone(),
                BehaviorBinding { schema_version: 1, attached_by: mush_core::GOD, attached_at: ctx.w.now() },
            );
        }
        store.units.insert(obj, unit);
    }

    // Binding-with-no-doc → a stale marker: detach it (programmatic, bypasses
    // the DetachGate). Bounded one-time scan over the world at boot.
    let stale: Vec<ObjId> = ctx
        .w
        .iter()
        .filter(|o| o.behaviors.contains_key(&unit_behavior) && !store.units.contains_key(&o.id))
        .map(|o| o.id)
        .collect();
    for obj in stale {
        tracing::warn!(%obj, "tactics: tactics.unit binding with no unit doc; detaching");
        let _ = ctx.w.detach_behavior(obj, &unit_behavior);
    }

    // E3c: the boot half of the component-asset scrub. Only `@destroy` emits
    // `ObjDestroyed`, so a programmatic `World::destroy` (or an object lost
    // during downtime) leaves a component pointing at nothing; this is the pass
    // that catches it. World-only eligibility, like everything else here — the
    // question is "does that object still exist?", never "does it have a title?"
    // (`mush-tactics` takes no economy dependency).
    let now = ctx.w.now();
    let repaired = {
        let w: &World = ctx.w;
        scrub_component_assets(&store.units, now, &|a| {
            w.get(a).is_none_or(|o| o.flags.contains(Flags::GOING))
        })
    };
    for u in repaired {
        tracing::warn!(unit = %u.object(), "tactics: component asset pointer(s) to a missing object scrubbed at boot");
        persist_unit(ctx.out, &u);
        store.units.insert(u.object(), u);
    }

    if let Some(slot) = ctx.w.module_state_mut::<UnitStore>() {
        *slot = store;
    }
}

// ---- commands ---------------------------------------------------------

fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}
fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) { Ok(()) } else { Err(CmdError::msg("Permission denied.")) }
}
fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tactics", msg.into());
}
fn rhs_required(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
    ctx.args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg(format!("Usage: {usage}")))
}

fn cmd_unit(catalog: &UnitCatalog, feed: &PanelFeedHandle, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        unit_create(catalog, feed, ctx)
    } else if ctx.switches.has("show") {
        unit_show(ctx)
    } else if ctx.switches.has("damage") {
        unit_damage(feed, ctx)
    } else if ctx.switches.has("remove") {
        unit_remove(ctx)
    } else {
        Err(CmdError::msg("Usage: @unit/create|show|damage|remove <obj>[=...]"))
    }
}

/// Why an object cannot become a tactical unit (parity with route_ineligible).
fn unit_ineligible(ctx: &Ctx<'_>, obj: ObjId) -> Option<String> {
    match ctx.w.get(obj) {
        None => return Some("No such object.".into()),
        Some(o) if o.flags.contains(Flags::GOING) => return Some("It is being destroyed.".into()),
        Some(o) if matches!(o.kind, ObjKind::Room | ObjKind::Exit) => {
            return Some(format!("{} cannot be a tactical unit.", display_name(ctx.w, obj)));
        }
        _ => {}
    }
    ctx.w
        .module_state::<UnitStore>()
        .is_some_and(|s| s.units.contains_key(&obj))
        .then(|| format!("{} is already a tactical unit (@unit/remove first).", display_name(ctx.w, obj)))
}

fn unit_create(catalog: &UnitCatalog, feed: &PanelFeedHandle, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let def_id = rhs_required(ctx, "@unit/create <obj>=<schema-id>")?;
    if let Some(why) = unit_ineligible(ctx, obj) {
        return Err(CmdError::msg(why));
    }
    let spec = catalog
        .get(def_id.trim())
        .ok_or_else(|| CmdError::msg(format!("No unit schema `{}` in the catalog.", def_id.trim())))?;
    let now = ctx.w.now();
    let mut u = TacticalUnit::from_spec(spec, obj).map_err(|e| CmdError::msg(e.to_string()))?;
    u.settle_mobility(now);
    ctx.w
        .attach_behavior(obj, BehaviorId::new(UNIT_BEHAVIOR).unwrap(), BehaviorBinding { schema_version: 1, attached_by: ctx.actor, attached_at: now })
        .map_err(|e| CmdError::msg(format!("cannot mark unit: {e}")))?;
    let frame = project(&u, ctx.actor, now);
    persist_unit(ctx.out, &u);
    ctx.w.module_state_mut::<UnitStore>().expect("unit store").units.insert(obj, u);
    feed.keyframe_to(ctx, Audience::Actor(ctx.actor), frame);
    info(ctx, format!("{} is now a tactical unit ({}).", display_name(ctx.w, obj), def_id.trim()));
    Ok(())
}

fn unit_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let store = ctx.w.module_state::<UnitStore>().expect("unit store");
        let u = store.units.get(&obj).ok_or_else(|| CmdError::msg("It is not a tactical unit."))?;
        let mut v = vec![format!(
            "{} — schema {} v{}, revision {}, mobility {}",
            display_name(ctx.w, obj),
            u.schema().id,
            u.schema().version,
            u.revision(),
            u.mobility()
                .and_then(|m| m.effective_mode(ctx.w.now()).map(|id| id.to_string()))
                .unwrap_or_else(|| "none".into()),
        )];
        for (id, s) in u.sections() {
            v.push(format!("  section {id}: {}", s.condition));
        }
        for (addr, cell) in u.cells() {
            v.push(format!("  cell {addr:?}: {}/{}", cell.current, cell.max));
        }
        for (id, c) in u.components() {
            v.push(format!("  component {id}: {} ({} v{})", c.condition, c.definition.id, c.definition.version));
        }
        for k in u.capabilities() {
            v.push(format!("  capability {} (source {:?})", k.id, k.source));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn unit_damage(feed: &PanelFeedHandle, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let json = rhs_required(ctx, "@unit/damage <obj>={json UnitMutationPlan}")?;
    let plan: UnitMutationPlan = serde_json::from_str(json.trim()).map_err(|e| CmdError::msg(format!("bad plan JSON: {e}")))?;
    // Safe subset: GrantCapability mints an unreconciled opaque capability (no
    // projector yet to check it) — refuse it via this staff-debug path.
    if plan.ops.iter().any(|op| matches!(op, UnitMutationOp::GrantCapability { .. })) {
        return Err(CmdError::msg("GrantCapability is not allowed via @unit/damage (needs a ruleset projector)."));
    }
    let now = ctx.w.now();
    let unit = ctx
        .w
        .module_state::<UnitStore>()
        .and_then(|s| s.units.get(&obj).cloned())
        .ok_or_else(|| CmdError::msg("It is not a tactical unit."))?;
    let (mut post, journal) = unit::apply(&unit, &plan, now).map_err(|e| CmdError::msg(e.to_string()))?;
    post.settle_mobility(now);
    let (from, to, n) = (journal.from_revision, journal.to_revision, journal.entries.len());
    let delta = serde_json::to_value(&journal).expect("journal serializes");
    persist_unit(ctx.out, &post);
    ctx.w.module_state_mut::<UnitStore>().expect("unit store").units.insert(obj, post);
    feed.delta_to(ctx, Audience::Actor(ctx.actor), delta);
    info(ctx, format!("Applied {n} op(s) to {} (revision {from} -> {to}).", display_name(ctx.w, obj)));
    Ok(())
}

fn unit_remove(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let removed = ctx.w.module_state_mut::<UnitStore>().expect("unit store").units.remove(&obj).is_some();
    if !removed {
        return Err(CmdError::msg("It is not a tactical unit."));
    }
    persist_unit_del(ctx.out, obj);
    // Programmatic detach bypasses the DetachGate (the store entry is already gone).
    let _ = ctx.w.detach_behavior(obj, &BehaviorId::new(UNIT_BEHAVIOR).unwrap());
    info(ctx, format!("{} is no longer a tactical unit.", display_name(ctx.w, obj)));
    Ok(())
}
