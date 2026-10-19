//! Registry sanction and pending escrow refund share one actual native owner
//! request and one Root image. Nothing is dispatched after acknowledgement.
use super::{
    history,
    publication::{Mode, ResaleFence, ResaleRoots},
    records::*,
    writer::PaidResaleWriter,
};
use crate::commerce::{
    records::{PublicationError, Record},
    registry_revocation::{
        RevocationFence,
        records::{AcceptedTerm, RevocationOperation},
    },
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, CommitToken, PrepareCtx};
use mush_economy::{
    managed_paid_resale::{
        PaidResaleDecisionKindV1, PaidResaleDecisionV1, PaidResaleOutcome,
        PaidResaleTerminalRequest,
    },
    registry_filing::{EconomyRequest, InitialFilingParticipant},
};
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

pub(crate) fn prepare(
    writer: &PaidResaleWriter,
    economy: &ParticipantHandle<InitialFilingParticipant>,
    txn: &mut ActionTxn<'_>,
    ctx: &mut PrepareCtx<'_>,
    op: &RevocationOperation,
    fence: RevocationFence,
) -> Result<Option<(CommitToken, ResaleRoots)>, PublicationError> {
    let AcceptedTerm::Paid { term } = &op.term else {
        return Ok(None);
    };
    let (f, ready, market) = writer.compound_parts()?;
    let Some(head) = txn
        .get(&f.paid_resale_heads, &term.licence().key())
        .map_err(bad)?
        .cloned()
    else {
        return Ok(None);
    };
    let before = txn
        .get(&f.paid_resales, &head.current().operation().to_hex())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    head.matches(&before)?;
    if !matches!(before.state(), PaidResaleState::Funded {}) {
        return Ok(None);
    }
    let b = before.binding();
    let pointer = txn
        .get(&f.paid_licences, &b.licence().key())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    pointer.validate()?;
    if b.term() != term
        || op.charter != b.charter()
        || op.line != b.line()
        || op.birth_operation != b.birth().operation
        || op.operation != ctx.commit_token()
        || pointer.operation() != b.birth().operation
        || pointer.term() != term.generation()
        || pointer.holder() != before.economic().consent().terms.seller
        || pointer.ownership()
            != if b.ownership().generation == 1 {
                None
            } else {
                Some(b.ownership())
            }
        || txn.get(&f.paid_terms, &term.key()).map_err(bad)? != Some(term)
    {
        return Err(PublicationError::Mismatch);
    }
    // The full accepted term/native purchase was proved by Registry capture.
    // Read its immutable birth identity rather than assuming a Follow term's
    // currently selected filed revision is the old transfer root's subject.
    let old = txn
        .get(&f.paid_allocations, &pointer.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let entitled = txn
        .get(&f.entitlement_allocations, &pointer.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let house = txn
        .get(&f.house_allocations, &pointer.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let revision = match (old, entitled, house) {
        (Some(row), None, None) => {
            row.validate()?;
            row.revision()
        }
        (None, Some(row), None) => {
            row.validate()?;
            row.revision()
        }
        (None, None, Some(row)) => {
            row.validate()?;
            row.revision()
        }
        _ => return Err(PublicationError::Mismatch),
    };
    history::native(ctx.world().map_err(bad)?, &before)?;
    history::roots(ctx.world().map_err(bad)?, &before, revision)?;
    let decision = PaidResaleDecisionV1 {
        format: 1,
        operation: op.operation,
        action: op.action.clone(),
        actor: op.actor,
        at_ic: op.at_ic.clone(),
        posted_at_utc: ctx.world().map_err(bad)?.now(),
        reason: PaidResaleDecisionKindV1::RegistryRevoked {
            sanction: op.operation,
            licence: term.licence().get(),
            generation: term.generation().get(),
            term_digest: b.native_basis()?.term.digest,
        },
    };
    decision
        .matches_consent(before.economic().consent())
        .map_err(bad)?;
    let output = ctx
        .prepare_owner(
            economy,
            EconomyRequest::PaidResaleTerminal(PaidResaleTerminalRequest {
                acceptance: before.operation(),
                decision: decision.clone(),
            }),
        )
        .and_then(|o| o.paid_resale())
        .map_err(bad)?;
    let native = output.history();
    if native.outcome() != PaidResaleOutcome::Refunded
        || native.consent() != before.economic().consent()
        || native.acceptance() != before.economic().acceptance()
        || native.terminal().is_none_or(|t| t.decision() != &decision)
    {
        return Err(PublicationError::Mismatch);
    }
    let mut row = before.clone();
    row.economic = native.clone();
    row.state = PaidResaleState::Refunded {
        terminal: op.operation,
        reason: decision.reason.clone(),
    };
    row.validate()?;
    let next = PaidResaleHead::from_record(&row)?;
    let roots = ResaleRoots {
        commit: op.operation,
        old: vec![history::acceptance_root(&before, revision)?],
        added: vec![history::terminal_root(&row, revision)?],
        fence: ResaleFence {
            action: op.action.clone(),
            commit: op.operation,
            at_ic: op.at_ic.clone(),
            at_utc: decision.posted_at_utc,
            mode: Mode::Registry { ready, fence },
        },
    };
    txn.replace(&f.paid_resales, &row.key(), row.clone())
        .map_err(bad)?;
    txn.replace(&f.paid_resale_heads, &next.key(), next.clone())
        .map_err(bad)?;
    market
        .publish_terminal(ctx, txn, &before, &row)
        .map_err(bad)?;
    if txn.get(&f.paid_resales, &row.key()).map_err(bad)? != Some(&row)
        || txn.get(&f.paid_resale_heads, &next.key()).map_err(bad)? != Some(&next)
        || txn.get(&f.paid_licences, &pointer.key()).map_err(bad)? != Some(&pointer)
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(Some((before.operation(), roots)))
}
