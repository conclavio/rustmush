//! **Handing a unit to a simulation authority, and taking it back** (16b cut
//! T.3; doc 16 §3.1 "Custody", §3.3 "The stock tanks are persistent objects",
//! §3.8 "Lifecycle").
//!
//! While a match runs the arena is the single writer for its units' poses and
//! condition cells, and the world task is the single writer for everything else
//! (D-16-1). The two share no state and no lock, so the only thing that can keep
//! them from disagreeing is a **marker on the unit's own record** saying that it
//! is out on loan — and a rule that every ordinary mutation refuses while the
//! marker is there.
//!
//! This module is both halves of the marker:
//!
//! * [`crate::custody::check_out`] writes `deployed { match, arena, snapshot }` onto the unit's
//!   `unit:<obj>` document in **one** write, and publishes the unit into
//!   [`mush_sim::deployed`], the world-side index every admission site reads.
//! * [`crate::custody::check_in`] takes it back: one [`UnitMutationPlan`] of whatever the arena
//!   returned through [`crate::unit::apply`], or — on `Void` — the check-out
//!   snapshot restored and **no arena-derived cell committed at all**; then the
//!   marker is cleared, in the same single write.
//! * **At boot** (16b cut AF.3) the markers are **kept**, because a running
//!   match survives a restart of the server half: the tactics boot pass leaves
//!   them on the records and in [`mush_sim::deployed`] and hands the list to the
//!   game crate ([`crate::custody::take_boot_deployed`]), which resumes the
//!   match or gives each unit back through [`crate::custody::restore_at_boot`];
//!   whatever it does not claim is restored at this crate's `on_all_loaded`.
//!
//! # Why the snapshot is a whole unit
//!
//! Doc 16 §3.1: *`snapshot` is the unit's cells and pose at check-out*, and on a
//! void *the world restores each unit from the check-out snapshot*. A persisted
//! unit points at exact definition versions (M32), so the honest snapshot is the
//! `TacticalUnit` itself rather than a cell list: restoring it cannot land a cell
//! value on a structure that has changed underneath, because the structure is
//! part of what was saved. It costs one unit's worth of bytes inside a record
//! that already holds one, for the length of one match.
//!
//! # Why the pose is in the marker and not only in the placement graph
//!
//! The world placement is left exactly where it was and frozen — the pilot's
//! channel grants require a placed unit, and revoking them for the length of the
//! match would take the cockpit away from the person sitting in it. But "frozen"
//! is a rule, not a lock: the pose in the marker is what a void restores *to*,
//! so a restart that lands between a placement write and a crash still returns
//! the unit to where it was when the arena took it.
//!
//! # What this module is not
//!
//! It is not the lifecycle. It knows nothing about teams, slots, boarding,
//! results or ledgers; the game crate (`tacsim::match_`) owns all of that and
//! calls these two functions once per unit. What lives here is what only the
//! crate that owns `TacticalUnit` can do.

use mush_core::{Ctx, ObjId, World};
use mush_sim::data::{DomainId, Pose};
use mush_sim::deployed::DeployedMark;
use serde::{Deserialize, Serialize};

use crate::unit::{Journal, TacticalUnit, UnitMutationPlan};

/// **The marker one deployed unit carries**, on its record and in RAM.
///
/// Serialized as the `deployed` field of the `unit:<obj>` document, which is the
/// durable half; [`mush_sim::deployed`] holds the [`DeployedMark`] out of it as
/// the world-side index the admission sites read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployedMarker {
    /// Which match holds the unit, and on which arena.
    pub mark: DeployedMark,
    /// The domain the unit was placed in at check-out, when it was placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<DomainId>,
    /// The pose it was placed at, when it was placed. A void restores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<Pose>,
    /// The unit's cells at check-out, whole. A void restores it.
    pub snapshot: TacticalUnit,
}

impl DeployedMarker {
    /// The match this marker names.
    #[must_use]
    pub const fn match_id(&self) -> u64 {
        self.mark.match_id
    }
}

/// Why a check-out or a check-in refused. Every variant is rendered by the
/// caller; none of them panics, because both run on the world task.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    /// The object has no active tactical unit record.
    #[error("{0} is not a tactical unit")]
    NotAUnit(ObjId),
    /// The tactics module is not composed, or its record schema is unavailable.
    #[error("the tactics unit layer is not available on this server")]
    Unavailable,
    /// The unit is already out on loan to another match.
    #[error("{unit} is already deployed in match {match_id}")]
    AlreadyDeployed {
        /// The unit.
        unit: ObjId,
        /// The match already holding it.
        match_id: u64,
    },
    /// The unit carries no `deployed` marker, so there is nothing to check in.
    #[error("{0} is not deployed")]
    NotDeployed(ObjId),
    /// The returned cells were refused by [`crate::unit::apply`] — refuse, never
    /// clamp (M32). Nothing was written.
    #[error("the returned cells were refused: {0}")]
    Refused(String),
    /// The record would not encode inside its canonical limit. Nothing was
    /// written.
    #[error("the unit record would not encode: {0}")]
    Encode(String),
}

