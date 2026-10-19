//! Prebuilt nonmonetary images. No observer, serializer or World callback runs
//! after the consuming bind. This module exposes no independent public apply.

use super::{
    CLOSE_ACTION, DRAFT_ACTION, RenewalOffer, RenewalOfferOutput, UnfundedCloseReason,
    action_matches, contract,
    history::{self, Closure, OfferBook, OfferRecord},
    refused,
};
use crate::contract::{EconomicContract, check_offerable};
use crate::module::EconomyStore;
use crate::record::{Account, EconomicAsset, HoldPurpose, InstrumentDef};
use crate::registry_filing::{MetaFence, json_write};
use mush_core::authoritative::ReplayWrite;
use mush_core::participant::{BoundParticipant, OwnerPrepareContext, ParticipantError};
use mush_core::{CommitToken, DefinitionRef, ObjId};
use mush_org::PrincipalId;

pub(super) struct OfferContext<'a> {
    pub world: &'a mush_core::World,
    pub actor: ObjId,
    pub principal: &'a mush_core::PrincipalId,
    pub action: &'a DefinitionRef,
    pub commit: CommitToken,
}
impl<'a> From<&'a OwnerPrepareContext<'_>> for OfferContext<'a> {
    fn from(c: &'a OwnerPrepareContext<'_>) -> Self {
        Self {
            world: c.world(),
            actor: c.invocation().actor(),
            principal: c.invocation().principal(),
            action: c.action(),
            commit: c.commit_token(),
        }
    }
}
fn actor(context: &OfferContext<'_>, buyer: ObjId) -> Result<(), ParticipantError> {
    if context.actor != buyer
        || context.principal != &(mush_core::PrincipalId::Object { id: buyer })
        || !super::subject::player(context.world, buyer)
    {
        return Err(refused());
    }
    Ok(())
}
fn open_counts(state: &EconomyStore, buyer: ObjId) -> (usize, usize) {
    let buyer = PrincipalId::Object(buyer);
    state
        .snapshot
        .contracts()
        .filter(|c| !c.state.is_terminal())
        .fold((0, 0), |(issued, party), c| {
            (
                issued + usize::from(c.issuer == buyer),
                party + usize::from(c.consent.parties.contains(&buyer)),
            )
        })
}
fn offer_current(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: &RenewalOffer,
) -> Result<(), ParticipantError> {
    actor(context, request.terms.buyer)?;
    if !action_matches(context.action, DRAFT_ACTION)
        || context.commit == request.basis.prior_quote
        || context.commit == request.basis.predecessor_operation
    {
        return Err(refused());
    }
    request.terms.check_current(state, context.world)?;
    let now = context.world.world_ic_now();
    if now.clock != request.terms.not_before.clock
        || now.at < request.terms.not_before.at
        || now.at >= request.terms.expires_at.at
    {
        return Err(refused());
    }
    let (issued, party) = open_counts(state, request.terms.buyer);
    if issued >= crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER
        || party >= crate::contract::MAX_LIVE_CONTRACTS_PER_PARTY
    {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn offer(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
) -> Result<(RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>), ParticipantError> {
    create(state, context, request, None)
}
pub(super) fn retry(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
    previous_operation: CommitToken,
) -> Result<(RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>), ParticipantError> {
    create(state, context, request, Some(previous_operation))
}
fn create(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
    previous_operation: Option<CommitToken>,
) -> Result<(RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>), ParticipantError> {
    request.validate()?;
    offer_current(state, context, &request)?;
    let id = crate::ContractId(state.next_contract);
    let next_contract = state.next_contract.checked_add(1).ok_or_else(refused)?;
    if state.snapshot.contract(id).is_some() {
        return Err(refused());
    }
    let mut row = OfferRecord {
        format: if previous_operation.is_some() { 2 } else { 1 },
        attempt: previous_operation
            .map(|previous_operation| super::RenewalAttemptV1::BuyerRetry { previous_operation }),
        operation: context.commit,
        action: context.action.clone(),
        offer: request,
        contract: id,
        consent_hash: [0; 32],
        created_at_ic: context.world.world_ic_now(),
        created_at_utc: context.world.now(),
        closure: None,
    };
    let after = contract(&row)?;
    row.consent_hash = after.consent_hash;
    state.renewal_offers.admits(&row)?;
    state
        .renewal_offers
        .require_retry(&row, &state.snapshot, &state.fold.renewal_receipts)?;
    check_offerable(&after, &state.snapshot).map_err(|_| refused())?;
    images(state, row, after, None, next_contract)
}
pub(super) fn close(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    operation: CommitToken,
    reason: UnfundedCloseReason,
) -> Result<(RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>), ParticipantError> {
    let before = state.renewal_offers.get(operation).ok_or_else(refused)?;
    close_current(state, context, before, reason)?;
    before.verify_documents(&state.snapshot)?;
    let before_contract = state
        .snapshot
        .contract(before.contract)
        .cloned()
        .ok_or_else(refused)?;
    let mut row = before.clone();
    row.closure = Some(Closure {
        commit: context.commit,
        action: context.action.clone(),
        at_ic: context.world.world_ic_now(),
        at_utc: context.world.now(),
        reason,
    });
    row.validate()?;
    let after = contract(&row)?;
    images(
        state,
        row,
        after,
        Some(before_contract),
        state.next_contract,
    )
}
fn close_current(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    before: &OfferRecord,
    reason: UnfundedCloseReason,
) -> Result<(), ParticipantError> {
    actor(context, before.offer.terms.buyer)?;
    let now = context.world.world_ic_now();
    if state.frozen
        || !action_matches(context.action, CLOSE_ACTION)
        || before.closure.is_some()
        || context.commit == before.operation
        || context.commit == before.offer.basis.predecessor_operation
        || context.commit == before.offer.basis.prior_quote
        || now.clock != before.created_at_ic.clock
        || now.at < before.created_at_ic.at
        || context.world.now() < before.created_at_utc
        || (reason == UnfundedCloseReason::Expired && now.at < before.offer.terms.expires_at.at)
    {
        return Err(refused());
    }
    Ok(())
}

pub struct OfferPatch {
    before_meta: MetaFence,
    before_book_generation: u64,
    before_record: Option<OfferRecord>,
    before_contract: Option<EconomicContract>,
    before_accounts: Vec<Account>,
    before_titles: Vec<EconomicAsset>,
    before_instrument: Option<InstrumentDef>,
    before_open_counts: Option<(usize, usize)>,
    row: OfferRecord,
    contract: EconomicContract,
    next_contract: u64,
    next_book_generation: u64,
}
fn images(
    state: &EconomyStore,
    row: OfferRecord,
    after: EconomicContract,
    before_contract: Option<EconomicContract>,
    next_contract: u64,
) -> Result<(RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>), ParticipantError> {
    row.validate()?;
    let next_book_generation = state.renewal_offers.next_generation()?;
    let mut writes = vec![
        json_write(crate::api::contract_key(after.id), &after)?,
        json_write(history::key(row.operation).into(), &row)?,
    ];
    let offered = row.closure.is_none();
    if offered {
        let mut meta = state.meta_doc();
        meta.as_object_mut()
            .ok_or(ParticipantError::Encoding)?
            .insert(
                "next_contract_id".into(),
                serde_json::Value::from(next_contract),
            );
        writes.push(json_write(crate::api::META_KEY.into(), &meta)?);
    }
    // Actual envelopes are prepared now as well as individual documents. The
    // later closed owner codec must retain this finite envelope ceiling.
    if serde_json::to_vec(&writes)
        .map_err(|_| ParticipantError::Encoding)?
        .len()
        > 32_768
    {
        return Err(refused());
    }
    let terms = &row.offer.terms;
    let mut before_accounts = Vec::new();
    let mut before_titles = Vec::new();
    let mut before_instrument = None;
    if offered {
        let mut ids = std::collections::BTreeSet::from([terms.payer]);
        ids.extend(terms.recipients().iter().map(|r| r.account));
        for id in ids {
            before_accounts.push(state.snapshot.account(id).cloned().ok_or_else(refused)?);
        }
        let mut titles = std::collections::BTreeSet::from([terms.charter]);
        titles.extend(
            terms
                .parents
                .iter()
                .filter(|p| p.title.is_some())
                .map(|p| p.charter),
        );
        for id in titles {
            before_titles.push(state.snapshot.asset(id).cloned().ok_or_else(refused)?);
        }
        before_instrument = Some(
            state
                .snapshot
                .instrument(&terms.instrument)
                .cloned()
                .ok_or_else(refused)?,
        );
    }
    let output = RenewalOfferOutput {
        operation: row.operation,
        contract: row.contract,
        consent_hash: row.consent_hash,
        at_ic: row
            .closure
            .as_ref()
            .map_or_else(|| row.created_at_ic.clone(), |c| c.at_ic.clone()),
        closed: row.closure.as_ref().map(|c| c.reason),
        attempt: row.attempt(),
        offer_digest: row.offer.digest()?,
        commit: row.closure.as_ref().map_or(row.operation, |c| c.commit),
        action: row
            .closure
            .as_ref()
            .map_or(&row.action, |c| &c.action)
            .clone(),
        posted_at_utc: row
            .closure
            .as_ref()
            .map_or(row.created_at_utc, |c| c.at_utc),
    };
    let patch = OfferPatch {
        before_meta: MetaFence::capture(state),
        before_book_generation: state.renewal_offers.generation(),
        before_record: state.renewal_offers.get(row.operation).cloned(),
        before_contract,
        before_accounts,
        before_titles,
        before_instrument,
        before_open_counts: offered.then(|| open_counts(state, terms.buyer)),
        row,
        contract: after,
        next_contract,
        next_book_generation,
    };
    Ok((output, patch, writes))
}
impl OfferPatch {
    pub(crate) fn recheck(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.recheck_in(state, &OfferContext::from(context))
    }
    pub(super) fn recheck_in(
        &self,
        state: &EconomyStore,
        context: &OfferContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.check_local(state)?;
        let (action, commit, at, utc) = self.row.closure.as_ref().map_or(
            (
                &self.row.action,
                self.row.operation,
                &self.row.created_at_ic,
                self.row.created_at_utc,
            ),
            |c| (&c.action, c.commit, &c.at_ic, c.at_utc),
        );
        let now = context.world.world_ic_now();
        if context.action != action
            || context.commit != commit
            || now.clock != at.clock
            || now.at < at.at
            || context.world.now() < utc
        {
            return Err(refused());
        }
        if let Some(close) = &self.row.closure {
            close_current(
                state,
                context,
                self.before_record.as_ref().ok_or_else(refused)?,
                close.reason,
            )?;
        } else {
            offer_current(state, context, &self.row.offer)?;
        }
        Ok(())
    }
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        // A previous money terminal lives in journal/fold state, not in the
        // OfferBook generation. Repeat its exact proof before custody as well.
        if self.before_record.is_none() {
            state.renewal_offers.admits(&self.row)?;
            state.renewal_offers.require_retry(
                &self.row,
                &state.snapshot,
                &state.fold.renewal_receipts,
            )?;
        }
        if state.frozen || !self.before_meta.matches(state)
            || state.renewal_offers.generation()!=self.before_book_generation
            || state.renewal_offers.get(self.row.operation)!=self.before_record.as_ref()
            || state.snapshot.contract(self.row.contract)!=self.before_contract.as_ref()
            || state.snapshot.holds().any(|h|matches!(h.purpose,HoldPurpose::Escrow{contract} if contract==self.row.contract))
            || self.before_accounts.iter().any(|a|state.snapshot.account(a.id)!=Some(a))
            || self.before_titles.iter().any(|t|state.snapshot.asset(t.object)!=Some(t))
            || self.before_instrument.as_ref().is_some_and(|i|state.snapshot.instrument(&i.id)!=Some(i))
            || self.before_open_counts.is_some_and(|n|open_counts(state,self.row.offer.terms.buyer)!=n) {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(self, state: &mut EconomyStore) -> Result<BoundOffer<'_>, ParticipantError> {
        self.check_local(state)?;
        Ok(BoundOffer {
            snapshot: &mut state.snapshot,
            book: &mut state.renewal_offers,
            next_contract: &mut state.next_contract,
            row: self.row,
            contract: self.contract,
            after_contract: self.next_contract,
            after_generation: self.next_book_generation,
        })
    }
}
pub struct BoundOffer<'a> {
    snapshot: &'a mut crate::EconomySnapshot,
    book: &'a mut OfferBook,
    next_contract: &'a mut u64,
    row: OfferRecord,
    contract: EconomicContract,
    after_contract: u64,
    after_generation: u64,
}
impl BoundParticipant for BoundOffer<'_> {
    fn apply(self) {
        self.snapshot.put_contract(self.contract);
        self.book.install(self.row, self.after_generation);
        *self.next_contract = self.after_contract;
    }
}
