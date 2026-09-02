//! The one structural-transition primitive (design §7, cut-1 review): a
//! transition is **planned** purely against the current state and world —
//! every invariant, capacity, and projection move pre-validated — and then
//! **applied** with the projection moves first (the only fallible step,
//! already validated), followed by infallible slot mutation, index rebuild,
//! and persistence. A refused transition mutates nothing.

use std::collections::BTreeSet;

use mush_core::{BehaviorId, Ctx, Flags, ObjId, Outbox, World};

use crate::data::{
    DomainId, DomainStatus, InteriorSpaceRecord, PlacementRecord, RECOVERY_LOG, SpaceAnchor,
    SpatialParent, SpatialState, UnplacedInfo, persist_domain, persist_placement, persist_space,
    push_op,
};
use crate::graph::{ancestors, project, rebuild};

/// Why a transition was refused. Nothing was mutated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpatialError {
    /// The object does not exist or is being destroyed.
    #[error("{0} does not exist (or is being destroyed)")]
    NoSuchObject(ObjId),
    /// A required behavior binding is missing.
    #[error("{obj} must carry the `{behavior}` behavior")]
    MissingBehavior {
        /// The object.
        obj: ObjId,
        /// The behavior id.
        behavior: String,
    },
    /// No such domain.
    #[error("no domain #{0}")]
    NoSuchDomain(DomainId),
    /// The domain accepts no placements.
    #[error("domain #{0} is {1:?} and accepts no placements")]
    DomainClosed(DomainId, DomainStatus),
    /// No interior-space record.
    #[error("{0} is not an interior space (use @spatial/space)")]
    NoSuchSpace(ObjId),
    /// No such compartment.
    #[error("{space} has no compartment `{slot}`")]
    NoSuchSlot {
        /// The space.
        space: ObjId,
        /// The slot.
        slot: String,
    },
    /// No such mount.
    #[error("{host} has no mount `{mount}`")]
    NoSuchMount {
        /// The host.
        host: ObjId,
        /// The mount.
        mount: String,
    },
    /// Capacity reached.
    #[error("{what} is full ({max} max)")]
    Full {
        /// What is full.
        what: String,
        /// The limit.
        max: u32,
    },
    /// The move would create a cycle.
    #[error("placing {obj} there would make it its own ancestor (via {via})")]
    Cycle {
        /// The object.
        obj: ObjId,
        /// Where the cycle closes.
        via: ObjId,
    },
    /// Too deep.
    #[error("{obj} would exceed the maximum spatial depth")]
    TooDeep {
        /// The object.
        obj: ObjId,
    },
    /// A space's anchor and placement disagree (design §5.2 #6).
    #[error("{0}")]
    AnchorConflict(String),
    /// The object still has structural dependents.
    #[error("{obj} still has {count} dependent(s); move them first")]
    HasDependents {
        /// The object.
        obj: ObjId,
        /// How many.
        count: usize,
    },
    /// The object is not placed / not a spatial entity.
    #[error("{0} is not placed")]
    NotPlaced(ObjId),
    /// The object is on a route: it is EITHER traveling OR sim-placed, never
    /// both (the mirror of `route_ineligible`).
    #[error("{0} is en route; @route/abort it before placing it in the simulation")]
    EnRoute(ObjId),
    /// A projection move would be refused by the world.
    #[error("cannot project {obj} into {dest}: {reason}")]
    Projection {
        /// The object.
        obj: ObjId,
        /// The destination.
        dest: ObjId,
        /// The world's reason.
        reason: String,
    },
    /// The domain is still referenced.
    #[error("domain #{0} is still referenced by {1} placement(s)/space(s); clear them first")]
    DomainReferenced(DomainId, usize),
}