/// What one check-in did, for the result record and the journal line.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckIn {
    /// The marker the unit was carrying.
    pub marker: DeployedMarker,
    /// The journal of the returned plan, when one was applied.
    pub journal: Option<Journal>,
    /// Whether the check-out snapshot was restored instead (a `Void`).
    pub restored: bool,
    /// The unit as it now stands.
    pub unit: TacticalUnit,
}

/// **Check one unit out to a simulation authority** (doc 16 §3.1 "Custody",
/// check-out).
///
/// One write: the unit's record gains its `deployed` marker, carrying the
/// snapshot a void will restore. The unit's cells are **not** touched, its world
/// placement is **not** touched (it is left frozen, deliberately), and its
/// revision does **not** move — a marker is not a mutation of the unit, it is a
/// statement about who may mutate it, which is exactly why the T6 gate can say
/// *the custody markers are the only unit writes during a match*.
///
/// The caller is expected to have already refused everything it means to refuse;
/// this refuses the two things only the record knows.
///
/// # Errors
///
/// [`CustodyError`]: not a unit, already deployed, no record schema, or a record
/// that will not encode. Nothing is written on any of them.
pub fn check_out(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    mark: DeployedMark,
) -> Result<DeployedMarker, CustodyError> {
    let (cells, schema) = {
        let store = ctx
            .w
            .module_state::<crate::module::UnitStore>()
            .ok_or(CustodyError::Unavailable)?;
        if let Some(existing) = store.deployed.get(&unit) {
            return Err(CustodyError::AlreadyDeployed {
                unit,
                match_id: existing.match_id(),
            });
        }
        let cells = store
            .units
            .get(&unit)
            .cloned()
            .ok_or(CustodyError::NotAUnit(unit))?;
        let schema = store
            .record_definition
            .clone()
            .ok_or(CustodyError::Unavailable)?;
        (cells, schema)
    };
    let placement = mush_sim::pose_of(ctx.w, unit);
    let marker = DeployedMarker {
        mark,
        domain: placement.as_ref().map(|(domain, _)| *domain),
        pose: placement.map(|(_, pose)| pose),
        snapshot: cells.clone(),
    };
    let document = crate::module::encode_unit_document(&schema, &cells, Some(&marker))
        .map_err(CustodyError::Encode)?;
    crate::module::persist_unit_document(ctx, unit, document.0);
    if let Some(store) = ctx.w.module_state_mut::<crate::module::UnitStore>() {
        store.set_deployed(unit, Some(marker.clone()), Some(document.1));
    }
    mush_sim::deployed::mark(ctx.w, unit, marker.mark.clone());
    Ok(marker)
}

/// **Take one unit back** (doc 16 §3.1 "Custody", check-in).
///
/// `plan` is what the arena returned: `Some(plan)` applies it through
/// [`crate::unit::apply`] — one plan, atomically, refuse-never-clamp — and
/// `None` is the `Void` path, which restores the check-out snapshot and commits
/// **no arena-derived cell at all**.
///
/// One write either way: the cells and the cleared marker go into the record
/// together, so a crash between them is not a state this code can produce.
///
/// The world placement is the caller's: `mush_sim::place` at the arena's final
/// pose on an ordinary end, the marker's own pose on a void. Both belong in the
/// same authoritative action as this call, and the caller is the one holding it.
///
/// # Errors
///
/// [`CustodyError`]: not deployed, no record schema, a refused plan, or a record
/// that will not encode. Nothing is written on any of them.
pub fn check_in(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    plan: Option<&UnitMutationPlan>,
) -> Result<CheckIn, CustodyError> {
    let (marker, current, schema) = {
        let store = ctx
            .w
            .module_state::<crate::module::UnitStore>()
            .ok_or(CustodyError::Unavailable)?;
        let marker = store
            .deployed
            .get(&unit)
            .cloned()
            .ok_or(CustodyError::NotDeployed(unit))?;
        let current = store
            .units
            .get(&unit)
            .cloned()
            .ok_or(CustodyError::NotAUnit(unit))?;
        let schema = store
            .record_definition
            .clone()
            .ok_or(CustodyError::Unavailable)?;
        (marker, current, schema)
    };
    let now = ctx.w.now();
    let (mut post, journal, restored) = match plan {
        Some(plan) => {
            let (post, journal) = crate::unit::apply(&current, plan, now)
                .map_err(|error| CustodyError::Refused(error.to_string()))?;
            (post, Some(journal), false)
        }
        // Doc 16 §3.1 "Failure": a void returns nothing. The snapshot is the
        // unit as the world last knew it, so restoring it is the one commit that
        // carries no arena state.
        None => (marker.snapshot.clone(), None, true),
    };
    post.settle_mobility(now);
    let document =
        crate::module::encode_unit_document(&schema, &post, None).map_err(CustodyError::Encode)?;
    crate::module::persist_unit_document(ctx, unit, document.0);
    if let Some(store) = ctx.w.module_state_mut::<crate::module::UnitStore>() {
        store.set_deployed(unit, None, None);
        store.replace_active(post.clone(), document.1);
    }
    mush_sim::deployed::clear(ctx.w, unit);
    if !restored {
        ctx.emit_world_event(&mush_core::WorldEvent::TacticalUnitChanged {
            unit,
            revision: post.revision(),
            responsible: None,
        });
        ctx.out.pin_deferred();
    }
    Ok(CheckIn {
        marker,
        journal,
        restored,
        unit: post,
    })
}

