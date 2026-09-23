//! **Units a simulation authority holds custody of** (16b cut T.3; doc 16 §3.1
//! "Custody", §3.6 "World-side exclusion").
//!
//! While an arena runs, *it* is the single writer for its units' poses and
//! condition cells and the world task is the single writer for everything else
//! (D-16-1). Nothing enforces that on its own: the world's ordinary verbs would
//! go on moving, damaging and un-stationing a unit the arena is integrating, and
//! the two authorities would disagree without either of them noticing.
//!
//! This module is the world side's half of the answer — **one index, one helper
//! and one named error**:
//!
//! * [`DeployedUnits`] is a RAM index of `unit -> `[`DeployedMark`], owned by
//!   this module and written only through [`crate::deployed::mark`] and [`crate::deployed::clear`].
//! * [`refuse_if_deployed`] is the ONE shared helper every admission site calls.
//! * [`UnitDeployed`] is the ONE named error they all render.
//!
//! **16b cut S.2** added the other half of the same index's job: [`crate::deployed::excluded`], the
//! ONE predicate every *listing* site calls for doc 16 §3.6's world-side exclusion.
//! An admission site refuses a write and says which match holds the unit; a listing
//! site says nothing at all, because a listing that explained its own omission
//! would be the leak it exists to prevent.
//!
//! # Why the index is here and the record is not
//!
//! The durable marker lives on the unit's own record — `mush-tactics`' `unit:<obj>`
//! document carries a `deployed` field holding the check-out snapshot, because
//! that snapshot is a `TacticalUnit` and only the crate that owns units can hold
//! one. But the *admission sites* are spread across `mush-sim` (the `@spatial`
//! moves), `mush-tactics` (the destroy and detach gates, `@unit/damage`,
//! `@unit/remove`) and `mush-campaign` (`@seat/revoke` and the campaign handoff),
//! and `mush-sim` is the only crate all three can see: `mush-tactics` and
//! `mush-campaign` both depend on it, and it depends on neither. A helper any
//! higher would be a helper half the sites could not call, and "one shared
//! helper" would quietly become three.
//!
//! So the index is a **projection**, the way [`crate::seat::ControlLeaseView`] is:
//! the record is authority, this is what the admission sites read, and the two
//! are kept together by the one crate that writes both
//! (`mush_tactics::custody`), including at boot.
//!
//! # What it deliberately does not do
//!
//! It does not decide *who* may deploy a unit, it does not know what a match is,
//! and it holds no snapshot. It answers one question — is this unit under
//! custody, and by which match — and refuses on the answer.

use std::collections::BTreeMap;

use mush_core::{ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::DomainId;

/// **Which authority holds a unit**, as the world-side index and the durable
/// marker both carry it.
///
/// The match id is a plain `u64` rather than the game crate's `MatchId`: this
/// crate is the spatial engine and knows nothing about matches, and the number
/// is here so a refusal can say *which* one rather than only *that* one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployedMark {
    /// The match holding the unit.
    pub match_id: u64,
    /// The arena definition that match is running on.
    pub arena: SmolStr,
}

impl DeployedMark {
    /// A mark for `match_id` on `arena`.
    #[must_use]
    pub fn new(match_id: u64, arena: impl Into<SmolStr>) -> DeployedMark {
        DeployedMark {
            match_id,
            arena: arena.into(),
        }
    }
}

/// **The one named error** every admission site returns for a deployed unit.
///
/// One text, one shape, everywhere: a staffer who tries to move, damage, detach,
/// destroy or un-seat a unit that is in a match gets the same sentence from all
/// six sites, and the sentence names the match so the next thing to try
/// (`@arena/abort <match>`) is obvious.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{unit} is deployed in arena match {match_id} ({arena}); the arena holds it until the match \
     ends."
)]
pub struct UnitDeployed {
    /// The unit under custody.
    pub unit: ObjId,
    /// The match holding it.
    pub match_id: u64,
    /// The arena that match runs on.
    pub arena: SmolStr,
}

/// The world-side index of units under a simulation authority's custody.
///
/// RAM only and deliberately so: the durable fact is the marker on the unit's own
/// record, and this is rebuilt from those markers at boot before any command can
/// run (`mush_tactics`' recovery pass). A restart with this index empty and a
/// marker still on disk is exactly the state boot reconciliation exists to end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeployedUnits {
    by_unit: BTreeMap<ObjId, DeployedMark>,
}

impl DeployedUnits {
    /// An empty index.
    #[must_use]
    pub fn new() -> DeployedUnits {
        DeployedUnits::default()
    }

    /// How many units are under custody.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_unit.len()
    }

    /// Whether nothing is under custody.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_unit.is_empty()
    }

    /// Every unit under custody, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (ObjId, &DeployedMark)> {
        self.by_unit.iter().map(|(unit, mark)| (*unit, mark))
    }
}

/// Record that `unit` is under `mark`'s custody. Replacing an existing mark is
/// the caller's business — a unit cannot be in two matches, and
/// [`mush_tactics`'s check-out](https://docs.rs/mush-tactics) refuses that before
/// it reaches here.
///
/// A no-op when this module is not composed, which is the same answer every
/// other `mush-sim` seam gives an uncomposed world.
pub fn mark(w: &mut World, unit: ObjId, mark: DeployedMark) {
    if let Some(index) = w.module_state_mut::<DeployedUnits>() {
        index.by_unit.insert(unit, mark);
    }
}