/// A structural transition (design §7).
#[derive(Clone, Debug, PartialEq)]
pub enum Transition {
    /// Place (or re-place) `obj` under `parent`.
    Place {
        /// The object.
        obj: ObjId,
        /// The new parent (never `Unplaced`; use `Unplace`).
        parent: SpatialParent,
    },
    /// Staff-unplace `obj`: it becomes a plain object again (projected to the
    /// recovery room unless `keep`).
    Unplace {
        /// The object.
        obj: ObjId,
        /// Why.
        reason: String,
        /// Who.
        by: Option<ObjId>,
        /// Leave `Obj.location` alone.
        keep: bool,
    },
    /// System-quarantine `root` and its whole subtree into the recovery room.
    Quarantine {
        /// The root.
        root: ObjId,
        /// Why.
        reason: String,
    },
    /// Define or re-anchor an interior space, creating the record if absent.
    Space {
        /// The space (a room carrying `sim.interior`).
        space: ObjId,
        /// The new anchor.
        anchor: SpaceAnchor,
    },
    /// Retire a domain (must be unreferenced).
    Retire {
        /// The domain.
        domain: DomainId,
    },
}

/// A validated transition: exactly what `apply` will do.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Placement records to set.
    pub set_placements: Vec<PlacementRecord>,
    /// Space records to set.
    pub set_spaces: Vec<InteriorSpaceRecord>,
    /// Domain ids whose status becomes Retired.
    pub retire: Vec<DomainId>,
    /// `Obj.location` moves `(object, destination)`, pre-validated.
    pub projections: Vec<(ObjId, ObjId)>,
    /// The audit line.
    pub audit: serde_json::Value,
}

fn live(w: &World, obj: ObjId) -> Result<(), SpatialError> {
    match w.get(obj) {
        Some(o) if !o.flags.contains(Flags::GOING) => Ok(()),
        _ => Err(SpatialError::NoSuchObject(obj)),
    }
}

fn needs(w: &World, obj: ObjId, behavior: &str) -> Result<(), SpatialError> {
    let id = BehaviorId::new(behavior).expect("static behavior id");
    if w.has_behavior(obj, &id) {
        Ok(())
    } else {
        Err(SpatialError::MissingBehavior {
            obj,
            behavior: behavior.to_string(),
        })
    }
}

/// Validate a projection move without mutating.
fn check_projection(w: &World, obj: ObjId, dest: ObjId) -> Result<(), SpatialError> {
    live(w, dest)?;
    let already = w.get(obj).and_then(|o| o.location) == Some(dest);
    if already {
        return Ok(());
    }
    w.can_move_to(obj, dest).map_err(|e| SpatialError::Projection {
        obj,
        dest,
        reason: e.to_string(),
    })
}

/// The anchor/placement consistency rule (design §5.2 #6) for a space with
/// `anchor`, given the parent it has or will have.
fn anchor_consistent(space: ObjId, anchor: &SpaceAnchor, parent: &SpatialParent) -> Result<(), SpatialError> {
    match (anchor, parent) {
        (SpaceAnchor::Unanchored, _) | (_, SpatialParent::Unplaced) => Ok(()),
        (SpaceAnchor::FixedDomain { domain, .. }, SpatialParent::Direct { domain: d, .. }) if d == domain => Ok(()),
        (SpaceAnchor::FixedDomain { domain, .. }, _) => Err(SpatialError::AnchorConflict(format!(
            "{space} is fixed in domain #{domain} but placed elsewhere"
        ))),
        (SpaceAnchor::MovingObject { host }, SpatialParent::Interior { space: s, .. }) if s == host => Ok(()),
        (SpaceAnchor::MovingObject { host }, SpatialParent::Attached { host: h, .. }) if h == host => Ok(()),
        (SpaceAnchor::MovingObject { host }, _) => Err(SpatialError::AnchorConflict(format!(
            "{space} rides {host} but is placed elsewhere"
        ))),
    }
}

/// Compute the projections for `obj` (with its parent as in `st`) and every
/// transitive attached descendant, validating each move.
fn projections_for(
    st: &SpatialState,
    w: &World,
    obj: ObjId,
    out: &mut Vec<(ObjId, ObjId)>,
) -> Result<(), SpatialError> {
    for o in st.subtree(obj) {
        if let Some(dest) = project(st, w, o) {
            check_projection(w, o, dest)?;
            out.push((o, dest));
        }
    }
    Ok(())
}

