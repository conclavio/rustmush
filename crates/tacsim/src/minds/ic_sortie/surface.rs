//! **The surface host's claim transitions** (doc 23 cut S.4b-2; §17.3, D-23-13, D-23-21,
//! D-23-23). The production producer of [`HostTransition`]: the surface tick's claim arm, and
//! nobody else, mints a [`StartPermit`] for a rig whose `RigRecord.claim` is `Opening` (the rig
//! in its bay, the epoch's IC id fixed at `go`) and a [`CloseEvent`] for one whose cursor is
//! `Closing` (the kind the finish wrote). Each permit is minted from the durable record on the
//! fire that submits it, so a reboot re-owes it from the record (the RAM inbox is empty at boot
//! by design, `boot.rs`). Reading the book ([`claim_of`]) is how every caller observes a
//! transition: never by request id.
use super::participant::{CloseEvent, HostTransition, StartPermit};
use super::records::{EventKind, bad};
use super::state::IcSorties;
use crate::arena::mind::SortieId;
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, ObjId, World};
use mush_tactics::UnitMutationPlan;
use std::sync::Arc;

/// **What the claim book holds for a unit** (doc 23 cut S.4b-2), as the surface host observes
/// it: the active Open, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimRead {
    /// An active Open: its token and its dispatcher.
    Active {
        /// The Open's commit.
        open: CommitToken,
        /// Who dispatched it.
        dispatcher: ObjId,
    },
    /// No active Open names the unit.
    None,
}

/// **The book's active claim for `unit`** (`IcSorties.active`), `None` where the claim path is
/// not composed or holds none.
#[must_use]
pub fn claim_of(world: &World, unit: ObjId) -> ClaimRead {
    let Some(state) = world.module_state::<IcSorties>() else {
        return ClaimRead::None;
    };
    state
        .active
        .get(&unit)
        .and_then(|open| state.rows.get(open))
        .filter(|row| row.closed.is_none())
        .map_or(ClaimRead::None, |row| ClaimRead::Active {
            open: row.open.operation,
            dispatcher: row.open.dispatcher,
        })
}

/// **The claim's retained seal** (D-23-14: the Open's capture is the one seal): what the
/// queued launch's start prints on the open line, `None` when the book holds no such Open.
#[must_use]
pub fn claim_seal(world: &World, open: CommitToken) -> Option<crate::arena::MindSeal> {
    world
        .module_state::<IcSorties>()?
        .rows
        .get(&open)
        .map(|row| row.open.execution.seal.clone())
}

/// **The active claim on `unit`, with what it retains** (doc 44 unit MW.13): its open token,
/// its dispatcher, its seal and its source, borrowed from the book. What a host that thinks
/// with a claimed mind outside an arena reads (`crate::port::ship_mind::sealed`); `None`
/// where the claim path is not composed or the unit has no active claim.
#[must_use]
pub fn claim_execution(
    world: &World,
    unit: ObjId,
) -> Option<(CommitToken, ObjId, &crate::arena::MindSeal, &str)> {
    let state = world.module_state::<IcSorties>()?;
    state
        .active
        .get(&unit)
        .and_then(|open| state.rows.get(open))
        .filter(|row| row.closed.is_none())
        .map(|row| {
            (
                row.open.operation,
                row.open.dispatcher,
                &row.open.execution.seal,
                row.open.execution.source.as_str(),
            )
        })
}

/// **An active claim's retained source by its digest** (doc 23 cut S.4b-2, §17.10's `sources`
/// seam): the text a field's arena installs the claim's runner from, which the store need not
/// hold (the claim retains its source whole, D-23-14).
#[must_use]
pub fn retained_source<'w>(world: &'w World, sha256: &[u8; 32]) -> Option<&'w str> {
    let state = world.module_state::<IcSorties>()?;
    state
        .active
        .values()
        .filter_map(|open| state.rows.get(open))
        .find(|row| row.open.execution.seal.source_sha256 == *sha256)
        .map(|row| row.open.execution.source.as_str())
}

/// **How many active claims the book holds under the filed `licence`** (doc 23 cut S.4b-3,
/// D-23-16: the licence seat): every active Open whose seal names that licence.
#[must_use]
pub fn active_under_licence(world: &World, licence: u64) -> usize {
    let Some(state) = world.module_state::<IcSorties>() else {
        return 0;
    };
    let word = licence.to_string();
    state
        .active
        .values()
        .filter_map(|open| state.rows.get(open))
        .filter(|row| {
            matches!(
                &row.open.execution.seal.requested.origin,
                crate::arena::profile_seal::MindOrigin::Filed { licence, .. } if licence.as_str() == word
            )
        })
        .count()
}

