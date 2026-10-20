//! Historical transfer truth from the real paired owners. Neither a current
//! pointer alone nor a caller-decoded receipt can establish this observation.
use super::{PublicationError, reader::Truth};
use crate::commerce::{
    entitlement::history::read,
    families::CommerceFamilies,
    paid_resale::{self, PaidHolderRecord, PaidResaleState},
    rating::RatingFamilies,
    read::CommerceRead,
};
use mush_core::{CommitToken, ObjId};
use mush_economy::managed_paid_resale::{MAX_HOLDER_GENERATION, PaidResaleModeV1};

pub(super) struct TransferProof {
    pub(super) seller: ObjId,
    pub(super) buyer: ObjId,
    pub(super) truth: Option<Truth>,
}

pub(super) fn resolve(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    operation: CommitToken,
    generation: u64,
) -> Result<TransferProof, PublicationError> {
    if !(2..=MAX_HOLDER_GENERATION).contains(&generation) {
        return Err(PublicationError::Unavailable);
    }
    let row = read(ctx, &f.paid_resales, operation.to_hex())?;
    if row.operation() != operation
        || row.binding().ownership().generation.checked_add(1) != Some(generation)
    {
        return Err(PublicationError::Mismatch);
    }
    let proof = paid_resale::audit::binding(ctx, f, ratings, row.binding())?;
    let c = row.economic().consent();
    if proof.seller() != c.terms.seller {
        return Err(PublicationError::Mismatch);
    }
    let pointer = read(ctx, &f.paid_licences, row.binding().licence().key())?;
    let born = paid_resale::history::birth(ctx, f, ratings, pointer)?;
    paid_resale::history::native(ctx.world(), row)?;
    paid_resale::history::roots(ctx.world(), row, born.revision)?;
    let truth = match row.state() {
        PaidResaleState::Funded {} | PaidResaleState::Refunded { .. } => None,
        PaidResaleState::Settled {
            ownership,
            terminal,
        } => {
            let holder = read(
                ctx,
                &f.paid_holders,
                PaidHolderRecord::key_for(pointer.id(), generation),
            )?;
            holder.matches_record(row)?;
            if holder.ownership()? != *ownership
                || holder.operation() != *terminal
                || holder.acceptance() != operation
                || holder.holder() != c.terms.buyer
            {
                return Err(PublicationError::Mismatch);
            }
            let (at, actor) = match (c.mode, row.economic().terminal()) {
                (PaidResaleModeV1::Immediate, None) => (c.accepted_at_utc, c.terms.buyer),
                (PaidResaleModeV1::Pending, Some(receipt))
                    if receipt.decision().reason.settles() =>
                {
                    (receipt.decision().posted_at_utc, receipt.decision().actor)
                }
                _ => return Err(PublicationError::Mismatch),
            };
            Some(Truth { at, actor })
        }
    };
    Ok(TransferProof {
        seller: c.terms.seller,
        buyer: c.terms.buyer,
        truth,
    })
}
