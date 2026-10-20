//! Exact Registry sanction and pending refund in the same acknowledged union.
//! The Registry provider supplies its real live authorization fence; no service
//! identity, callback or second Root preparation is fabricated here.
use super::{
    Readiness, bad, history,
    publication::{Mode, PendingFence, PendingRoots},
    records::{PendingHead, PendingState},
};
use crate::commerce::{
    charter_transfer::records::CharterTransfer,
    families::CommerceFamilies,
    records::{PublicationError, Record},
    registry_revocation::{
        RevocationFence,
        records::{AcceptedTerm, RevocationOperation},
    },
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionInvocation, ActionTxn, CommitToken, PrepareCtx};
use mush_economy::{
    managed_charter_transfer::{
        CharterPendingDecisionKindV1, CharterPendingDecisionV1, CharterPendingTerminalRequest,
    },
    registry_filing::{EconomyRequest, InitialFilingParticipant},
};

pub(crate) fn prepare(
    f: &CommerceFamilies,
    ready: &Readiness,
    economy: &ParticipantHandle<InitialFilingParticipant>,
    inv: &ActionInvocation,
    txn: &mut ActionTxn<'_>,
    ctx: &mut PrepareCtx<'_>,
    sanction: &RevocationOperation,
    authorization: RevocationFence,
) -> Result<Option<(CommitToken, PendingRoots)>, PublicationError> {
    let AcceptedTerm::Owner { term } = &sanction.term else {
        return Ok(None);
    };
    let Some(head) = txn
        .get(&f.charter_pending_heads, &sanction.charter.key())
        .map_err(bad)?
        .cloned()
    else {
        return Ok(None);
    };
    let before = txn
        .get(&f.charter_pending, &head.current.operation.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    head.matches(&before)?;
    if !matches!(before.state, PendingState::Funded {}) {
        return Ok(None);
    }
    before.owner_term.matches(term)?;
    if before.charter != sanction.charter
        || sanction.operation != ctx.commit_token()
        || sanction.actor != inv.actor()
    {
        return Err(PublicationError::Mismatch);
    }
    let offer = txn
        .get(&f.charter_pending_offers, &before.offer.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    before.matches_offer(&offer)?;
    let birth = txn
        .get(&f.operations, &offer.birth.to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    offer.matches_birth(&birth)?;
    let world = ctx.world().map_err(bad)?;
    let native = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if native
        .pending_charter_history(before.operation)
        .map_err(bad)?
        != &before.economic
    {
        return Err(PublicationError::Mismatch);
    }
    let ownership = crate::commerce::charter_transfer::history::current_in_txn(
        txn,
        world,
        f,
        &birth,
        offer.seller,
    )?;
    if ownership != before.ownership
        || txn
            .get(
                &f.charter_transfers,
                &CharterTransfer::key_for(
                    before.charter,
                    ownership
                        .generation
                        .checked_add(1)
                        .ok_or(PublicationError::Capacity)?,
                ),
            )
            .map_err(bad)?
            .is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    let decision = CharterPendingDecisionV1 {
        format: 1,
        operation: ctx.commit_token(),
        action: sanction.action.clone(),
        actor: inv.actor(),
        at_ic: world.world_ic_now(),
        posted_at_utc: world.now(),
        reason: CharterPendingDecisionKindV1::RegistryRevoked {
            sanction: sanction.operation,
            licence: term.licence.get(),
            generation: term.generation.get(),
            term_digest: before.owner_term.digest,
        },
    };
    let fence = PendingFence {
        action: sanction.action.clone(),
        commit: ctx.commit_token(),
        actor: inv.actor(),
        parameters: inv.parameters().clone(),
        at_ic: decision.at_ic.clone(),
        at_utc: decision.posted_at_utc,
        ready: ready.clone(),
        mode: Mode::RegistryRefund(Box::new(authorization)),
    };
    fence.check_world(world)?;
    let output = ctx
        .prepare_owner(
            economy,
            EconomyRequest::CharterPendingTerminal(CharterPendingTerminalRequest {
                funding: before.operation,
                decision: decision.clone(),
            }),
        )
        .map_err(bad)?
        .charter_pending()
        .map_err(bad)?;
    let economic = output.history().clone();
    if economic.consent() != before.economic.consent()
        || economic.funding() != before.economic.funding()
        || economic
            .terminal()
            .is_none_or(|t| t.decision() != &decision)
    {
        return Err(PublicationError::Mismatch);
    }
    let mut row = before.clone();
    row.economic = economic;
    row.state = PendingState::Refunded {
        terminal: ctx.commit_token(),
        reason: decision.reason,
    };
    row.matches_offer(&offer)?;
    let head = PendingHead::of(&row)?;
    let roots = PendingRoots {
        commit: ctx.commit_token(),
        old: vec![
            history::offer_root(&offer)?,
            history::funding_root(&before, &offer)?,
        ],
        added: vec![history::terminal_root(&row, &offer)?],
        fence,
    };
    txn.replace(&f.charter_pending, &row.key(), row.clone())
        .map_err(bad)?;
    txn.replace(&f.charter_pending_heads, &head.key(), head.clone())
        .map_err(bad)?;
    if txn.get(&f.charter_pending, &row.key()).map_err(bad)? != Some(&row)
        || txn
            .get(&f.charter_pending_heads, &head.key())
            .map_err(bad)?
            != Some(&head)
    {
        return Err(PublicationError::Mismatch);
    }
    // The caller inserts the exact sanction/status before the fixed Core close
    // checks the full four-row transaction and releases future capacity.
    Ok(Some((before.operation, roots)))
}
