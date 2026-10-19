//! Complete four-owner recovery. Current actor, title, licence pointers and
//! parent eligibility never replace retained Open facts or mint a Close.
use super::super::{
    Capacity, LicenceId, RevisionIdentity,
    families::CommerceFamilies,
    rating::{RatingFamilies, RatingRef},
    records::{OwnerTerm, PublicationError, Record},
};
use super::{ClaimInventory, ClaimRecord, IcClaimConfig, IcCloseKind, MAX_IC_CLAIM_HISTORY};
use crate::RollStore;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ModuleCtx, ObjId, World};
use mush_sim::ic_exclusion::{IcEventRef, IcExclusions};
use mush_tactics::UnitStore;
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

/// Owner-created history projection, not a source/execution/close capability.
/// Only the actual typed-family scan below constructs these values.
pub struct IcClaimHistory {
    pub(super) row: ClaimRecord,
}
impl IcClaimHistory {
    pub fn open(&self) -> CommitToken {
        self.row.open.operation
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.row.open.action
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.row.open.protocol.manifest
    }
    pub fn participants(&self) -> [DefinitionRef; 4] {
        self.row.open.protocol.participants.ordered()
    }
    pub fn unit(&self) -> ObjId {
        self.row.open.unit
    }
    pub fn dispatcher(&self) -> ObjId {
        self.row.open.dispatcher
    }
    pub fn sortie(&self) -> u64 {
        self.row.open.sortie.get()
    }
    pub fn licence(&self) -> LicenceId {
        self.row.open.licence
    }
    pub fn revision(&self) -> RevisionIdentity {
        self.row.open.revision
    }
    pub fn rating(&self) -> RatingRef {
        self.row.open.rating
    }
    pub fn seal_digest(&self) -> DefinitionDigest {
        self.row.open.seal_digest
    }
    pub fn custody_digest(&self) -> DefinitionDigest {
        self.row.open.custody_digest
    }
    pub fn exclusion_digest(&self) -> DefinitionDigest {
        self.row.open.exclusion_digest
    }
    pub fn admission_digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.row.open.admission_digest()
    }
    pub fn opened_at_ic(&self) -> &mush_core::ClockStamp {
        &self.row.open.at_ic
    }
    pub fn closed(
        &self,
    ) -> Option<(
        CommitToken,
        &DefinitionRef,
        u64,
        IcEventRef,
        IcCloseKind,
        &mush_core::ClockStamp,
    )> {
        self.row.closed.as_ref().map(|c| {
            let kind = match c.kind {
                super::CloseKind::ReturnedToBase { base } => IcCloseKind::ReturnedToBase { base },
                super::CloseKind::PoweredDownInBase { base } => {
                    IcCloseKind::PoweredDownInBase { base }
                }
                super::CloseKind::Destroyed {} => IcCloseKind::Destroyed,
                super::CloseKind::RecoveredToBase { base } => IcCloseKind::RecoveredToBase { base },
            };
            (
                c.operation,
                &c.action,
                c.transition,
                IcEventRef {
                    sequence: c.event_sequence,
                    digest: c.event_digest,
                },
                kind,
                &c.at_ic,
            )
        })
    }
}
pub(super) fn rows(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
) -> Result<Vec<IcClaimHistory>, PublicationError> {
    let mut rows = Vec::new();
    super::super::audit::scan(ctx, &f.ic_claims, |row| {
        if rows.len() >= MAX_IC_CLAIM_HISTORY {
            return Err(PublicationError::Capacity);
        }
        rows.push(IcClaimHistory { row: row.clone() });
        Ok(())
    })?;
    Ok(rows)
}
pub(super) fn local(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    rows: &[IcClaimHistory],
) -> Result<ClaimInventory, PublicationError> {
    for history in rows {
        let row = &history.row;
        row.validate()?;
        let open = &row.open;
        let exact = IcClaimConfig::new(
            open.protocol.manifest.clone(),
            open.protocol.participants.seal.clone(),
        )
        .map_err(bad)?;
        if exact.protocol != open.protocol {
            return Err(PublicationError::Mismatch);
        }
        if let Some(close) = &row.closed {
            let exact = IcClaimConfig::new(
                close.protocol.manifest.clone(),
                close.protocol.participants.seal.clone(),
            )
            .map_err(bad)?;
            if exact.protocol != close.protocol {
                return Err(PublicationError::Mismatch);
            }
        }
        let filed = super::super::audit::read(ctx, &f.filed, open.filed.key())?;
        let position = filed.content.position();
        if filed.content.revision() != open.revision
            || filed.rating != open.rating
            || position.charter != open.charter
            || position.line != open.line
        {
            return Err(PublicationError::Mismatch);
        }
        let rating = ratings.resolve_receipt(ctx, &open.rating).map_err(bad)?;
        if rating.subject().revision() != open.revision {
            return Err(PublicationError::Mismatch);
        }
        match open.capacity {
            Capacity::PaidSeats { seats } => {
                let licence = super::super::audit::read(ctx, &f.paid_licences, open.licence.key())?;
                let term = super::super::audit::read(
                    ctx,
                    &f.paid_terms,
                    format!("{}/{}", open.licence.key(), open.term.key()),
                )?;
                let grant =
                    super::super::entitlement::history::granted(ctx, f, ratings, licence, term)?;
                if grant.holder() != open.dispatcher
                    || grant.filed() != open.filed
                    || grant.revision() != open.revision
                    || grant.rating() != open.rating
                    || grant.charter() != open.charter
                    || grant.line() != open.line
                    || term.window() != open.window
                    || term.terms().seats() != seats
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            Capacity::OwnedDispatch {} => {
                let licence = super::super::audit::read(ctx, &f.licences, open.licence.key())?;
                let term = super::super::audit::read(
                    ctx,
                    &f.terms,
                    OwnerTerm::encoded_key(open.licence, open.term),
                )?;
                let birth =
                    super::super::audit::read(ctx, &f.operations, licence.operation.to_hex())?;
                let historical_owner = match &open.charter_ownership {
                    None => birth.requester,
                    Some(proof) => {
                        super::super::charter_transfer::history::historical(ctx, f, birth, proof)?
                    }
                };
                if historical_owner != open.dispatcher
                    || birth.charter != open.charter
                    || birth.line != open.line
                    || term.window != open.window
                    || term.licence != open.licence
                    || term.generation != open.term
                    || licence.charter != open.charter
                    || licence.line != open.line
                {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        roll.check_required_root(&super::roots::required(row)?)
            .map_err(bad)?;
    }
    ClaimInventory::reconstruct(rows.iter().map(|v| v.row.clone()))
}
pub(super) fn physical(world: &World, rows: &[IcClaimHistory]) -> Result<(), PublicationError> {
    for (kind, owner) in [
        (std::any::TypeId::of::<UnitStore>(), "tactics"),
        (std::any::TypeId::of::<IcExclusions>(), "sim"),
    ] {
        if world
            .module_state_owner(kind)
            .is_none_or(|v| v.as_str() != owner)
        {
            return Err(PublicationError::Unavailable);
        }
    }
    let units = world
        .module_state::<UnitStore>()
        .ok_or(PublicationError::Unavailable)?
        .ic_custody_inventory()
        .map_err(bad)?;
    let exclusion = world
        .module_state::<IcExclusions>()
        .ok_or(PublicationError::Unavailable)?
        .inventory()
        .map_err(bad)?;
    if units.len() != rows.len() || exclusion.len() != rows.len() {
        return Err(PublicationError::Mismatch);
    }
    for history in rows {
        let open = &history.row.open;
        let unit = units
            .iter()
            .find(|v| v.open() == open.operation)
            .ok_or(PublicationError::Missing)?;
        let spatial = exclusion
            .iter()
            .find(|v| v.identity().open == open.operation)
            .ok_or(PublicationError::Missing)?;
        let id = spatial.identity();
        if unit.unit() != open.unit
            || unit.sortie() != open.sortie.get()
            || unit.seal_digest() != open.seal_digest
            || unit.digest().map_err(bad)? != open.custody_digest
            || spatial.digest().map_err(bad)? != open.exclusion_digest
            || spatial.dispatcher() != open.dispatcher
            || id.unit != open.unit
            || id.sortie != open.sortie.get()
            || id.seal_digest != open.seal_digest
            || id.custody_digest != open.custody_digest
        {
            return Err(PublicationError::Mismatch);
        }
        match (history.closed(), unit.closed(), spatial.closed()) {
            (None, None, None) => {}
            (
                Some((commit, action, _, event, _, _)),
                Some((u_commit, u_action, u_event, _)),
                Some((s_commit, s_action, s_event)),
            ) if commit == u_commit
                && commit == s_commit
                && action == u_action
                && action == s_action
                && &event == u_event
                && &event == s_event => {}
            _ => return Err(PublicationError::Mismatch),
        }
    }
    Ok(())
}
pub(crate) fn disabled(ctx: &ModuleCtx<'_>, f: &CommerceFamilies) -> Result<(), PublicationError> {
    if !rows(ctx, f)?.is_empty() {
        return Err(PublicationError::Unavailable);
    }
    // Ordinary old-only rosters may omit either owner. A present physical
    // owner must have a completely loaded empty inventory, never absent data.
    if let Some(units) = ctx.state::<UnitStore>() {
        if !units.ic_custody_inventory().map_err(bad)?.is_empty() {
            return Err(PublicationError::Unavailable);
        }
    }
    if let Some(exclusions) = ctx.state::<IcExclusions>() {
        if !exclusions.inventory().map_err(bad)?.is_empty() {
            return Err(PublicationError::Unavailable);
        }
    }
    Ok(())
}
