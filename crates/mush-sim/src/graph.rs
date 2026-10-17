//! Pure graph semantics over [`SpatialState`] (design §5.2–§5.3): the ONE
//! ancestor relation used for acyclicity and depth, effective domain,
//! presence, projection (design D1), and the derived-index rebuild/verify.

use std::collections::BTreeSet;

use mush_core::{ObjId, World};

use crate::data::{
    Derived, DomainId, MAX_SPATIAL_DEPTH, Presence, SpaceAnchor, SpatialParent, SpatialState,
};
use crate::transition::SpatialError;

/// The structural parent OBJECT of `obj` per the ancestor relation:
/// Interior → the space; Attached → the host; Direct → the domain host.
#[must_use]
pub fn parent_object(st: &SpatialState, obj: ObjId) -> Option<ObjId> {
    match st.parent_of(obj) {
        SpatialParent::Direct { domain, .. } => st.domains.get(&domain).map(|d| d.host),
        SpatialParent::Interior { space, .. } => Some(space),
        SpatialParent::Attached { host, .. } => Some(host),
        SpatialParent::Unplaced => None,
    }
}

/// The anchor edge of a space: MovingObject → host; FixedDomain → domain host.
#[must_use]
pub fn anchor_object(st: &SpatialState, space: ObjId) -> Option<ObjId> {
    match &st.spaces.get(&space)?.anchor {
        SpaceAnchor::MovingObject { host } => Some(*host),
        SpaceAnchor::FixedDomain { domain, .. } => st.domains.get(domain).map(|d| d.host),
        SpaceAnchor::Unanchored => None,
    }
}

/// The next ancestor of `x`: its placement parent if placed, else — for a
/// space — its anchor edge.
#[must_use]
pub fn next_ancestor(st: &SpatialState, x: ObjId) -> Option<ObjId> {
    if st.is_placed(x) {
        parent_object(st, x)
    } else {
        anchor_object(st, x)
    }
}

/// Every ancestor of `start` (excluding it), nearest first, via
/// [`next_ancestor`]. `Cycle` on a revisit; `TooDeep` past
/// [`MAX_SPATIAL_DEPTH`].
pub fn ancestors(st: &SpatialState, start: ObjId) -> Result<Vec<ObjId>, SpatialError> {
    let mut seen = BTreeSet::from([start]);
    let mut out = Vec::new();
    let mut cur = start;
    while let Some(n) = next_ancestor(st, cur) {
        if !seen.insert(n) {
            return Err(SpatialError::Cycle { obj: start, via: n });
        }
        out.push(n);
        if out.len() > MAX_SPATIAL_DEPTH {
            return Err(SpatialError::TooDeep { obj: start });
        }
        cur = n;
    }
    Ok(out)
}

/// The effective domain of `obj` (design §5.3), bounded.
#[must_use]
pub fn effective_domain(st: &SpatialState, obj: ObjId) -> Option<DomainId> {
    let mut cur = obj;
    for _ in 0..=MAX_SPATIAL_DEPTH {
        match st.parent_of(cur) {
            SpatialParent::Direct { domain, .. } => return Some(domain),
            SpatialParent::Interior { space, .. } => match &st.spaces.get(&space)?.anchor {
                SpaceAnchor::FixedDomain { domain, .. } => return Some(*domain),
                SpaceAnchor::MovingObject { host } => cur = *host,
                SpaceAnchor::Unanchored => return None,
            },
            SpatialParent::Attached { host, .. } => cur = host,
            SpatialParent::Unplaced => {
                if cur == obj {
                    // The object itself is unplaced: no effective domain.
                    return None;
                }
                // An intermediate unplaced SPACE still contributes its anchor
                // edge (a bay riding a carrier that is itself a riding bay).
                match st.spaces.get(&cur).map(|s| s.anchor.clone()) {
                    Some(SpaceAnchor::FixedDomain { domain, .. }) => return Some(domain),
                    Some(SpaceAnchor::MovingObject { host }) if host != cur => cur = host,
                    _ => return None,
                }
            }
        }
    }
    None
}

/// Derived presence (design §5.3): Direct → Independent; Attached → exposed
/// per the mount; Interior → Suppressed; Unplaced → none.
#[must_use]
pub fn presence(st: &SpatialState, obj: ObjId) -> Option<Presence> {
    match st.parent_of(obj) {
        SpatialParent::Direct { .. } => Some(Presence::Independent),
        SpatialParent::Interior { .. } => Some(Presence::Suppressed),
        SpatialParent::Attached { host, mount, .. } => {
            let exposed = st
                .mounts
                .get(&host)
                .and_then(|m| m.mounts.get(&mount))
                .is_some_and(|m| m.exposed);
            Some(if exposed {
                Presence::DerivedExposed
            } else {
                Presence::Suppressed
            })
        }
        SpatialParent::Unplaced => None,
    }
}