/// Forget that `unit` is under custody, handing back the mark it held.
pub fn clear(w: &mut World, unit: ObjId) -> Option<DeployedMark> {
    w.module_state_mut::<DeployedUnits>()?.by_unit.remove(&unit)
}

/// Which match holds `unit`, if any.
#[must_use]
pub fn mark_of(w: &World, unit: ObjId) -> Option<&DeployedMark> {
    w.module_state::<DeployedUnits>()?.by_unit.get(&unit)
}

/// Whether `unit` is under a simulation authority's custody.
#[must_use]
pub fn is_deployed(w: &World, unit: ObjId) -> bool {
    mark_of(w, unit).is_some()
}

/// **The one shared helper** (16b T2): refuse an ordinary mutation of a unit an
/// arena holds.
///
/// Called at every real admission site rather than at one chokepoint, because
/// there is no chokepoint: a placement, a mutation plan, a behaviour detach, an
/// object destruction and a seat revocation are five different transactions with
/// five different callers, and doc 16 §3.1 names all five.
///
/// # Errors
///
/// [`UnitDeployed`], naming the unit, its match and its arena.
pub fn refuse_if_deployed(w: &World, unit: ObjId) -> Result<(), UnitDeployed> {
    match mark_of(w, unit) {
        None => Ok(()),
        Some(mark) => Err(UnitDeployed {
            unit,
            match_id: mark.match_id,
            arena: mark.arena.clone(),
        }),
    }
}

/// **What a world-side listing is about to show**, for [`crate::deployed::excluded`] (16b cut S.2).
///
/// Two kinds, because doc 16 §3.6's exclusion is about two kinds of thing — the
/// deployed unit itself, and the domain it is mustered in — and one enum keeps them
/// answered by one function instead of by one copy per site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// One object a listing, a room render or a status line would name.
    Unit(ObjId),
    /// One domain a window, a map frame or a domain listing would name.
    Domain(DomainId),
}

/// **The one exclusion predicate** (16b cut S.2; doc 16 §3.6 "World-side
/// exclusion": *while an arena holds custody, its domain is excluded from
/// `visible_rooms`, `units_for`, the atlas `+map` frame, `look` and every world
/// unit listing, for every viewer including staff*).
///
/// One function, called at every listing site, so the rule has one definition and
/// a new listing cannot be written that quietly forgets it. It takes no viewer on
/// purpose: the exclusion is **not** a permission check and has no staff bypass —
/// the only view of arena state is the cockpit frame the arena builds per pilot,
/// and a staffer who wants to see a running match uses `@arena/status`.
///
/// * [`Shown::Unit`] — the unit is under an arena's custody. This is
///   [`is_deployed`] read as a *visibility* question rather than as an admission
///   one; the two are the same test and deliberately not the same function name,
///   because the sentences they produce are different ([`UnitDeployed`] names the
///   match, an exclusion names nothing at all).
/// * [`Shown::Domain`] — some deployed unit's frozen world placement is in it,
///   which is exactly *the arena's muster domain while a match runs*. Check-out
///   leaves each unit placed where it stood ([`crate::pose_of`] still answers for
///   it, since only [`crate::participants`] projects it away), so the muster domain
///   identifies itself and nothing has to be told which domain a match belongs to.
///   The walk is over the custody index, which holds at most
///   `match.max_slots_per_side × 2` units per match, and not over the placement
///   graph.
#[must_use]
pub fn excluded(w: &World, shown: Shown) -> bool {
    match shown {
        Shown::Unit(unit) => mark_of(w, unit).is_some(),
        Shown::Domain(domain) => w.module_state::<DeployedUnits>().is_some_and(|index| {
            index
                .by_unit
                .keys()
                .any(|unit| crate::pose_of(w, *unit).is_some_and(|(held, _)| held == domain))
        }),
    }
}

/// Every unit under custody, with its mark, in id order.
#[must_use]
pub fn deployed_units(w: &World) -> Vec<(ObjId, DeployedMark)> {
    w.module_state::<DeployedUnits>()
        .into_iter()
        .flat_map(|index| {
            index
                .by_unit
                .iter()
                .map(|(unit, mark)| (*unit, mark.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unmarked_unit_is_admitted_and_a_marked_one_is_refused_by_name() {
        let mut w = World::new_minimal();
        w.module_state_insert(DeployedUnits::new());
        let unit = ObjId(7);
        assert!(refuse_if_deployed(&w, unit).is_ok());

        mark(
            &mut w,
            unit,
            DeployedMark::new(42, "tacsim.arena.proving_ground"),
        );
        let refusal = refuse_if_deployed(&w, unit).expect_err("a deployed unit refuses");
        assert_eq!(refusal.match_id, 42);
        assert_eq!(refusal.unit, unit);
        let text = refusal.to_string();
        assert!(text.contains("42"), "the refusal names its match: {text}");
        assert!(
            text.contains("proving_ground"),
            "and the arena it is running on: {text}"
        );

        let taken = clear(&mut w, unit).expect("the mark comes back");
        assert_eq!(taken.match_id, 42);
        assert!(refuse_if_deployed(&w, unit).is_ok(), "and the unit is free");
        assert!(clear(&mut w, unit).is_none(), "clearing twice is a no-op");
    }

    #[test]
    fn an_uncomposed_world_admits_everything() {
        let w = World::new_minimal();
        assert!(refuse_if_deployed(&w, ObjId(1)).is_ok());
        assert!(deployed_units(&w).is_empty());
    }
}
