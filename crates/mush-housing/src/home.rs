//! The `Obj.home` invariant helpers (E5a, 08d §3.3, **D-E5-11**).
//!
//! Core's destroy dispersal moves a destroyed room's contents to each object's
//! `Obj.home` through a raw `move_to` that honours **no hook**
//! (`mush-core/src/world.rs`). So housing's access predicate — which is only a hook —
//! is silent exactly when it matters most: destroy a room an evicted tenant is
//! standing in and they land wherever `Obj.home` still points, which is the home they
//! no longer hold.
//!
//! The fix is not a second hook, it is an invariant: **`Obj.home` follows the tenancy
//! 1:1.** Housing sets it on `Active` and clears it on `Ended`, and these two pure
//! functions say exactly which objects each of those touches. E5b applies them; the
//! library computes them so the rule is testable without a world.

use std::collections::BTreeSet;

use mush_core::ObjId;

use crate::record::Tenancy;

/// Every object whose `Obj.home` should be pointed at the property's anchor when the
/// tenancy goes `Active`: its tenants, and only its tenants.
///
/// Guests are **not** homed — a guest has an invitation, not a residence, and homing
/// them would silently make somebody else's home their destroy-dispersal destination.
#[must_use]
pub fn homed_on_activate(t: &Tenancy) -> BTreeSet<ObjId> {
    t.tenants.clone()
}

/// Every object whose `Obj.home` should be **cleared** when the tenancy ends.
///
/// The union of two sets:
///
/// - everything housing recorded in `homed` — the tenants it pointed at this
///   property, including any since dropped from the roster;
/// - every current tenant or guest whose home *resolves inside* the property,
///   because a guest or an item can be homed on a room without housing having done
///   it.
///
/// **One safety carve-out on the first set** (a deliberate narrowing of §3.3's "every
/// object in `homed`"): an id whose home now resolves *outside* the property is left
/// alone. Somebody else owns that pointer — most obviously a tenant who has already
/// taken up a second tenancy elsewhere — and clearing it would break *that* tenancy's
/// invariant to enforce this one. An id whose home resolves to nothing is included and
/// harmlessly re-cleared.
///
/// **"Inside" is `anchor` or `property_rooms`, and the anchor counts whether or not it
/// is in the set.** `property_rooms` is a set E5b builds from the world at ending time,
/// and a room that has been destroyed or is `GOING` drops out of it — but the anchor is
/// precisely where every tenant's `Obj.home` was pointed, so a sweep that could not see
/// it would leave behind exactly the pointers this invariant exists to clear, in the
/// one case (a property whose rooms are disappearing) where it matters most.
///
/// `resolve_home` is `|obj| world.get(obj).and_then(|o| o.home)`; a destroyed or
/// unreadable object answers `None`.
///
/// This covers the tenancy's own roster. Items a departing tenant homed inside the
/// property are found with [`homed_inside`] over the property's contents, which is a
/// world walk only E5b can do.
///
/// **Handoff (E5b).** Clearing is per *object*, and an object can be a tenant of a
/// second live tenancy elsewhere: the carve-out above catches the common case (their
/// home already points at the other property), but somebody homed *here* who is also
/// on another live lease is unhomed by this sweep and left homed on nothing. After
/// applying the set, E5b re-homes anybody who is still a tenant of another live
/// tenancy onto that tenancy's anchor, in the same breath — the invariant is
/// "`Obj.home` follows *a* live tenancy", not "the one that just ended".
pub fn to_unhome_on_end(
    t: &Tenancy,
    resolve_home: impl Fn(ObjId) -> Option<ObjId>,
    anchor: ObjId,
    property_rooms: &BTreeSet<ObjId>,
) -> BTreeSet<ObjId> {
    let inside = |h: ObjId| h == anchor || property_rooms.contains(&h);
    let mut out = BTreeSet::new();
    for who in &t.homed {
        match resolve_home(*who) {
            Some(h) if !inside(h) => {}
            _ => {
                out.insert(*who);
            }
        }
    }
    for who in t.tenants.iter().chain(t.guests.iter()) {
        if let Some(h) = resolve_home(*who)
            && inside(h)
        {
            out.insert(*who);
        }
    }
    out
}

/// Every candidate whose `Obj.home` resolves inside the property — the general form of
/// [`to_unhome_on_end`]'s second half, and it treats `anchor` as inside for the same
/// reason.
///
/// E5b feeds it the property's contents at ending time, so an item a departing tenant
/// homed in the spare room is unhomed with them and disperses to Limbo rather than
/// back into somebody else's house.
pub fn homed_inside(
    candidates: impl IntoIterator<Item = ObjId>,
    resolve_home: impl Fn(ObjId) -> Option<ObjId>,
    anchor: ObjId,
    property_rooms: &BTreeSet<ObjId>,
) -> BTreeSet<ObjId> {
    candidates
        .into_iter()
        .filter(|c| {
            matches!(resolve_home(*c), Some(h) if h == anchor || property_rooms.contains(&h))
        })
        .collect()
}
