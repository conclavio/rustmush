//! Complete immutable monetary/holder/root pairing. Live source eligibility
//! never participates in recovery of property or already captured claims.
use super::{chain, economics, history, records::*};
use crate::commerce::{
    LicenceId, RevisionIdentity,
    audit::{read, scan},
    families::CommerceFamilies,
    ic_claims::ClaimInventory,
    rating::RatingFamilies,
    read::CommerceRead,
    records::{PublicationError, Record},
    registry_revocation::records::{AcceptedTerm, RevokedTerm},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{CommitToken, ModuleCtx, ObjId};
use mush_economy::managed_paid_resale::PaidResaleDecisionKindV1 as Decision;
use std::collections::{BTreeMap, BTreeSet};
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

/// Historical disclosure only. This carries no publication or current-use
/// authority and does not require the captured holder still owns the licence.
pub struct HistoricalBinding {
    pub(super) seller: ObjId,
    pub(super) revision: RevisionIdentity,
    pub(super) economics: super::ResaleEconomics,
}
impl HistoricalBinding {
    pub fn seller(&self) -> ObjId {
        self.seller
    }
    pub fn economics(&self) -> &super::ResaleEconomics {
        &self.economics
    }
}
pub(crate) fn binding(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    b: &PaidResaleBinding,
) -> Result<HistoricalBinding, PublicationError> {
    use crate::commerce::entitlement::history::read as actual;
    b.validate()?;
    let pointer = actual(ctx, &f.paid_licences, b.licence().key())?;
    let born = history::birth(ctx, f, ratings, pointer)?;
    let term = actual(ctx, &f.paid_terms, b.term().key())?;
    if pointer.charter() != b.charter()
        || pointer.line() != b.line()
        || &born.reference != b.birth()
        || term != b.term()
        || history::issued_clock(ctx, f, pointer, term)? != *b.term_clock()
    {
        return Err(PublicationError::Mismatch);
    }
    // A resale's seller is a character (M8c: organisation-held licences
    // are not resold).
    let seller = history::historical(ctx, f, ratings, pointer, b.ownership())?
        .character()
        .ok_or(PublicationError::Mismatch)?;
    let economics = economics::accepted(ctx, f, ratings, pointer, term)?;
    if economics.economics_digest() != b.economics_digest()
        || economics.royalty_chain_digest() != b.royalty_chain_digest()
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(HistoricalBinding {
        seller,
        revision: born.revision,
        economics,
    })
}

pub(crate) fn root(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    anchor: &RootAnchor,
) -> Result<RequiredRoot, PublicationError> {
    let (operation, terminal) = match anchor {
        RootAnchor::PaidResaleFunding(op, _) => (*op, false),
        RootAnchor::PaidResaleTerminal(op, _) => (*op, true),
        _ => return Err(PublicationError::Mismatch),
    };
    let row = read(ctx, &f.paid_resales, operation.to_hex())?;
    let pointer = read(ctx, &f.paid_licences, row.binding().licence().key())?;
    let born = history::birth(ctx, f, ratings, pointer)?;
    if terminal {
        history::terminal_root(row, born.revision)
    } else {
        history::acceptance_root(row, born.revision)
    }
}

fn count_at(
    claims: Option<&ClaimInventory>,
    licence: LicenceId,
    next: u64,
) -> Result<u32, PublicationError> {
    match claims {
        Some(book) => book.paid_at(licence, next),
        None if next == 1 => Ok(0),
        None => Err(PublicationError::Mismatch),
    }
}

/// Invoked after the real IC inventory has passed all four-owner joins, and
/// before Roll commerce readiness opens. Even an empty typed inventory must
/// agree with the actual completed native history inventory.
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
    claims: Option<&ClaimInventory>,
) -> Result<(), PublicationError> {
    let e = ctx
        .state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let roll = ctx
        .state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    let mut native = BTreeSet::new();
    for row in e.paid_resale_histories().map_err(bad)? {
        if !enabled || native.len() >= MAX_OPERATIONS || !native.insert(row.consent().operation) {
            return Err(PublicationError::Mismatch);
        }
    }
    let mut operations = BTreeMap::<LicenceId, Vec<CommitToken>>::new();
    let mut all_operations = BTreeSet::new();
    let mut all_offers = BTreeSet::new();
    let mut terminals = BTreeSet::new();
    scan(ctx, &f.paid_resales, |row| {
        if !enabled
            || !native.remove(&row.operation())
            || all_operations.len() >= MAX_OPERATIONS
            || !all_operations.insert(row.operation())
            || !all_offers.insert(row.offer())
        {
            return Err(PublicationError::Mismatch);
        }
        history::native(ctx.w, row)?;
        let proof = binding(ctx, f, ratings, row.binding())?;
        if proof.seller != row.economic().consent().terms.seller
            || proof.economics.instrument() != &row.economic().consent().terms.instrument
            || count_at(
                claims,
                row.binding().licence(),
                row.claims().next_transition(),
            )? != row.claims().open_paid()
        {
            return Err(PublicationError::Mismatch);
        }
        history::roots(ctx.w, row, proof.revision)?;
        if let Some(t) = row.economic().terminal() {
            let d = t.decision();
            if !terminals.insert(d.operation) {
                return Err(PublicationError::Mismatch);
            }
            match &d.reason {
                Decision::LastClaimClosed { next_transition } => {
                    if *next_transition <= row.claims().next_transition()
                        || count_at(claims, row.binding().licence(), *next_transition)? != 0
                    {
                        return Err(PublicationError::Mismatch);
                    }
                }
                Decision::DeadlineReached {
                    next_transition,
                    open_paid,
                } => {
                    if *next_transition < row.claims().next_transition()
                        || count_at(claims, row.binding().licence(), *next_transition)?
                            != *open_paid
                    {
                        return Err(PublicationError::Mismatch);
                    }
                }
                Decision::RegistryRevoked {
                    sanction,
                    licence,
                    generation,
                    term_digest,
                } => {
                    let op = read(ctx, &f.registry_revocations, sanction.to_hex())?;
                    read(
                        ctx,
                        &f.revoked_terms,
                        RevokedTerm::key_for(
                            row.binding().licence(),
                            row.binding().term().generation(),
                        ),
                    )?
                    .matches(op)?;
                    if !matches!(&op.term, AcceptedTerm::Paid { term } if term.as_ref() == row.binding().term())
                        || op.charter != row.binding().charter()
                        || *licence != row.binding().licence().get()
                        || *generation != row.binding().term().generation().get()
                        || *term_digest != row.binding().native_basis()?.term.digest
                        || (*sanction == d.operation
                            && (op.action != d.action
                                || op.actor != d.actor
                                || op.at_ic != d.at_ic))
                    {
                        return Err(PublicationError::Mismatch);
                    }
                    roll.check_required_root(&op.root()?).map_err(bad)?;
                }
                Decision::ClockChanged {}
                | Decision::RecipientUnavailable {}
                | Decision::MonetaryBindingChanged {} => (),
            }
        }
        operations
            .entry(row.binding().licence())
            .or_default()
            .push(row.operation());
        Ok(())
    })?;
    if !native.is_empty()
        || all_offers
            .iter()
            .any(|k| all_operations.contains(k) || terminals.contains(k))
        || terminals.iter().any(|k| all_operations.contains(k))
    {
        return Err(PublicationError::Mismatch);
    }
    let mut holder_keys = BTreeMap::<LicenceId, BTreeSet<u64>>::new();
    let mut holder_count = 0usize;
    scan(ctx, &f.paid_holders, |holder| {
        holder_count += 1;
        if !enabled
            || holder_count > MAX_OPERATIONS
            || !operations.contains_key(&holder.licence())
            || !holder_keys
                .entry(holder.licence())
                .or_default()
                .insert(holder.generation())
        {
            return Err(PublicationError::Mismatch);
        }
        holder.matches_record(read(ctx, &f.paid_resales, holder.acceptance().to_hex())?)
    })?;
    let mut heads = BTreeSet::new();
    for (licence, ids) in &operations {
        let pointer = read(ctx, &f.paid_licences, licence.key())?;
        let born = history::birth(ctx, f, ratings, pointer)?;
        let head = read(ctx, &f.paid_resale_heads, licence.key())?;
        let rows = ids
            .iter()
            .map(|id| read(ctx, &f.paid_resales, id.to_hex()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut terms = BTreeMap::new();
        for row in &rows {
            terms.insert(
                row.binding().term().generation(),
                read(ctx, &f.paid_terms, row.binding().term().key())?,
            );
        }
        let mut holders = BTreeMap::new();
        if let Some(keys) = holder_keys.get(licence) {
            for generation in keys {
                holders.insert(
                    *generation,
                    read(
                        ctx,
                        &f.paid_holders,
                        PaidHolderRecord::key_for(*licence, *generation),
                    )?,
                );
            }
        }
        chain::verify(
            pointer,
            &born.reference,
            born.holder,
            Some(head),
            &rows,
            &holders,
            &terms,
        )?;
        heads.insert(*licence);
    }
    scan(ctx, &f.paid_resale_heads, |head| {
        if !enabled || !heads.remove(&head.licence()) {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    if !heads.is_empty() {
        return Err(PublicationError::Mismatch);
    }
    scan(ctx, &f.paid_licences, |pointer| {
        if pointer.ownership().is_some() && (!enabled || !operations.contains_key(&pointer.id())) {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    Ok(())
}
