//! Complete typed/native/root pairing. Claim observations are checked against
//! the actual four-owner audited inventory before commerce readiness opens.
use super::{history, records::*};
use crate::commerce::{
    CharterId,
    audit::{read, scan},
    charter_transfer::{history as ownership, records::CharterTransfer},
    families::CommerceFamilies,
    ic_claims::ClaimInventory,
    records::{PublicationError, Record},
    registry_revocation::records::{AcceptedTerm, RevokedTerm},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{CommitToken, ModuleCtx};
use mush_economy::managed_charter_transfer::CharterPendingDecisionKindV1 as Decision;
use std::collections::{BTreeMap, BTreeSet};

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

pub(crate) fn root(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    anchor: &RootAnchor,
) -> Result<RequiredRoot, PublicationError> {
    match anchor {
        RootAnchor::CharterPendingOffer(operation, _) => {
            history::offer_root(read(ctx, &f.charter_pending_offers, operation.to_hex())?)
        }
        RootAnchor::CharterPendingFunding(operation, _)
        | RootAnchor::CharterPendingTerminal(operation, _) => {
            let row = read(ctx, &f.charter_pending, operation.to_hex())?;
            let offer = read(ctx, &f.charter_pending_offers, row.offer.to_hex())?;
            if matches!(anchor, RootAnchor::CharterPendingFunding(..)) {
                history::funding_root(row, offer)
            } else {
                history::terminal_root(row, offer)
            }
        }
        _ => Err(PublicationError::Mismatch),
    }
}

/// Called only after the real IC installer has completed its four-owner
/// inverse audit (or disabled mode has proved every IC family empty).
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    enabled: bool,
    claims: Option<&ClaimInventory>,
) -> Result<(), PublicationError> {
    let roll = ctx
        .w
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    let economy = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let native = economy.pending_charter_histories().map_err(bad)?;
    let mut native_keys = BTreeSet::new();
    for h in native {
        if !enabled
            || native_keys.len() >= MAX_PENDING_HISTORY as usize
            || !native_keys.insert(h.consent().operation)
        {
            return Err(PublicationError::Mismatch);
        }
    }
    let mut offers = BTreeSet::new();
    let mut named = BTreeMap::new();
    scan(ctx, &f.charter_pending_offers, |offer| {
        if !enabled || offers.len() >= crate::MAX_RECORDS || !offers.insert(offer.operation) {
            return Err(PublicationError::Mismatch);
        }
        named.insert(offer.operation, (offer.buyer, offer.charter));
        let birth = read(ctx, &f.operations, offer.birth.to_hex())?;
        offer.matches_birth(birth)?;
        let filed = read(ctx, &f.filed, offer.filed.key())?;
        let term = crate::commerce::owner_attestation::history::historical(
            ctx,
            roll,
            f,
            birth,
            offer.owner_term.generation,
        )?;
        offer.owner_term.matches(&term)?;
        if ownership::historical(ctx, f, birth, &offer.ownership)? != offer.seller
            || filed.content.position().charter != offer.charter
            || filed.content.position().line != offer.line
            || filed.content.revision() != offer.revision
        {
            return Err(PublicationError::Mismatch);
        }
        if offer.ownership.generation > 1 {
            let previous = read(
                ctx,
                &f.charter_transfers,
                CharterTransfer::key_for(offer.charter, offer.ownership.generation),
            )?;
            if previous.ownership()? != offer.ownership
                || offer.offered_at_utc < previous.economic.facts()?.posted_at_utc
            {
                return Err(PublicationError::Mismatch);
            }
        }
        roll.check_required_root(&history::offer_root(offer)?)
            .map_err(bad)?;
        roll.check_required_root(&RequiredRoot {
            anchor: RootAnchor::Filed(offer.filed),
            identity: offer.revision,
        })
        .map_err(bad)
    })?;
    let mut chains = BTreeMap::<CharterId, BTreeMap<u64, CommitToken>>::new();
    let mut used_offers = BTreeSet::new();
    let mut terminals = BTreeSet::new();
    scan(ctx, &f.charter_pending, |row| {
        if !enabled
            || !native_keys.remove(&row.operation)
            || !offers.contains(&row.offer)
            || offers.contains(&row.operation)
            || !used_offers.insert(row.offer)
            || chains
                .entry(row.charter)
                .or_default()
                .insert(row.sequence, row.operation)
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let offer = read(ctx, &f.charter_pending_offers, row.offer.to_hex())?;
        row.matches_offer(offer)?;
        if economy
            .pending_charter_history(row.operation)
            .map_err(bad)?
            != &row.economic
        {
            return Err(PublicationError::Mismatch);
        }
        if count_at(claims, row.charter, row.claim_observation.next_transition)?
            != row.claim_observation.open_owned
        {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&history::funding_root(row, offer)?)
            .map_err(bad)?;
        if let Some(terminal) = row.economic.terminal() {
            let d = terminal.decision();
            if !terminals.insert(d.operation) || offers.contains(&d.operation) {
                return Err(PublicationError::Mismatch);
            }
            match &d.reason {
                Decision::LastClaimClosed { next_transition } => {
                    if *next_transition < row.claim_observation.next_transition
                        || count_at(claims, row.charter, *next_transition)? != 0
                    {
                        return Err(PublicationError::Mismatch);
                    }
                }
                Decision::DeadlineReached {
                    next_transition,
                    open_owned,
                } => {
                    if *next_transition < row.claim_observation.next_transition
                        || count_at(claims, row.charter, *next_transition)? != *open_owned
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
                    let status = read(
                        ctx,
                        &f.revoked_terms,
                        RevokedTerm::key_for(row.owner_term.licence, row.owner_term.generation),
                    )?;
                    status.matches(op)?;
                    let AcceptedTerm::Owner { term } = &op.term else {
                        return Err(PublicationError::Mismatch);
                    };
                    row.owner_term.matches(term)?;
                    if op.charter != row.charter
                        || *licence != term.licence.get()
                        || *generation != term.generation.get()
                        || *term_digest != row.owner_term.digest
                        || (*sanction == d.operation
                            && (op.action != d.action
                                || op.actor != d.actor
                                || op.at_ic != d.at_ic))
                    {
                        return Err(PublicationError::Mismatch);
                    }
                    roll.check_required_root(&op.root()?).map_err(bad)?;
                }
                // Native immutable clock evidence is validated by the full
                // history. A missing recipient is a live owner check, never a
                // requirement that their object remain missing forever.
                Decision::ClockChanged | Decision::RecipientUnavailable => {}
            }
            roll.check_required_root(&history::terminal_root(row, offer)?)
                .map_err(bad)?;
        }
        if let PendingState::Settled { transfer, .. } = &row.state {
            let ownership = read(
                ctx,
                &f.charter_transfers,
                CharterTransfer::key_for(row.charter, transfer.generation),
            )?;
            roll.check_required_root(&history::transfer_root(ownership, offer, row)?)
                .map_err(bad)?;
        }
        Ok(())
    })?;
    if !native_keys.is_empty() {
        return Err(PublicationError::Mismatch);
    }
    // 17g C2f: the per-(buyer, charter) offer index, clock-free.
    let mut indexed = Vec::new();
    scan(ctx, &f.charter_pending_buyer_offers, |row| {
        if !enabled {
            return Err(PublicationError::Mismatch);
        }
        indexed.push((row.key(), row.clone()));
        Ok(())
    })?;
    super::index::check(indexed, |token| named.get(&token).copied(), &used_offers)?;
    let mut heads = BTreeSet::new();
    for (charter, chain) in &chains {
        let mut previous: Option<&PendingRecord> = None;
        for (ordinal, operation) in chain {
            if terminals.contains(operation) {
                return Err(PublicationError::Mismatch);
            }
            let row = read(ctx, &f.charter_pending, operation.to_hex())?;
            match previous {
                None if *ordinal == 1 && row.previous.is_none() => {}
                Some(prior)
                    if !matches!(prior.state, PendingState::Funded {})
                        && prior.sequence.checked_add(1) == Some(*ordinal)
                        && row.previous == Some(prior.reference()?)
                        && row.economic.funding().journal_seq()
                            > prior
                                .economic
                                .terminal()
                                .ok_or(PublicationError::Mismatch)?
                                .journal_seq() => {}
                _ => return Err(PublicationError::Mismatch),
            }
            previous = Some(row);
        }
        let last = previous.ok_or(PublicationError::Missing)?;
        let head = read(ctx, &f.charter_pending_heads, charter.key())?;
        head.matches(last)?;
        heads.insert(*charter);
        if matches!(last.state, PendingState::Funded {}) {
            let offer = read(ctx, &f.charter_pending_offers, last.offer.to_hex())?;
            let birth = read(ctx, &f.operations, offer.birth.to_hex())?;
            // A retained pending gate prohibits a new ownership generation;
            // present clocks/recipient liveness do not block boot or cleanup.
            let current = ctx
                .action_state(&f.charter_ownership, &charter.key())
                .map_err(bad)?
                .map(|head| head.current.clone())
                .unwrap_or(
                    crate::commerce::charter_transfer::records::CharterOwnershipRef::birth(birth)?,
                );
            if current != last.ownership
                || economy.snapshot().asset(charter.object()).is_none_or(|a| {
                    a.title_owner != mush_org::PrincipalId::Object(offer.seller)
                        || a.revision != offer.title_revision
                })
            {
                return Err(PublicationError::Mismatch);
            }
        }
    }
    scan(ctx, &f.charter_pending_heads, |head| {
        if !enabled || !heads.remove(&head.charter) {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    if !heads.is_empty() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

fn count_at(
    claims: Option<&ClaimInventory>,
    charter: CharterId,
    next: u64,
) -> Result<u32, PublicationError> {
    match claims {
        Some(book) => book.owned_charter_at(charter, next),
        None if next == 1 => Ok(0),
        None => Err(PublicationError::Mismatch),
    }
}