/// Plan `t` against `st`/`w`. Pure: nothing is mutated.
/// Refuse to sim-bind an object that is mid-journey — the placement-side half
/// of the EITHER-journey-OR-sim-placed invariant (`route_ineligible` is the
/// route-side half). Enforced in the validate stage, so a refusal mutates
/// nothing.
fn route_free(w: &World, obj: ObjId) -> Result<(), SpatialError> {
    if w.module_state::<crate::route::RouteState>().is_some_and(|rs| rs.is_traveling(obj)) {
        return Err(SpatialError::EnRoute(obj));
    }
    Ok(())
}

pub fn plan(st: &SpatialState, w: &World, t: &Transition) -> Result<Plan, SpatialError> {
    let mut plan = Plan::default();
    match t {
        Transition::Place { obj, parent } => {
            let obj = *obj;
            live(w, obj)?;
            needs(w, obj, "sim.participant")?;
            route_free(w, obj)?;
            let parent_obj = match parent {
                SpatialParent::Direct { domain, .. } => {
                    let d = st.domains.get(domain).ok_or(SpatialError::NoSuchDomain(*domain))?;
                    if d.status != DomainStatus::Active {
                        return Err(SpatialError::DomainClosed(*domain, d.status));
                    }
                    live(w, d.presentation_location)?;
                    d.host
                }
                SpatialParent::Interior { space, slot } => {
                    live(w, *space)?;
                    let s = st.spaces.get(space).ok_or(SpatialError::NoSuchSpace(*space))?;
                    needs(w, *space, "sim.interior")?;
                    if let Some(slot) = slot {
                        let cap = s.compartments.get(slot).ok_or_else(|| SpatialError::NoSuchSlot {
                            space: *space,
                            slot: slot.to_string(),
                        })?;
                        if let Some(max) = cap.maximum_items {
                            let mut n = st.occupants(*space, Some(slot));
                            if matches!(st.parent_of(obj), SpatialParent::Interior { space: sp, slot: ref sl, .. } if sp == *space && sl.as_ref() == Some(slot)) {
                                n = n.saturating_sub(1); // re-placing into the same slot
                            }
                            if n as u32 >= max {
                                return Err(SpatialError::Full { what: format!("{space} compartment `{slot}`"), max });
                            }
                        }
                    }
                    *space
                }
                SpatialParent::Attached { host, mount, .. } => {
                    live(w, *host)?;
                    let spec = st
                        .mounts
                        .get(host)
                        .and_then(|m| m.mounts.get(mount))
                        .ok_or_else(|| SpatialError::NoSuchMount { host: *host, mount: mount.to_string() })?;
                    if let Some(max) = spec.capacity.maximum_items {
                        let mut n = st.attachments(*host, mount);
                        if matches!(st.parent_of(obj), SpatialParent::Attached { host: h, mount: ref m, .. } if h == *host && m == mount) {
                            n = n.saturating_sub(1);
                        }
                        if n as u32 >= max {
                            return Err(SpatialError::Full { what: format!("{host} mount `{mount}`"), max });
                        }
                    }
                    *host
                }
                SpatialParent::Unplaced => return Err(SpatialError::NotPlaced(obj)),
            };
            // Acyclic + bounded: obj must not be an ancestor of its new parent.
            if parent_obj == obj {
                return Err(SpatialError::Cycle { obj, via: obj });
            }
            let up = ancestors(st, parent_obj)?;
            if up.contains(&obj) {
                return Err(SpatialError::Cycle { obj, via: parent_obj });
            }
            // Depth invariant: every node's ancestor chain is at most
            // MAX_SPATIAL_DEPTH. The deepest node under `obj` after the move has
            // (parent's ancestors) + 1 (obj) + (height of obj's own subtree).
            let height = st.subtree_height(obj);
            if up.len() + 1 + height > crate::data::MAX_SPATIAL_DEPTH {
                return Err(SpatialError::TooDeep { obj });
            }
            // A space being placed must agree with its anchor.
            if let Some(s) = st.spaces.get(&obj) {
                anchor_consistent(obj, &s.anchor, parent)?;
            }
            // Build the post-state view for projection: clone the state with
            // the new record, then project the subtree.
            let prev = st.placements.get(&obj);
            let rec = PlacementRecord {
                object: obj,
                parent: parent.clone(),
                revision: prev.map_or(1, |p| p.revision + 1),
                unplaced: None,
            };
            let mut post = st.clone();
            post.placements.insert(obj, rec.clone());
            post.derived = rebuild(&post);
            projections_for(&post, w, obj, &mut plan.projections)?;
            plan.audit = serde_json::json!({
                "op": "place", "obj": obj.0, "parent": serde_json::to_value(parent).unwrap_or_default(),
                "previous": prev.map(|p| serde_json::to_value(&p.parent).unwrap_or_default()),
            });
            plan.set_placements.push(rec);
        }
        Transition::Unplace { obj, reason, by, keep } => {
            let obj = *obj;
            let prev = st.placements.get(&obj).filter(|p| p.parent.is_placed()).ok_or(SpatialError::NotPlaced(obj))?;
            let deps = st.children_of(obj).len();
            if deps > 0 {
                return Err(SpatialError::HasDependents { obj, count: deps });
            }
            if !keep {
                check_projection(w, obj, st.recovery_room)?;
                plan.projections.push((obj, st.recovery_room));
            }
            plan.set_placements.push(PlacementRecord {
                object: obj,
                parent: SpatialParent::Unplaced,
                revision: prev.revision + 1,
                unplaced: Some(UnplacedInfo {
                    reason: reason.clone(),
                    by: *by,
                    at: w.now(),
                    previous: Some(prev.parent.clone()),
                    quarantined: false,
                }),
            });
            plan.audit = serde_json::json!({ "op": "unplace", "obj": obj.0, "reason": reason, "by": by.map(|b| b.0) });
        }
        Transition::Quarantine { root, reason } => {
            let root = *root;
            let mut moved = Vec::new();
            for o in st.subtree(root) {
                let Some(prev) = st.placements.get(&o) else { continue };
                if !prev.parent.is_placed() && prev.unplaced.as_ref().is_some_and(|u| u.quarantined) {
                    continue; // already quarantined
                }
                // Prefer the recovery room; fall back to Limbo if it is gone.
                let dest = if live(w, st.recovery_room).is_ok() { st.recovery_room } else { mush_core::LIMBO };
                if w.get(o).is_some() && check_projection(w, o, dest).is_ok() {
                    plan.projections.push((o, dest));
                }
                plan.set_placements.push(PlacementRecord {
                    object: o,
                    parent: SpatialParent::Unplaced,
                    revision: prev.revision + 1,
                    unplaced: Some(UnplacedInfo {
                        reason: reason.clone(),
                        by: None,
                        at: w.now(),
                        previous: Some(prev.parent.clone()),
                        quarantined: true,
                    }),
                });
                moved.push(o.0);
            }
            plan.audit = serde_json::json!({ "op": "quarantine", "root": root.0, "objects": moved, "reason": reason });
        }
        Transition::Space { space, anchor } => {
            let space = *space;
            live(w, space)?;
            route_free(w, space)?;
            // The base record: keep an existing one's compartments/rooms, or a
            // fresh Unanchored shell whose sole room is the space itself.
            let base = st.spaces.get(&space).cloned().unwrap_or_else(|| InteriorSpaceRecord {
                object: space,
                anchor: SpaceAnchor::Unanchored,
                compartments: Default::default(),
                rooms: std::iter::once(space).collect(),
                egresses: Default::default(),
            });
            match anchor {
                SpaceAnchor::FixedDomain { domain, .. } => {
                    let d = st.domains.get(domain).ok_or(SpatialError::NoSuchDomain(*domain))?;
                    if d.status == DomainStatus::Retired {
                        return Err(SpatialError::DomainClosed(*domain, d.status));
                    }
                    // The anchor edge runs space -> domain.host, so the host must
                    // not sit under the space.
                    if d.host == space {
                        return Err(SpatialError::Cycle { obj: space, via: space });
                    }
                    let up = ancestors(st, d.host)?;
                    if up.contains(&space) {
                        return Err(SpatialError::Cycle { obj: space, via: d.host });
                    }
                    if up.len() + 1 + st.subtree_height(space) > crate::data::MAX_SPATIAL_DEPTH {
                        return Err(SpatialError::TooDeep { obj: space });
                    }
                }
                SpaceAnchor::MovingObject { host } => {
                    live(w, *host)?;
                    if *host == space {
                        return Err(SpatialError::Cycle { obj: space, via: space });
                    }
                    let up = ancestors(st, *host)?;
                    if up.contains(&space) {
                        return Err(SpatialError::Cycle { obj: space, via: *host });
                    }
                    if up.len() + 1 + st.subtree_height(space) > crate::data::MAX_SPATIAL_DEPTH {
                        return Err(SpatialError::TooDeep { obj: space });
                    }
                }
                SpaceAnchor::Unanchored => {}
            }
            anchor_consistent(space, anchor, &st.parent_of(space))?;
            let mut rec = base;
            rec.anchor = anchor.clone();
            plan.audit = serde_json::json!({ "op": "space", "space": space.0, "anchor": serde_json::to_value(anchor).unwrap_or_default() });
            plan.set_spaces.push(rec);
        }
        Transition::Retire { domain } => {
            let d = st.domains.get(domain).ok_or(SpatialError::NoSuchDomain(*domain))?;
            if d.status == DomainStatus::Retired {
                return Err(SpatialError::DomainClosed(*domain, d.status));
            }
            let refs = st.derived.domain_direct.get(domain).map_or(0, BTreeSet::len)
                + st.derived.spaces_in_domain.get(domain).map_or(0, BTreeSet::len);
            if refs > 0 {
                return Err(SpatialError::DomainReferenced(*domain, refs));
            }
            plan.retire.push(*domain);
            plan.audit = serde_json::json!({ "op": "retire", "domain": domain.0 });
        }
    }
    Ok(plan)
}

