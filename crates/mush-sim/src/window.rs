//! Windows on domains (doc 14 §2.4 and §3.4; plan §3.5): the sim's side of a
//! tactical window — the domain a window opens on, the metric placements inside
//! it, and their release. The atlas owns the composite and the resolver; this
//! module owns the records and the transitions, so the placement graph stays the
//! one truth and every pose change is a planned, audited transition.

use mush_core::{BINDING_SCHEMA_VERSION, BehaviorBinding, BehaviorId, Ctx, ObjId, World};

use crate::data::{
    DomainClock, DomainId, DomainRecord, DomainStatus, GeometryFn, MetricGeometry, Pose, ResolveFn,
    RoomResolver, SpatialParent, SpatialState, WindowGeometry, WindowRec, passive_definition_ref,
    persist_domain, persist_meta,
};
use crate::graph::rebuild;
use crate::transition::{Transition, run};

/// The most windows open at once (doc 14 §3.4: `MAX_ACTIVE_DOMAINS` bounds the
/// total).
pub const MAX_WINDOWS: usize = crate::domain::MAX_ACTIVE_DOMAINS;

/// What opening a window needs.
#[derive(Clone, Debug)]
pub struct OpenWindow {
    /// The object that hosts the domain: the first anchor room.
    pub host: ObjId,
    /// Where a participant off every footprint projects to.
    pub presentation: ObjId,
    /// The frame.
    pub rec: WindowRec,
}

fn state(w: &World) -> Result<&SpatialState, String> {
    w.module_state::<SpatialState>()
        .ok_or_else(|| "the simulation module is not composed".to_string())
}

fn state_mut(w: &mut World) -> Result<&mut SpatialState, String> {
    w.module_state_mut::<SpatialState>()
        .ok_or_else(|| "the simulation module is not composed".to_string())
}

/// Bind `behavior` to `obj` if it is not bound already: the window's ground
/// hosts a domain and its participants are spatial participants by the fact of
/// standing on the map, not by a staffer's `@behavior/add`.
fn attach(ctx: &mut Ctx<'_>, obj: ObjId, behavior: &str) -> Result<(), String> {
    let id = BehaviorId::new(behavior).map_err(|e| e.to_string())?;
    if ctx.w.has_behavior(obj, &id) {
        return Ok(());
    }
    let binding = BehaviorBinding {
        schema_version: BINDING_SCHEMA_VERSION,
        attached_by: ctx.actor,
        attached_at: ctx.w.now(),
    };
    ctx.w
        .attach_behavior(obj, id, binding)
        .map_err(|e| e.to_string())
}

/// Register the room resolver (doc 14 §3.4). Once per composition.
///
/// # Errors
///
/// A message when the simulation module is not composed or a resolver is
/// registered already.
pub fn register_room_resolver(w: &mut World, f: ResolveFn) -> Result<(), String> {
    let slot = w
        .module_state_mut::<RoomResolver>()
        .ok_or_else(|| "the simulation module is not composed".to_string())?;
    if slot.resolve.is_some() {
        return Err("a room resolver is registered already".to_string());
    }
    slot.resolve = Some(f);
    Ok(())
}

/// Register the window geometry (doc 14 §5.2; S5). Once per composition.
///
/// # Errors
///
/// A message when the simulation module is not composed or a geometry is
/// registered already.
pub fn register_window_geometry(w: &mut World, f: GeometryFn) -> Result<(), String> {
    let slot = w
        .module_state_mut::<WindowGeometry>()
        .ok_or_else(|| "the simulation module is not composed".to_string())?;
    if slot.geometry.is_some() {
        return Err("a window geometry is registered already".to_string());
    }
    slot.geometry = Some(f);
    Ok(())
}

/// The metric between `near` and `far` when both stand on one window, from
/// the registered geometry.
#[must_use]
pub fn metric_between(w: &World, near: ObjId, far: ObjId) -> Option<MetricGeometry> {
    let f = w.module_state::<WindowGeometry>()?.geometry.clone()?;
    f(w, near, far)
}

/// Open a window: a passive domain on `host` presenting in `presentation`,
/// carrying `rec`. The host gains `sim.domain` if it lacks it.
///
/// # Errors
///
/// A message when the host already hosts a live domain, the presentation room
/// is not a live room, [`MAX_WINDOWS`] are open, or the allocator is spent.
pub fn open_window(ctx: &mut Ctx<'_>, open: OpenWindow) -> Result<DomainId, String> {
    {
        let st = state(ctx.w)?;
        if let Some(d) = st.domain_of_host(open.host) {
            return Err(format!(
                "{} already hosts domain #{}",
                mush_core::output::display_name(ctx.w, open.host),
                d.id
            ));
        }
        if windows(ctx.w).count() >= MAX_WINDOWS {
            return Err(format!("{MAX_WINDOWS} windows are open already"));
        }
    }
    let live_room = ctx.w.get(open.presentation).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING)
    });
    if !live_room {
        return Err("a window presents in a live room".to_string());
    }
    attach(ctx, open.host, "sim.domain")?;
    let (rec, next) = {
        let s = state_mut(ctx.w)?;
        let id = s.alloc_domain_id().map_err(|e| e.to_string())?;
        let rec = DomainRecord {
            id,
            host: open.host,
            driver: passive_definition_ref(),
            status: DomainStatus::Active,
            clock: DomainClock::Passive,
            presentation_location: open.presentation,
            revision: 1,
            window: Some(open.rec),
        };
        s.domains.insert(id, rec.clone());
        s.derived = rebuild(s);
        (rec, s.next_domain_id)
    };
    persist_domain(ctx.out, &rec);
    persist_meta(ctx.out, next);
    Ok(rec.id)
}