/// The marker `unit` is carrying, if any.
#[must_use]
pub fn marker_of(w: &World, unit: ObjId) -> Option<&DeployedMarker> {
    w.module_state::<crate::module::UnitStore>()?
        .deployed
        .get(&unit)
}

/// Every unit under custody, with its marker, in id order.
#[must_use]
pub fn deployed(w: &World) -> Vec<(ObjId, DeployedMarker)> {
    w.module_state::<crate::module::UnitStore>()
        .into_iter()
        .flat_map(|store| {
            store
                .deployed
                .iter()
                .map(|(unit, marker)| (*unit, marker.clone()))
        })
        .collect()
}

/// **The markers this boot found** (16b T5; rewritten by 16b cut AF.3), taken
/// once.
///
/// Since AF.3 a running match survives a restart of the server half, so the
/// tactics boot pass no longer restores a marked unit: it keeps the marker on
/// the record, in [`mush_sim::deployed`] and on disk, and leaves the list here.
/// The game crate takes them in its own `on_world_loaded` — which composition
/// runs after this crate's and before any `on_all_loaded`, the **claim window**
/// — and either resumes the match they name or restores them through
/// [`restore_at_boot`]; whatever is still here at this module's
/// `on_all_loaded` is restored then, so a world with no game layer never keeps
/// a marker past its boot.
///
/// Taken rather than read, so the second caller gets nothing and a match cannot
/// be claimed twice.
#[must_use]
pub fn take_boot_deployed(w: &mut World) -> Vec<(ObjId, DeployedMarker)> {
    w.module_state_mut::<crate::module::UnitStore>()
        .map(|store| std::mem::take(&mut store.boot_deployed))
        .unwrap_or_default()
}

/// **Restore one unit a boot found checked out and nobody resumed** (16b cut
/// AF.3; the per-unit form of what the T5 boot pass did to every marker).
///
/// The marker's check-out snapshot goes into the active record, the marker is
/// cleared in RAM and in [`mush_sim::deployed`], and the marker-less document is
/// persisted in one write on this crate's namespace (the game crate's boot
/// crosses into it through its declared delegation, as a check-in does). The
/// world placement is not touched: the game crate's stock-tank reset puts a
/// roster tank back at muster, and a staff unit keeps the placement it was
/// frozen at.
///
/// # Errors
///
/// [`CustodyError::NotDeployed`] when the unit carries no marker,
/// [`CustodyError::Unavailable`] with no record schema, and
/// [`CustodyError::Encode`] when the snapshot will not encode — the marker then
/// stays, and the next boot tries again. Nothing is written on any of them.
pub fn restore_at_boot(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CustodyError> {
    let (marker, schema) = {
        let store = ctx
            .w
            .module_state::<crate::module::UnitStore>()
            .ok_or(CustodyError::Unavailable)?;
        let marker = store
            .deployed
            .get(&unit)
            .cloned()
            .ok_or(CustodyError::NotDeployed(unit))?;
        let schema = store
            .record_definition
            .clone()
            .ok_or(CustodyError::Unavailable)?;
        (marker, schema)
    };
    let (document, digest) = crate::module::encode_unit_document(&schema, &marker.snapshot, None)
        .map_err(CustodyError::Encode)?;
    tracing::warn!(
        %unit,
        match_id = marker.match_id(),
        arena = %marker.mark.arena,
        "tactics: a unit was still checked out to an arena at boot and its match did not \
         resume; the check-out snapshot is restored and the marker cleared"
    );
    if let Some(store) = ctx.w.module_state_mut::<crate::module::UnitStore>() {
        store.replace_active(marker.snapshot.clone(), digest);
        store.set_deployed(unit, None, None);
    }
    mush_sim::deployed::clear(ctx.w, unit);
    crate::module::persist_unit_document(ctx, unit, document);
    Ok(())
}