/// Where an **unplaced** record projects, if anywhere (E7 cut 6 fold, D2):
/// quarantined → the recovery room; a carrier root the [`JourneyBook`] says is
/// **under way** → the in-transit room; anything else → `None`, i.e. "left
/// where it is".
///
/// The two arms are deliberately distinct: a *stopped* journey quarantines its
/// root (D1) and a *moving* one does not, so staff reading the recovery room
/// see wrecks and not ships in passage.
///
/// [`JourneyBook`]: crate::journey::JourneyBook
#[must_use]
fn unplaced_projection(st: &SpatialState, w: &World, obj: ObjId) -> Option<ObjId> {
    if st.is_quarantined(obj) {
        return Some(st.recovery_room);
    }
    w.module_state::<crate::journey::JourneyBook>()
        .is_some_and(|book| book.under_way(obj))
        .then_some(st.in_transit_room)
}

/// Where `obj`'s `Obj.location` must be for its placement (design D1):
/// Direct → the domain's presentation room; Interior → the occupant's current
/// room when it is a member of the space, else the space (cut IC4); Attached
/// exposed → the host's projected location (falling back to the host's actual
/// location); Attached suppressed → the host itself; Unplaced → the recovery
/// room when quarantined, the in-transit room while under way, else `None`
/// (left where it is).
#[must_use]
pub fn project(st: &SpatialState, w: &World, obj: ObjId) -> Option<ObjId> {
    let mut cur = obj;
    for _ in 0..=MAX_SPATIAL_DEPTH {
        match st.parent_of(cur) {
            SpatialParent::Direct { domain, pose } => {
                let d = st.domains.get(&domain)?;
                // S4: a domain with a window resolves the pose to the room whose
                // footprint holds it (doc 14 §3.4); without a resolver, or off
                // every footprint, the presentation room stands.
                if d.window.is_some()
                    && let Some(room) = w
                        .module_state::<crate::data::RoomResolver>()
                        .and_then(|r| r.resolve.as_ref())
                        .and_then(|resolve| resolve(w, domain, &pose))
                {
                    return Some(room);
                }
                return Some(d.presentation_location);
            }
            SpatialParent::Interior { space, .. } => {
                // Cut IC4 (doc 06 §7.5; doc 18 §4.2 (b)): an occupant standing
                // in one of the space's member rooms stays there, so boot
                // reconcile and every carrier move or re-placement leave a
                // walker where they walked; anyone else (placed from outside,
                // or standing in a room the space no longer lists) falls back
                // to the space object — entrants arrive through the lock.
                // `cur` is the object whose Interior placement is resolved, so
                // an exposed mount riding an occupant follows the occupant.
                if let Some(loc) = w.get(cur).and_then(|o| o.location)
                    && st
                        .spaces
                        .get(&space)
                        .is_some_and(|s| s.rooms.contains(&loc))
                {
                    return Some(loc);
                }
                return Some(space);
            }
            SpatialParent::Attached { host, .. } => {
                if presence(st, cur) == Some(Presence::DerivedExposed) {
                    // Follow the host's projection; if the host has none, its
                    // real location. An unplaced host that still projects
                    // somewhere (quarantined, or under way — D2) is followed
                    // too, so an exposed mount leaves the origin room with the
                    // ship it is bolted to instead of staying behind.
                    if st.is_placed(host) || unplaced_projection(st, w, host).is_some() {
                        cur = host;
                        continue;
                    }
                    return w.get(host).and_then(|o| o.location);
                }
                return Some(host);
            }
            SpatialParent::Unplaced => {
                return unplaced_projection(st, w, cur);
            }
        }
    }
    None
}