/// Replace a window's frame (a re-composite, growth): the domain's revision
/// moves with it.
///
/// # Errors
///
/// A message when the domain is not a live window.
pub fn update_window(ctx: &mut Ctx<'_>, domain: DomainId, rec: WindowRec) -> Result<(), String> {
    let updated = {
        let s = state_mut(ctx.w)?;
        let d = s
            .domains
            .get_mut(&domain)
            .filter(|d| d.status == DomainStatus::Active && d.window.is_some())
            .ok_or_else(|| format!("domain #{domain} has no open window"))?;
        d.window = Some(rec);
        d.revision = d.revision.saturating_add(1);
        d.clone()
    };
    persist_domain(ctx.out, &updated);
    Ok(())
}

/// The window on `domain`, when it is open.
#[must_use]
pub fn window_of(w: &World, domain: DomainId) -> Option<&WindowRec> {
    w.module_state::<SpatialState>()?
        .domains
        .get(&domain)
        .filter(|d| d.status == DomainStatus::Active)?
        .window
        .as_ref()
}

/// Every open window, by domain.
pub fn windows(w: &World) -> impl Iterator<Item = (DomainId, &WindowRec)> + '_ {
    w.module_state::<SpatialState>()
        .into_iter()
        .flat_map(|st| st.domains.values())
        .filter(|d| d.status == DomainStatus::Active)
        .filter_map(|d| d.window.as_ref().map(|rec| (d.id, rec)))
}

/// Place (or re-pose) `obj` in `domain` at `pose`: one planned transition, the
/// projection to the resolved room included. The object gains
/// `sim.participant` if it lacks it.
///
/// # Errors
///
/// The transition's refusal, or the pose's.
pub fn place(ctx: &mut Ctx<'_>, obj: ObjId, domain: DomainId, pose: Pose) -> Result<(), String> {
    pose.validate()?;
    attach(ctx, obj, "sim.participant")?;
    run(
        ctx,
        &Transition::Place {
            obj,
            parent: SpatialParent::Direct { domain, pose },
        },
    )
    .map_err(|e| e.to_string())
}

/// Release `obj` from its window, leaving it in the room it resolved to.
///
/// # Errors
///
/// The transition's refusal.
pub fn unplace(ctx: &mut Ctx<'_>, obj: ObjId, reason: &str) -> Result<(), String> {
    let by = ctx.actor;
    run(
        ctx,
        &Transition::Unplace {
            obj,
            reason: reason.to_string(),
            by: Some(by),
            keep: true,
        },
    )
    .map_err(|e| e.to_string())
}

/// Where `obj` stands, when it is directly placed.
#[must_use]
pub fn pose_of(w: &World, obj: ObjId) -> Option<(DomainId, Pose)> {
    let st = w.module_state::<SpatialState>()?;
    match st.parent_of(obj) {
        SpatialParent::Direct { domain, pose } => Some((domain, pose)),
        _ => None,
    }
}

/// Everyone directly placed in `domain`, by id, with their poses.
#[must_use]
pub fn participants(w: &World, domain: DomainId) -> Vec<(ObjId, Pose)> {
    let Some(st) = w.module_state::<SpatialState>() else {
        return Vec::new();
    };
    st.placements
        .values()
        .filter_map(|p| match &p.parent {
            SpatialParent::Direct { domain: d, pose } if *d == domain => {
                Some((p.object, pose.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Close a window: release every participant where they stand and retire the
/// domain. Returns who was released.
///
/// # Errors
///
/// A message when the domain is not a live window, or a release is refused.
pub fn close_window(ctx: &mut Ctx<'_>, domain: DomainId) -> Result<Vec<ObjId>, String> {
    if window_of(ctx.w, domain).is_none() {
        return Err(format!("domain #{domain} has no open window"));
    }
    let members: Vec<ObjId> = participants(ctx.w, domain)
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    for &obj in &members {
        unplace(ctx, obj, "the window closed")?;
    }
    run(ctx, &Transition::Retire { domain }).map_err(|e| e.to_string())?;
    Ok(members)
}