/// Apply a plan. The projection moves run first: every one was pre-validated by
/// `plan` via `can_move_to`, so a refusal here means the world changed between
/// plan and apply — which cannot happen inside one command, since callers run
/// plan and apply back to back on the single world thread. Only then are the
/// records, indexes, and persistence written. On the (unreachable-in-practice)
/// mid-batch refusal, `Err` is returned after any earlier moves but before any
/// record change.
pub fn apply(st: &mut SpatialState, w: &mut World, out: &mut Outbox, plan: Plan) -> Result<(), SpatialError> {
    for &(obj, dest) in &plan.projections {
        if w.get(obj).and_then(|o| o.location) == Some(dest) {
            continue;
        }
        w.move_to(obj, dest).map_err(|e| SpatialError::Projection {
            obj,
            dest,
            reason: e.to_string(),
        })?;
    }
    for p in &plan.set_placements {
        st.placements.insert(p.object, p.clone());
    }
    for s in &plan.set_spaces {
        st.spaces.insert(s.object, s.clone());
    }
    for d in &plan.retire {
        if let Some(rec) = st.domains.get_mut(d) {
            rec.status = DomainStatus::Retired;
            rec.revision += 1;
        }
    }
    st.derived = rebuild(st);
    for p in &plan.set_placements {
        persist_placement(out, p);
    }
    for s in &plan.set_spaces {
        persist_space(out, s);
    }
    for d in &plan.retire {
        if let Some(rec) = st.domains.get(d) {
            persist_domain(out, rec);
        }
    }
    if !plan.audit.is_null() {
        let (_, op) = st.recovery.append(RECOVERY_LOG, plan.audit);
        push_op(out, op);
    }
    Ok(())
}

/// Plan and apply in one step against a command context.
pub fn run(ctx: &mut Ctx<'_>, t: &Transition) -> Result<(), SpatialError> {
    let plan = {
        let st = ctx.w.module_state::<SpatialState>().expect("sim state slot installed at registration");
        plan(st, ctx.w, t)?
    };
    // Take the slot out to satisfy the borrow checker (world + slot are both
    // mutated); put it back before returning.
    let mut st = std::mem::take(ctx.w.module_state_mut::<SpatialState>().expect("sim state slot installed at registration"));
    let r = apply(&mut st, ctx.w, ctx.out, plan);
    *ctx.w.module_state_mut::<SpatialState>().expect("sim state slot installed at registration") = st;
    r
}

