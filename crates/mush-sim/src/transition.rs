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
    SpatialCounterError, SpatialParent, SpatialState, UnplacedInfo, checked_placement_revision,
    persist_domain, persist_placement, persist_space, push_op,
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
    /// Boot held the object's unreadable authoritative placement. Only an
    /// explicit repair/migration workflow may replace that record.
    #[error("{0} has a held unreadable placement; repair it before re-placing")]
    HeldPlacement(ObjId),
    /// The object is on a route: it is EITHER traveling OR sim-placed, never
    /// both (the mirror of `route_ineligible`).
    #[error("{0} is en route; @route/abort it before placing it in the simulation")]
    EnRoute(ObjId),
    /// E7c6 B2/B3: a carrier under way is a **sealed subtree**. Nothing crosses
    /// the boundary while it moves — a craft cannot launch mid-leg and nobody
    /// boards mid-passage — so nothing can be orphaned by an arrival (the
    /// T lens). Movement *within* the subtree stays free.
    #[error(
        "{obj} is aboard {root}, which is under way; nothing crosses a carrier's boundary mid-journey"
    )]
    CarrierSealed {
        /// The object whose placement was refused.
        obj: ObjId,
        /// The carrier root that is under way.
        root: ObjId,
    },
    /// E7c6: the carrier's subtree is larger than one move may visit.
    #[error("{root} carries {count} records; a carrier move visits at most {max}")]
    CarrierTooLarge {
        /// The carrier root.
        root: ObjId,
        /// How many records its subtree holds.
        count: usize,
        /// [`crate::journey::MAX_CARRIER_SUBTREE`].
        max: usize,
    },
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
    /// The authoritative recovery audit append could not be prepared. This is
    /// checked before any world or spatial mutation.
    #[error("recovery ledger append refused: {0}")]
    RecoveryLedger(String),
    /// A persisted monotonic counter cannot represent the requested change.
    #[error(transparent)]
    Counter(#[from] SpatialCounterError),
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
    /// E7 cut 6 (B2/B3): move a **carrier root** — and, by construction, its
    /// whole subtree — in one atomic step. `parent` is `None` while the carrier
    /// is under way (the root is deliberately not directly placed in a live
    /// domain; only its subtree is) and `Some(..)` when it comes to rest.
    ///
    /// This is the ONE checked carrier path: reach it through
    /// [`move_carrier_subtree`], never by constructing the variant elsewhere.
    CarrierMove {
        /// The carrier root.
        root: ObjId,
        /// Where it comes to rest, or `None` to put it under way.
        parent: Option<SpatialParent>,
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
    w.can_move_to(obj, dest)
        .map_err(|e| SpatialError::Projection {
            obj,
            dest,
            reason: e.to_string(),
        })
}

/// The anchor/placement consistency rule (design §5.2 #6) for a space with
/// `anchor`, given the parent it has or will have.
fn anchor_consistent(
    space: ObjId,
    anchor: &SpaceAnchor,
    parent: &SpatialParent,
) -> Result<(), SpatialError> {
    match (anchor, parent) {
        (SpaceAnchor::Unanchored, _) | (_, SpatialParent::Unplaced) => Ok(()),
        (SpaceAnchor::FixedDomain { domain, .. }, SpatialParent::Direct { domain: d, .. })
            if d == domain =>
        {
            Ok(())
        }
        (SpaceAnchor::FixedDomain { domain, .. }, _) => Err(SpatialError::AnchorConflict(format!(
            "{space} is fixed in domain #{domain} but placed elsewhere"
        ))),
        (SpaceAnchor::MovingObject { host }, SpatialParent::Interior { space: s, .. })
            if s == host =>
        {
            Ok(())
        }
        (SpaceAnchor::MovingObject { host }, SpatialParent::Attached { host: h, .. })
            if h == host =>
        {
            Ok(())
        }
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
    // Bounded on records VISITED (D-LBK-5), and by ONE constant: `Place`,
    // `CarrierMove` and the root quarantine all build their projection bundle
    // here, so `MAX_CARRIER_SUBTREE` caps every one of them with the same
    // refuse-never-truncate shape (E7 cut 6 fold, C8(b)).
    let subtree = st
        .subtree_bounded(obj, crate::journey::MAX_CARRIER_SUBTREE)
        .map_err(|count| SpatialError::CarrierTooLarge {
            root: obj,
            count,
            max: crate::journey::MAX_CARRIER_SUBTREE,
        })?;
    for o in subtree {
        if let Some(dest) = project(st, w, o) {
            check_projection(w, o, dest)?;
            out.push((o, dest));
        }
    }
    Ok(())
}

/// Plan `t` against `st`/`w`. Pure: nothing is mutated.
/// Resolve the structural parent OBJECT a [`SpatialParent`] names, running every
/// existence, liveness and capacity check that placing `obj` there requires.
/// Shared by [`Transition::Place`] and [`Transition::CarrierMove`], so the two
/// can never diverge on what a legal parent is.
fn resolve_parent(
    st: &SpatialState,
    w: &World,
    obj: ObjId,
    parent: &SpatialParent,
) -> Result<ObjId, SpatialError> {
    match parent {
        SpatialParent::Direct { domain, .. } => {
            let d = st
                .domains
                .get(domain)
                .ok_or(SpatialError::NoSuchDomain(*domain))?;
            if d.status != DomainStatus::Active {
                return Err(SpatialError::DomainClosed(*domain, d.status));
            }
            live(w, d.presentation_location)?;
            Ok(d.host)
        }
        SpatialParent::Interior { space, slot } => {
            live(w, *space)?;
            let s = st
                .spaces
                .get(space)
                .ok_or(SpatialError::NoSuchSpace(*space))?;
            needs(w, *space, "sim.interior")?;
            if let Some(slot) = slot {
                let cap = s
                    .compartments
                    .get(slot)
                    .ok_or_else(|| SpatialError::NoSuchSlot {
                        space: *space,
                        slot: slot.to_string(),
                    })?;
                if let Some(max) = cap.maximum_items {
                    let mut n = st.occupants(*space, Some(slot));
                    if matches!(st.parent_of(obj), SpatialParent::Interior { space: sp, slot: ref sl, .. } if sp == *space && sl.as_ref() == Some(slot))
                    {
                        n = n.saturating_sub(1); // re-placing into the same slot
                    }
                    if n as u32 >= max {
                        return Err(SpatialError::Full {
                            what: format!("{space} compartment `{slot}`"),
                            max,
                        });
                    }
                }
            }
            Ok(*space)
        }
        SpatialParent::Attached { host, mount, .. } => {
            live(w, *host)?;
            let spec = st
                .mounts
                .get(host)
                .and_then(|m| m.mounts.get(mount))
                .ok_or_else(|| SpatialError::NoSuchMount {
                    host: *host,
                    mount: mount.to_string(),
                })?;
            if let Some(max) = spec.capacity.maximum_items {
                let mut n = st.attachments(*host, mount);
                if matches!(st.parent_of(obj), SpatialParent::Attached { host: h, mount: ref m, .. } if h == *host && m == mount)
                {
                    n = n.saturating_sub(1);
                }
                if n as u32 >= max {
                    return Err(SpatialError::Full {
                        what: format!("{host} mount `{mount}`"),
                        max,
                    });
                }
            }
            Ok(*host)
        }
        SpatialParent::Unplaced => Err(SpatialError::NotPlaced(obj)),
    }
}

/// **The carrier seal** (E7c6 B3, the T lens; E7 cut 6 fold C1). While a
/// carrier root is under way its subtree is sealed: a placement may not cross
/// the boundary in **any** direction, so a craft cannot launch mid-leg, nobody
/// boards mid-passage, and nothing walks out through `Unplace` (which is the
/// economy's `InteriorCustodyProvider::move_out` path — it deliberately skips
/// this module's own `CarryAttempt` hook, so the check has to live in the
/// transition, not the gate).
///
/// A move *within* one carrier's subtree (an occupant walking between
/// compartments) is untouched, and so is the carrier's own move, which goes
/// through [`Transition::CarrierMove`] and names itself `exempt`.
///
/// `parent_obj` is `None` for a crossing that leaves the graph entirely
/// (`Unplace`, or a `CarrierMove` putting a root under way). `exempt` is the
/// carrier root whose OWN journey is performing the move: its own boundary is
/// not a crossing for itself.
fn carrier_seal_with(
    st: &SpatialState,
    w: &World,
    obj: ObjId,
    parent_obj: Option<ObjId>,
    exempt: Option<ObjId>,
) -> Result<(), SpatialError> {
    let Some(book) = w.module_state::<crate::journey::JourneyBook>() else {
        return Ok(());
    };
    let root_of = |x: ObjId| {
        crate::journey::under_way_root_of(st, book, x).filter(|root| Some(*root) != exempt)
    };
    let from = root_of(obj);
    let to = parent_obj.and_then(root_of);
    if from == to {
        return Ok(());
    }
    match from.or(to) {
        Some(root) => Err(SpatialError::CarrierSealed { obj, root }),
        None => Ok(()),
    }
}

/// The seal over an ordinary placement crossing (`Place`, `Unplace`).
fn carrier_seal(
    st: &SpatialState,
    w: &World,
    obj: ObjId,
    parent_obj: Option<ObjId>,
) -> Result<(), SpatialError> {
    carrier_seal_with(st, w, obj, parent_obj, None)
}

pub fn plan(st: &SpatialState, w: &World, t: &Transition) -> Result<Plan, SpatialError> {
    let mut plan = Plan::default();
    match t {
        Transition::Place { obj, parent } => {
            let obj = *obj;
            if st.held_placement(obj).is_some() {
                return Err(SpatialError::HeldPlacement(obj));
            }
            live(w, obj)?;
            needs(w, obj, "sim.participant")?;
            let parent_obj = resolve_parent(st, w, obj, parent)?;
            carrier_seal(st, w, obj, Some(parent_obj))?;
            // Acyclic + bounded: obj must not be an ancestor of its new parent.
            if parent_obj == obj {
                return Err(SpatialError::Cycle { obj, via: obj });
            }
            let up = ancestors(st, parent_obj)?;
            if up.contains(&obj) {
                return Err(SpatialError::Cycle {
                    obj,
                    via: parent_obj,
                });
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
                revision: match prev {
                    Some(p) => checked_placement_revision(obj, p.revision)?,
                    None => 1,
                },
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
        Transition::Unplace {
            obj,
            reason,
            by,
            keep,
        } => {
            let obj = *obj;
            let prev = st
                .placements
                .get(&obj)
                .filter(|p| p.parent.is_placed())
                .ok_or(SpatialError::NotPlaced(obj))?;
            let deps = st.children_of(obj).len();
            if deps > 0 {
                return Err(SpatialError::HasDependents { obj, count: deps });
            }
            // The same seal, on the crossing that leaves the graph entirely.
            // `custody::InteriorCustodyProvider::move_out` reaches here with
            // this module's own carry hook deliberately skipped, so this is the
            // only place that withdrawal can be refused.
            carrier_seal(st, w, obj, None)?;
            if !keep {
                check_projection(w, obj, st.recovery_room)?;
                plan.projections.push((obj, st.recovery_room));
            }
            plan.set_placements.push(PlacementRecord {
                object: obj,
                parent: SpatialParent::Unplaced,
                revision: checked_placement_revision(obj, prev.revision)?,
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
                let Some(prev) = st.placements.get(&o) else {
                    continue;
                };
                if !prev.parent.is_placed() && prev.unplaced.as_ref().is_some_and(|u| u.quarantined)
                {
                    continue; // already quarantined
                }
                // Prefer the recovery room; fall back to Limbo if it is gone.
                let dest = if live(w, st.recovery_room).is_ok() {
                    st.recovery_room
                } else {
                    mush_core::LIMBO
                };
                if w.get(o).is_some() && check_projection(w, o, dest).is_ok() {
                    plan.projections.push((o, dest));
                }
                // Destruction quarantine is a terminal safety transition. At
                // revision exhaustion, retaining MAX is safer than refusing
                // and leaving a live graph edge aimed at a destroyed object.
                // Recovery/re-placement still fail closed because their
                // checked revision advance cannot move beyond MAX.
                let revision = match checked_placement_revision(o, prev.revision) {
                    Ok(revision) => revision,
                    Err(error) => {
                        tracing::error!(
                            object = %o,
                            %error,
                            "sim: placement revision exhausted; quarantining at terminal revision"
                        );
                        u64::MAX
                    }
                };
                plan.set_placements.push(PlacementRecord {
                    object: o,
                    parent: SpatialParent::Unplaced,
                    revision,
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
            // The base record: keep an existing one's compartments/rooms, or a
            // fresh Unanchored shell whose sole room is the space itself.
            let base = st
                .spaces
                .get(&space)
                .cloned()
                .unwrap_or_else(|| InteriorSpaceRecord {
                    object: space,
                    anchor: SpaceAnchor::Unanchored,
                    compartments: Default::default(),
                    rooms: std::iter::once(space).collect(),
                    egresses: Default::default(),
                });
            match anchor {
                SpaceAnchor::FixedDomain { domain, .. } => {
                    let d = st
                        .domains
                        .get(domain)
                        .ok_or(SpatialError::NoSuchDomain(*domain))?;
                    if d.status == DomainStatus::Retired {
                        return Err(SpatialError::DomainClosed(*domain, d.status));
                    }
                    // The anchor edge runs space -> domain.host, so the host must
                    // not sit under the space.
                    if d.host == space {
                        return Err(SpatialError::Cycle {
                            obj: space,
                            via: space,
                        });
                    }
                    let up = ancestors(st, d.host)?;
                    if up.contains(&space) {
                        return Err(SpatialError::Cycle {
                            obj: space,
                            via: d.host,
                        });
                    }
                    if up.len() + 1 + st.subtree_height(space) > crate::data::MAX_SPATIAL_DEPTH {
                        return Err(SpatialError::TooDeep { obj: space });
                    }
                }
                SpaceAnchor::MovingObject { host } => {
                    live(w, *host)?;
                    if *host == space {
                        return Err(SpatialError::Cycle {
                            obj: space,
                            via: space,
                        });
                    }
                    let up = ancestors(st, *host)?;
                    if up.contains(&space) {
                        return Err(SpatialError::Cycle {
                            obj: space,
                            via: *host,
                        });
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
            let d = st
                .domains
                .get(domain)
                .ok_or(SpatialError::NoSuchDomain(*domain))?;
            if d.status == DomainStatus::Retired {
                return Err(SpatialError::DomainClosed(*domain, d.status));
            }
            let refs = st
                .derived
                .domain_direct
                .get(domain)
                .map_or(0, BTreeSet::len)
                + st.derived
                    .spaces_in_domain
                    .get(domain)
                    .map_or(0, BTreeSet::len);
            if refs > 0 {
                return Err(SpatialError::DomainReferenced(*domain, refs));
            }
            plan.retire.push(*domain);
            plan.audit = serde_json::json!({ "op": "retire", "domain": domain.0 });
        }
        Transition::CarrierMove { root, parent } => {
            let root = *root;
            if st.held_placement(root).is_some() {
                return Err(SpatialError::HeldPlacement(root));
            }
            live(w, root)?;
            needs(w, root, "sim.participant")?;
            // Bounded on records VISITED (D-LBK-5), closing §7's missing
            // per-carrier cap: a pathological carrier cannot make an arrival
            // bundle unbounded — the move is refused instead.
            // The walk itself stops at the cap (E7 cut 6 fold, C8(a)): the
            // refusal costs `MAX_CARRIER_SUBTREE + 1` records visited, not the
            // whole pathological graph, and it is still refuse-never-truncate.
            let subtree = st
                .subtree_bounded(root, crate::journey::MAX_CARRIER_SUBTREE)
                .map_err(|count| SpatialError::CarrierTooLarge {
                    root,
                    count,
                    max: crate::journey::MAX_CARRIER_SUBTREE,
                })?;
            let prev = st.placements.get(&root);
            let revision = match prev {
                Some(p) => checked_placement_revision(root, p.revision)?,
                None => 1,
            };
            let rec = match parent {
                Some(parent) => {
                    let parent_obj = resolve_parent(st, w, root, parent)?;
                    // A carrier may not come to rest inside ANOTHER carrier
                    // that is under way: the arrival is a boundary crossing
                    // like any other (E7 cut 6 fold, C1(b)). Its own journey is
                    // exempt, which is what makes this the ONE carrier path.
                    carrier_seal_with(st, w, root, Some(parent_obj), Some(root))?;
                    // **The carrier-root invariant.** A root may never come to
                    // rest inside its own subtree — not at any depth, not
                    // through a bay it carries. Stated twice on purpose: once
                    // over the descendant relation (the atomic subtree that
                    // moves with it) and once over the ancestor relation
                    // (`graph::ancestors`), which are duals, so a malformed
                    // record set cannot slip between them.
                    if parent_obj == root || subtree.contains(&parent_obj) {
                        return Err(SpatialError::Cycle {
                            obj: root,
                            via: parent_obj,
                        });
                    }
                    let up = ancestors(st, parent_obj)?;
                    if up.contains(&root) {
                        return Err(SpatialError::Cycle {
                            obj: root,
                            via: parent_obj,
                        });
                    }
                    if up.len() + 1 + st.subtree_height(root) > crate::data::MAX_SPATIAL_DEPTH {
                        return Err(SpatialError::TooDeep { obj: root });
                    }
                    if let Some(s) = st.spaces.get(&root) {
                        anchor_consistent(root, &s.anchor, parent)?;
                    }
                    PlacementRecord {
                        object: root,
                        parent: parent.clone(),
                        revision,
                        unplaced: None,
                    }
                }
                None => {
                    // Nor may a craft launch OUT of a sealed mothership by
                    // being given a journey of its own (C1(a)).
                    carrier_seal_with(st, w, root, None, Some(root))?;
                    PlacementRecord {
                        object: root,
                        parent: SpatialParent::Unplaced,
                        revision,
                        unplaced: Some(UnplacedInfo {
                            reason: "under way on a carrier journey".into(),
                            by: None,
                            at: w.now(),
                            previous: prev
                                .map(|p| p.parent.clone())
                                .filter(SpatialParent::is_placed),
                            // NOT quarantined: a carrier under way is a live,
                            // crewed object whose subtree stays placed; only its
                            // own direct placement is suspended.
                            quarantined: false,
                        }),
                    }
                }
            };
            // The whole subtree's projections are recomputed against the post
            // state and pre-validated, so the move is atomic: everything hosted
            // arrives with the root, and a refusal mutates nothing.
            let mut post = st.clone();
            post.placements.insert(root, rec.clone());
            post.derived = rebuild(&post);
            projections_for(&post, w, root, &mut plan.projections)?;
            plan.audit = serde_json::json!({
                "op": "carrier_move",
                "root": root.0,
                "parent": parent
                    .as_ref()
                    .map(|p| serde_json::to_value(p).unwrap_or_default()),
                "previous": prev.map(|p| serde_json::to_value(&p.parent).unwrap_or_default()),
                "subtree": subtree.len(),
            });
            plan.set_placements.push(rec);
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
pub fn apply(
    st: &mut SpatialState,
    w: &mut World,
    out: &mut Outbox,
    plan: Plan,
) -> Result<(), SpatialError> {
    let prepared_audit = if plan.audit.is_null() {
        None
    } else {
        Some(
            st.recovery
                .prepare_append(RECOVERY_LOG, plan.audit.clone())
                .map_err(|error| SpatialError::RecoveryLedger(error.to_string()))?,
        )
    };
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
            // Retirement is terminal. If this record has exhausted its
            // revision space, retaining MAX while closing the domain is safer
            // than leaving it active and permanently unretirable.
            let revision = rec.revision.saturating_add(1);
            if revision == rec.revision {
                tracing::error!(
                    domain = %d,
                    "sim: domain revision exhausted; retiring at terminal revision"
                );
            }
            rec.status = DomainStatus::Retired;
            rec.revision = revision;
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
    if let Some(prepared) = prepared_audit {
        let (_, op) = st
            .recovery
            .apply_prepared(prepared)
            .expect("recovery ledger cannot change during one spatial apply");
        push_op(out, op);
    }
    Ok(())
}

/// Apply an already-validated [`Plan`] against a command context.
fn apply_plan(ctx: &mut Ctx<'_>, plan: Plan) -> Result<(), SpatialError> {
    // Take the slot out to satisfy the borrow checker (world + slot are both
    // mutated); put it back before returning.
    let mut st = std::mem::take(
        ctx.w
            .module_state_mut::<SpatialState>()
            .expect("sim state slot installed at registration"),
    );
    let r = apply(&mut st, ctx.w, ctx.out, plan);
    *ctx.w
        .module_state_mut::<SpatialState>()
        .expect("sim state slot installed at registration") = st;
    r
}

/// Plan and apply in one step against a command context.
pub fn run(ctx: &mut Ctx<'_>, t: &Transition) -> Result<(), SpatialError> {
    let plan = {
        let st = ctx
            .w
            .module_state::<SpatialState>()
            .expect("sim state slot installed at registration");
        plan(st, ctx.w, t)?
    };
    apply_plan(ctx, plan)
}

/// **Latch (or release) the quarantine flag on ONE unplaced carrier root**
/// (E7 cut 6 fold, D1) — never its subtree, which keeps riding it.
///
/// This is not a second carrier mover: the root's own record is already
/// `Unplaced` and stays `Unplaced`; only the `quarantined` latch (and therefore
/// [`crate::graph::project`]'s answer) changes. It goes through the same
/// validate-then-apply `Plan` every other transition does, so a refusal mutates
/// nothing and the whole subtree's projections are re-derived together.
///
/// A **stopped** journey latches it (the root lands in `recovery_room`, is
/// `is_quarantined`, and `placed_or_quarantined` covers it, so no bystander can
/// pick a dropship up); `@journey/divert` releases it again as the ship gets
/// under way (back to `in_transit_room`).
///
/// # Errors
///
/// Every error [`plan`]/[`apply`] can raise, plus [`SpatialError::NotPlaced`]
/// when `root` has no placement record at all.
pub(crate) fn set_carrier_root_quarantine(
    ctx: &mut Ctx<'_>,
    root: ObjId,
    quarantined: bool,
    reason: &str,
) -> Result<(), SpatialError> {
    let plan = {
        let st = ctx
            .w
            .module_state::<SpatialState>()
            .expect("sim state slot installed at registration");
        let prev = st
            .placements
            .get(&root)
            .ok_or(SpatialError::NotPlaced(root))?;
        if prev.parent.is_placed() {
            // A placed root needs no holding pen; leave the graph alone.
            return Ok(());
        }
        if prev
            .unplaced
            .as_ref()
            .is_some_and(|u| u.quarantined == quarantined)
        {
            return Ok(()); // already in the requested posture
        }
        let mut unplaced = prev.unplaced.clone().unwrap_or(UnplacedInfo {
            reason: reason.to_string(),
            by: None,
            at: ctx.w.now(),
            previous: None,
            quarantined,
        });
        unplaced.quarantined = quarantined;
        unplaced.reason = reason.to_string();
        let rec = PlacementRecord {
            object: root,
            parent: SpatialParent::Unplaced,
            revision: checked_placement_revision(root, prev.revision)?,
            unplaced: Some(unplaced),
        };
        let mut post = st.clone();
        post.placements.insert(root, rec.clone());
        post.derived = rebuild(&post);
        let mut plan = Plan::default();
        projections_for(&post, ctx.w, root, &mut plan.projections)?;
        plan.audit = serde_json::json!({
            "op": if quarantined { "journey_hold" } else { "journey_release" },
            "root": root.0,
            "reason": reason,
        });
        plan.set_placements.push(rec);
        plan
    };
    apply_plan(ctx, plan)
}

/// **The one checked carrier move** (E7 cut 6, B2/B3).
///
/// Every carrier-root placement change on the tree goes through here —
/// `@journey/depart`, the journey tick's arrival, and the boot re-run of that
/// same tick — so the carrier-root invariant is stated once:
///
/// - **no cycle**: a root can never come to rest inside its own subtree, at any
///   depth (checked over the descendant relation *and* over
///   [`crate::graph::ancestors`], which are duals);
/// - **the subtree is atomic**: only the root's own placement record changes, so
///   every anchored interior, occupant, mount, craft and hold keeps its exact
///   relationship and the whole set's projections are recomputed and
///   pre-validated together — a refusal mutates nothing;
/// - **bounded** on records visited ([`crate::journey::MAX_CARRIER_SUBTREE`]);
/// - **crew control follows**: every actor crewing anything in the subtree has
///   its links re-derived afterwards, which — with `links::controls_are_live` —
///   is what keeps a *cruising* root's crew in command instead of silently
///   dropping every grant mid-journey.
///
/// Returns the subtree that moved (root first), for the caller's conservation
/// assertions.
pub fn move_carrier_subtree(
    ctx: &mut Ctx<'_>,
    root: ObjId,
    parent: Option<SpatialParent>,
) -> Result<Vec<ObjId>, SpatialError> {
    run(ctx, &Transition::CarrierMove { root, parent })?;
    let subtree = ctx
        .w
        .module_state::<SpatialState>()
        .map(|st| st.subtree(root))
        .unwrap_or_default();
    for unit in &subtree {
        crate::links::refresh_unit_actors(ctx.w, *unit);
    }
    Ok(subtree)
}