/// Rebuild every derived index from the records.
#[must_use]
pub fn rebuild(st: &SpatialState) -> Derived {
    let mut d = Derived::default();
    for dom in st.domains.values() {
        if dom.status != crate::data::DomainStatus::Retired {
            d.domain_by_host.insert(dom.host, dom.id);
            d.presentation_of
                .entry(dom.presentation_location)
                .or_default()
                .insert(dom.id);
        }
    }
    for p in st.placements.values() {
        match &p.parent {
            SpatialParent::Direct { domain, .. } => {
                d.domain_direct.entry(*domain).or_default().insert(p.object);
            }
            SpatialParent::Interior { space, .. } => {
                d.children.entry(*space).or_default().insert(p.object);
            }
            SpatialParent::Attached { host, .. } => {
                d.children.entry(*host).or_default().insert(p.object);
            }
            SpatialParent::Unplaced => {}
        }
    }
    for s in st.spaces.values() {
        match &s.anchor {
            SpaceAnchor::MovingObject { host } => {
                d.anchored_spaces.entry(*host).or_default().insert(s.object);
            }
            SpaceAnchor::FixedDomain { domain, .. } => {
                d.spaces_in_domain
                    .entry(*domain)
                    .or_default()
                    .insert(s.object);
            }
            SpaceAnchor::Unanchored => {}
        }
        // Cut IC4 (doc 18 §4.2 (c)): room → the space listing it, the space
        // object mapping to itself. Key order and first-wins keep this
        // deterministic; `Transition::Members` and boot's member pass make a
        // collision impossible.
        for room in &s.rooms {
            d.member_of.entry(*room).or_insert(s.object);
        }
    }
    // Locality and projection need the children index above, but only the
    // records, so compute them straight from the records.
    for p in st.placements.values() {
        if let Some(dom) = effective_domain(st, p.object) {
            d.local_objects.entry(dom).or_default().insert(p.object);
            if matches!(
                presence(st, p.object),
                Some(Presence::Independent | Presence::DerivedExposed)
            ) {
                d.projected.entry(dom).or_default().insert(p.object);
            }
        }
    }
    for a in st.assignments.values() {
        for (sid, seat) in &a.seats {
            d.assignments_by_actor
                .entry(seat.actor)
                .or_default()
                .insert(a.host);
            d.seats_by_actor
                .entry(seat.actor)
                .or_default()
                .insert((a.host, sid.clone()));
        }
    }
    d
}

/// The domain a space belongs to, read from its anchor (not `effective_domain`,
/// which short-circuits to `None` on the space itself): `FixedDomain` → that
/// domain; `MovingObject` → the host's effective domain (nested carriers); a
/// dangling/`Unanchored` space → `None`.
#[must_use]
pub fn space_domain(st: &SpatialState, space: ObjId) -> Option<DomainId> {
    match &st.spaces.get(&space)?.anchor {
        SpaceAnchor::FixedDomain { domain, .. } => {
            st.domains.contains_key(domain).then_some(*domain)
        }
        SpaceAnchor::MovingObject { host } => effective_domain(st, *host),
        SpaceAnchor::Unanchored => None,
    }
}

/// **Doc 18 cut IC9 (lead ruling L16)** — the interior space aboard a moving
/// object that `room` belongs to: the space whose `rooms` lists `room` (through
/// the derived `member_of` index, cut IC4), or `room` itself when it is a space
/// object, **when that space's anchor is [`SpaceAnchor::MovingObject`]**; `None`
/// for a room of a fixed or unanchored interior and for a room in no space.
///
/// The atlas applier asks it so no authored exit ever leads into (or out of) a
/// room aboard a hull that flies (§3-F's destination seal stays unbuilt,
/// D-18-22). World-only and bounded: two map lookups.
#[must_use]
pub fn anchored_space_of(w: &World, room: ObjId) -> Option<ObjId> {
    let st = w.module_state::<SpatialState>()?;
    let space = st
        .derived
        .member_of
        .get(&room)
        .copied()
        .or_else(|| st.spaces.contains_key(&room).then_some(room))?;
    match st.spaces.get(&space)?.anchor {
        SpaceAnchor::MovingObject { .. } => Some(space),
        SpaceAnchor::FixedDomain { .. } | SpaceAnchor::Unanchored => None,
    }
}

/// Recompute the indexes and compare to the maintained ones.
pub fn verify_indexes(st: &SpatialState) -> Result<(), String> {
    let fresh = rebuild(st);
    if fresh == st.derived {
        Ok(())
    } else {
        Err(format!(
            "derived indexes drifted from the records:\n maintained: {:?}\n fresh: {:?}",
            st.derived, fresh
        ))
    }
}

/// Every placed or quarantined object whose `Obj.location` disagrees with its
/// projection: `(object, expected, actual)`.
#[must_use]
pub fn verify_projections(st: &SpatialState, w: &World) -> Vec<(ObjId, ObjId, Option<ObjId>)> {
    let mut bad = Vec::new();
    for &obj in st.placements.keys() {
        if let Some(expected) = project(st, w, obj) {
            let actual = w.get(obj).and_then(|o| o.location);
            if actual != Some(expected) {
                bad.push((obj, expected, actual));
            }
        }
    }
    bad
}