/// **Whether the book holds `open` closed** (the Close's seam, §17.3: `rows[open].closed`).
#[must_use]
pub fn claim_closed(world: &World, open: CommitToken) -> Option<CommitToken> {
    world
        .module_state::<IcSorties>()?
        .rows
        .get(&open)?
        .closed
        .as_ref()
        .map(|closed| closed.operation)
}

/// **The surface host's Open permit** (D-23-13, D-23-14, D-23-21): the rig's live unit as it
/// stands in its bay, the operator, and the epoch's IC id `SortieId::ic(tv, launch_ic)` fixed
/// at `go`.
///
/// **Doc 23 cut S.4d (D-23-25)**: Sim admits a unit lying directly on a domain as a berth
/// hold where berths are configured. Only a ship docked at a hangar port has a berth, so a
/// unit placed on a domain anywhere else (a rig stood on a site's ground, a ship on a site's
/// pad) is given no permit here: its claim is not asked, where before the cut Sim refused it
/// as "not in a configured base". Both arms mint through this one function.
///
/// # Errors
///
/// The rig has no tactical unit, or it lies on a domain that is not a hangar port's berth.
pub(crate) fn surface_start(
    world: &World,
    operator: ObjId,
    tv: ObjId,
    launch_ic: i64,
) -> Result<Arc<StartPermit>, ParticipantError> {
    if on_a_domain_without_a_berth(world, tv) {
        return Err(bad("a claim opens in a bay or at a hangar port's berth"));
    }
    Ok(Arc::new(StartPermit {
        unit: mush_tactics::module::unit_of(world, tv)
            .ok_or_else(|| bad("surface unit"))?
            .clone(),
        actor: operator,
        sortie: Some(SortieId::ic(tv, launch_ic)),
        _transition: HostTransition::surface(),
    }))
}

/// Whether `unit` lies directly on a domain that is not a hangar port's berth
/// ([`surface_start`]).
fn on_a_domain_without_a_berth(world: &World, unit: ObjId) -> bool {
    world
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|spatial| {
            matches!(
                spatial.parent_of(unit),
                mush_sim::SpatialParent::Direct { .. }
            )
        })
        && crate::port::ship::hangar_port_of(world, unit).is_none()
}

/// **The four close kinds as the host event carries them**: `ReturnedToBase`,
/// `PoweredDownInBase` and `RecoveredToBase` name their base, `Destroyed` none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceClose {
    /// `+tv/recover`'s `Done`, staff `home` into its own hull.
    ReturnedToBase(ObjId),
    /// A launch cancelled after its Open, or a ramp that landed nobody.
    PoweredDownInBase(ObjId),
    /// The rig's hull is irrecoverable.
    Destroyed,
    /// A second carrier's completion or a salvage load.
    RecoveredToBase(ObjId),
}

/// **The surface host's Close event** (D-23-13, D-23-15): over the claim's Open as the book
/// holds it, the hull as it stands now, an empty condition plan fenced on its live revision
/// (the hosts applied every physical change already), the close ordinal 1 and the world's IC
/// instant.
///
/// # Errors
///
/// The Open is not in the book, is closed, or the unit is gone.
pub(crate) fn surface_close(
    world: &World,
    operator: ObjId,
    open: CommitToken,
    kind: SurfaceClose,
) -> Result<Arc<CloseEvent>, ParticipantError> {
    let state = world
        .module_state::<IcSorties>()
        .ok_or_else(|| bad("claim book"))?;
    let record = state.rows.get(&open).ok_or_else(|| bad("surface open"))?;
    if record.closed.is_some() {
        return Err(bad("surface closed"));
    }
    let unit = mush_tactics::module::unit_of(world, record.open.unit.object())
        .ok_or_else(|| bad("surface unit"))?
        .clone();
    let kind = match kind {
        SurfaceClose::ReturnedToBase(base) => EventKind::ReturnedToBase { base },
        SurfaceClose::PoweredDownInBase(base) => EventKind::PoweredDownInBase { base },
        SurfaceClose::Destroyed => EventKind::Destroyed {},
        SurfaceClose::RecoveredToBase(base) => EventKind::RecoveredToBase { base },
    };
    Ok(Arc::new(CloseEvent::surface(
        operator,
        open,
        unit.clone(),
        record.open.execution.digest(),
        kind,
        world.world_ic_now(),
        UnitMutationPlan::new(
            vec![],
            Some(unit.revision()),
            Some("ic surface close: the hosts applied the hull's changes".into()),
        )
        .map_err(bad)?,
    )))
}
