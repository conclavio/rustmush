//! Doc 17h §3 M8h (the lead's rulings A2 and A3): the Roll read behind
//! tacsim's `tacsim.house_attestation` tick, folded into
//! `holding_reader::OriginalOwnerReader`'s command kinds beside M8f's two
//! procurement kinds (`procurement_read`), and GOD only as they are: a tick
//! runs as GOD, every other caller is `Refused` before anything is read, and
//! nothing it answers reaches a player surface.
//!
//! `HouseAttestationDue { after }` walks the owner licences in licence order
//! after `after`, at most [`HOUSE_LICENCES_PER_READ`] of them, and answers
//! those born of a House filing (`InitialFilingOperation::economic.house`)
//! whose current owner generation is DUE by the House provider's own rule
//! (`owner_attestation::provider`: the term has ended and Grace has not, so
//! the generation is in `TermPhase::Grace`). Each due row names the House and
//! the House's scope (the scope the provider asks `roll.holder.org` on), the
//! current generation, and the two refusals the tick must not retry:
//! - `unattestable`: an ungoverned seal mark inside the attestation window
//!   (`seal_marks::attestation_window`, the ONE window the provider and the
//!   `licence.unattestable` fact read; M8c B5, D-UM-42);
//! - `revoked`: the Registry revoked the generation
//!   (`registry_revocation::current::require`, asked as the provider asks it).
//!
//! The answer carries no digest, account, rating, source or revision.
use super::families::CommerceFamilies;
use super::read::CommerceRead;
use super::records::{OwnerLicence, Record};
use super::{LicenceId, TermPhase, WorldIcSeconds};
use crate::RollStore;
use mush_core::GOD;
use mush_core::read_service::{OwnerReadContext, OwnerReadError};
use serde::Serialize;

/// Owner licences one `HouseAttestationDue` read walks at most; a House one
/// costs its birth, its current generation, its seal marks and its
/// revocation mark, any other one its birth alone.
pub const HOUSE_LICENCES_PER_READ: usize = 32;

/// One House owner licence whose current generation is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HouseAttestationDue {
    licence: LicenceId,
    organization: mush_org::OrgId,
    scope: mush_org::OrgScopeId,
    generation: u64,
    unattestable: bool,
    revoked: bool,
}
impl HouseAttestationDue {
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    /// The House the filing was made for.
    #[must_use]
    pub const fn organization(&self) -> mush_org::OrgId {
        self.organization
    }
    /// The House's scope, where its attesting character's `roll.holder.org`
    /// grant is asked.
    #[must_use]
    pub const fn scope(&self) -> mush_org::OrgScopeId {
        self.scope
    }
    /// The current (due) owner generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// An ungoverned seal mark inside the term: the provider refuses it, and
    /// the generation lapses (M8c B5).
    #[must_use]
    pub const fn unattestable(&self) -> bool {
        self.unattestable
    }
    /// The Registry revoked the generation: the provider refuses it.
    #[must_use]
    pub const fn revoked(&self) -> bool {
        self.revoked
    }
}

/// One `HouseAttestationDue` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HouseAttestationDuePage {
    rows: Vec<HouseAttestationDue>,
    next: Option<LicenceId>,
}
impl HouseAttestationDuePage {
    #[must_use]
    pub fn rows(&self) -> &[HouseAttestationDue] {
        &self.rows
    }
    /// The licence the next read starts after; `None` once every owner
    /// licence has been walked.
    #[must_use]
    pub const fn next(&self) -> Option<LicenceId> {
        self.next
    }
}

/// `HouseAttestationDue` (the module note).
pub(super) fn due(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    families: &CommerceFamilies,
    after: Option<LicenceId>,
) -> Result<HouseAttestationDuePage, OwnerReadError> {
    if ctx.actor() != GOD {
        return Err(OwnerReadError::Refused);
    }
    let start = after.map(|licence| licence.key());
    let scanned = ctx.scan_action_state(
        &families.licences,
        start.as_deref(),
        HOUSE_LICENCES_PER_READ.saturating_add(1),
    )?;
    let walked: Vec<OwnerLicence> = scanned
        .entries()
        .iter()
        .take(HOUSE_LICENCES_PER_READ)
        .map(|entry| entry.value().clone())
        .collect();
    let more = scanned.entries().len() > HOUSE_LICENCES_PER_READ;
    let now = WorldIcSeconds::from_stamp(&ctx.world().world_ic_now())
        .map_err(|_| OwnerReadError::Unavailable)?;
    let mut rows = Vec::new();
    for licence in &walked {
        if licence.validate().is_err() {
            return Err(OwnerReadError::Integrity);
        }
        if let Some(row) = house_due(ctx, roll, families, licence, now)? {
            rows.push(row);
        }
    }
    let next = if more {
        walked.last().map(|licence| licence.id)
    } else {
        None
    };
    Ok(HouseAttestationDuePage { rows, next })
}

/// One owner licence's due row: `None` for a personal birth or a generation
/// not in Grace.
fn house_due(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    families: &CommerceFamilies,
    licence: &OwnerLicence,
    now: WorldIcSeconds,
) -> Result<Option<HouseAttestationDue>, OwnerReadError> {
    let Some(birth) = ctx
        .get(&families.operations, &licence.operation.to_hex())
        .map_err(|_| OwnerReadError::Unavailable)?
    else {
        return Err(OwnerReadError::Integrity);
    };
    let Some(house) = birth.economic.house.as_ref() else {
        return Ok(None);
    };
    let (organization, scope) = (house.organization(), house.scope());
    let term = super::owner_attestation::history::current(ctx, roll, families, licence)
        .map_err(|_| OwnerReadError::Integrity)?;
    if term.window.phase_at(now) != TermPhase::Grace {
        return Ok(None);
    }
    let (from, to) = super::seal_marks::attestation_window(term.window, now);
    let unattestable = super::seal_marks::marks_in(ctx, families, licence.id, from, to)
        .map_err(|_| OwnerReadError::Integrity)?
        .iter()
        .any(|mark| !mark.governed);
    let revoked = super::registry_revocation::current::require(
        ctx,
        families,
        licence.id,
        term.generation,
        Some(now),
    )
    .is_err();
    Ok(Some(HouseAttestationDue {
        licence: licence.id,
        organization,
        scope,
        generation: term.generation.get(),
        unattestable,
        revoked,
    }))
}
