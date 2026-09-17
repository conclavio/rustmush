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
    /// S7: the driver the domain records; the passive one when `None`.
    pub driver: Option<mush_core::DefinitionRef>,
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
/// S7: register the indirect-fire arc the atlas answers over a shared window.
///
/// # Errors
///
/// A world without the simulation, or an arc already registered.
pub fn register_window_arc(w: &mut World, f: crate::data::ArcFn) -> Result<(), String> {
    let slot = w
        .module_state_mut::<WindowGeometry>()
        .ok_or_else(|| "the simulation is not composed on this world".to_string())?;
    if slot.arc.is_some() {
        return Err("the window arc is already registered".to_string());
    }
    slot.arc = Some(f);
    Ok(())
}

/// S7: whether an indirect shot from `near` clears the ground to `far`; `None`
/// when no arc is registered or the two share no window.
#[must_use]
pub fn arc_between(w: &World, near: ObjId, far: ObjId) -> Option<bool> {
    let f = w.module_state::<WindowGeometry>()?.arc.clone()?;
    f(w, near, far)
}

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
            driver: open.driver.clone().unwrap_or_else(passive_definition_ref),
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

/// **Open a passive domain on `host`, presenting in `presentation`** — the body
/// of `@domain/create <host>=sim.passive`, callable without a command line
/// (16b cut C.2).
///
/// A passive domain is a metric frame with no window composite and no clock: an
/// object placed in it has a [`Pose`] and therefore a position, which is all the
/// arena's check-out needs of a world placement. Cut C's proving-ground applier
/// opens one on the room it makes, so the eight stock tanks are *placed* objects
/// on a fresh world and `@arena/start` has something to take custody of.
///
/// The host gains `sim.domain` if it lacks it, exactly as [`open_window`] does.
///
/// # Errors
///
/// A message when the host is gone or going, the host already has a domain,
/// [`MAX_WINDOWS`] are open, the presentation object is not a live room, or the
/// allocator is spent.
pub fn open_passive_domain(
    ctx: &mut Ctx<'_>,
    host: ObjId,
    presentation: ObjId,
) -> Result<DomainId, String> {
    {
        let st = state(ctx.w)?;
        if let Some(d) = st.domain_of_host(host) {
            return Err(format!("that host already has domain #{}", d.id));
        }
        if windows(ctx.w).count() >= MAX_WINDOWS {
            return Err(format!("{MAX_WINDOWS} windows are open already"));
        }
    }
    if ctx
        .w
        .get(host)
        .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Err("no such host".to_string());
    }
    let live_room = ctx.w.get(presentation).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING)
    });
    if !live_room {
        return Err("a domain presents in a live room".to_string());
    }
    attach(ctx, host, "sim.domain")?;
    let (rec, next) = {
        let s = state_mut(ctx.w)?;
        let id = s.alloc_domain_id().map_err(|e| e.to_string())?;
        let rec = DomainRecord {
            id,
            host,
            driver: passive_definition_ref(),
            status: DomainStatus::Active,
            clock: DomainClock::Passive,
            presentation_location: presentation,
            revision: 1,
            window: None,
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

/// **Place `obj` inside the interior space `space`** — one planned transition,
/// the projection onto the space included, the `sim.participant` behaviour
/// attached if it is missing (16b cut K.2, K2; doc 16 §3.7 "Boarding").
///
/// The interior twin of [`place`], and it exists for the same reason: the two
/// facts a placement needs — the behaviour and the transition — are this crate's,
/// and a caller outside it that spelled them itself would be a second placement
/// path. `@spatial/place/interior` is this write behind a staff verb with an
/// argument parser.
///
/// `slot` names a compartment the space **declares** (`@spatial/slot`); `None`
/// puts the object in the space itself, which is what a one-seat pod is.
///
/// # Errors
///
/// The behaviour's refusal, or the transition's — no such space, not an interior
/// space, no such compartment, a full one, or a cycle.
pub fn place_interior(
    ctx: &mut Ctx<'_>,
    obj: ObjId,
    space: ObjId,
    slot: Option<crate::data::SlotId>,
) -> Result<(), String> {
    attach(ctx, obj, "sim.participant")?;
    run(
        ctx,
        &Transition::Place {
            obj,
            parent: SpatialParent::Interior { space, slot },
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

/// Everyone directly placed in `domain`, by id, with their poses — **the
/// world-side roster**.
///
/// 16b cut T.3 (doc 16 §3.1 "Custody", §3.6 "World-side exclusion"): a unit an
/// arena holds is **not** one of them. Its world placement stays exactly where it
/// was and is deliberately left frozen (so the pilot's channel grants, which
/// require a placed unit, survive the match), but while the arena is the single
/// writer for that unit's pose the world's own answer to "who is here" would be a
/// stale one — and every world-side pulse, map frame and unit listing reads this
/// function. Excluding it here is what makes doc 16's rule *a deployed unit is
/// never a participant of a world-side pulse* true by construction rather than by
/// each caller remembering; the frozen placement alone does **not** imply it,
/// because a frozen placement is still a placement.
///
/// [`raw_participants`] is the unfiltered read, for the two callers that must see
/// a deployed unit: closing the window (which has to release it) and the custody
/// check-out itself.
#[must_use]
pub fn participants(w: &World, domain: DomainId) -> Vec<(ObjId, Pose)> {
    raw_participants(w, domain)
        .into_iter()
        .filter(|(obj, _)| !crate::deployed::is_deployed(w, *obj))
        .collect()
}

/// Everyone directly placed in `domain`, **including** units under an arena's
/// custody — the placement graph's own answer, before doc 16 §3.6's exclusion.
#[must_use]
pub fn raw_participants(w: &World, domain: DomainId) -> Vec<(ObjId, Pose)> {
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
/// 16b cut T.3: this is one of the two readers that takes the **raw** roster.
/// A window that still holds a unit under an arena's custody is refused outright
/// below — retiring the domain out from under a running match would leave the
/// arena integrating a unit whose world placement no longer exists — but the
/// release itself must be able to see everything the placement graph holds, not
/// doc 16 §3.6's projection of it.
///
/// # Errors
///
/// A message when the domain is not a live window, when it holds a deployed
/// unit, or when a release is refused.
pub fn close_window(ctx: &mut Ctx<'_>, domain: DomainId) -> Result<Vec<ObjId>, String> {
    if window_of(ctx.w, domain).is_none() {
        return Err(format!("domain #{domain} has no open window"));
    }
    let members: Vec<ObjId> = raw_participants(ctx.w, domain)
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    for &obj in &members {
        crate::deployed::refuse_if_deployed(ctx.w, obj).map_err(|error| error.to_string())?;
    }
    for &obj in &members {
        unplace(ctx, obj, "the window closed")?;
    }
    run(ctx, &Transition::Retire { domain }).map_err(|e| e.to_string())?;
    Ok(members)
}
