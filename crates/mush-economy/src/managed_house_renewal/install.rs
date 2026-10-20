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
fn open_counts(state: &EconomyStore, terms: &super::RenewalOfferTermsV1) -> (usize, usize) {
    let buyer = PrincipalId::Object(terms.buyer);
    let issuer = PrincipalId::Organization(terms.organization);
    state
        .snapshot
        .contracts()
        .filter(|c| !c.state.is_terminal())
        .fold((0, 0), |(issued, party), c| {
            (
                issued + usize::from(c.issuer == issuer),
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
    let (issued, party) = open_counts(state, &request.terms);
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
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    create(state, context, request, super::RenewalAttemptV1::Automatic)
}
pub(super) fn retry(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
    previous_operation: CommitToken,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    create(
        state,
        context,
        request,
        super::RenewalAttemptV1::BuyerRetry { previous_operation },
    )
}
pub(super) fn holder_offer(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
    basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    previous_operation: Option<CommitToken>,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    basis.validate()?;
    let attempt = match previous_operation {
        None => super::RenewalAttemptV1::HolderAutomatic { basis },
        Some(previous_operation) => super::RenewalAttemptV1::HolderRetry {
            basis,
            previous_operation,
        },
    };
    create(state, context, request, attempt)
}
fn create(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalOffer,
    attempt: super::RenewalAttemptV1,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    request.validate()?;
    offer_current(state, context, &request)?;
    let id = crate::ContractId(state.next_contract);
    let next_contract = state.next_contract.checked_add(1).ok_or_else(refused)?;
    if state.snapshot.contract(id).is_some() {
        return Err(refused());
    }
    let mut row = OfferRecord {
        format: attempt.record_format(),
        attempt: (attempt != super::RenewalAttemptV1::Automatic).then_some(attempt),
        operation: context.commit,
        action: context.action.clone(),
        offer: request,
        contract: id,
        consent_hash: [0; 32],
        created_at_ic: context.world.world_ic_now(),
        created_at_utc: context.world.now(),
        closure: None,
    };
    OfferBook::require_holder_origin(&row, &state.fold.paid_resale_receipts)?;
    let after = contract(&row)?;
    row.consent_hash = after.consent_hash;
    state.house_renewal_offers.admits(&row)?;
    state.house_renewal_offers.require_retry(
        &row,
        &state.snapshot,
        &state.fold.house_renewal_receipts,
    )?;
    check_offerable(&after, &state.snapshot).map_err(|_| refused())?;
    images(state, row, after, None, next_contract, None)
}
pub(super) fn close(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    operation: CommitToken,
    reason: UnfundedCloseReason,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    let before = state
        .house_renewal_offers
        .get(operation)
        .ok_or_else(refused)?;
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
        None,
    )
}
pub(super) fn service_close(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    operation: CommitToken,
    reason: super::RenewalServiceCloseReason,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    let before = state
        .house_renewal_offers
        .get(operation)
        .ok_or_else(refused)?;
    service_close_current(state, context, before, reason)?;
    let record = super::service::ServiceRecord {
        format: 1,
        operation,
        closure: super::RenewalServiceClosureV1::create(context, reason)?,
    };
    let after = super::service::contract(before, &record)?;
    let before_contract = state
        .snapshot
        .contract(before.contract)
        .cloned()
        .ok_or_else(refused)?;
    images(
        state,
        before.clone(),
        after,
        Some(before_contract),
        state.next_contract,
        Some(record),
    )
}
fn service_close_current(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    before: &OfferRecord,
    reason: super::RenewalServiceCloseReason,
) -> Result<(), ParticipantError> {
    if state.frozen
        || context.actor != mush_core::GOD
        || context.principal != &(mush_core::PrincipalId::Object { id: mush_core::GOD })
        || !super::subject::player(context.world, mush_core::GOD)
        || !action_matches(context.action, super::SERVICE_CLOSE_ACTION)
        || state
            .house_renewal_offers
            .service(before.operation)
            .is_some()
    {
        return Err(refused());
    }
    let audit = state.house_renewal_audit().map_err(|_| refused())?;
    let actual = audit.get(before.operation).map_err(|_| refused())?;
    if actual.history().is_some() || actual.closure().is_some() || actual.service_closed().is_some()
    {
        return Err(refused());
    }
    super::RenewalServiceClosureV1::create(context, reason)?.require_origin(before)
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
        || state
            .house_renewal_offers
            .service(before.operation)
            .is_some()
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
    service: Option<super::service::ServiceRecord>,
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
    service: Option<super::service::ServiceRecord>,
) -> Result<(RenewalOfferOutput, Box<OfferPatch>, Vec<ReplayWrite>), ParticipantError> {
    row.validate()?;
    let next_book_generation = state.house_renewal_offers.next_generation()?;
    let mut writes = vec![json_write(crate::api::contract_key(after.id), &after)?];
    if let Some(record) = &service {
        record.require_origin(&row)?;
        if serde_json::to_vec(&after)
            .map_err(|_| ParticipantError::Encoding)?
            .len()
            > history::MAX_CONTRACT_BYTES
        {
            return Err(refused());
        }
        writes.push(json_write(
            super::service::key(row.operation).into(),
            record,
        )?);
    } else {
        writes.push(json_write(history::key(row.operation).into(), &row)?);
    }
    let offered = row.closure.is_none() && service.is_none();
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
        let titles = std::collections::BTreeSet::from([terms.charter]);
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
    let mut output = RenewalOfferOutput {
        operation: row.operation,
        contract: row.contract,
        consent_hash: row.consent_hash,
        at_ic: row
            .closure
            .as_ref()
            .map_or_else(|| row.created_at_ic.clone(), |c| c.at_ic.clone()),
        closed: row.closure.as_ref().map(|c| c.reason),
        service_closed: service.as_ref().map(|r| r.closure.clone()),
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
    if let Some(record) = &service {
        output.at_ic = record.closure.at_ic().clone();
        output.commit = record.closure.commit();
        output.action = record.closure.action().clone();
        output.posted_at_utc = record.closure.posted_at_utc();
    }
    let patch = Box::new(OfferPatch {
        before_meta: MetaFence::capture(state),
        before_book_generation: state.house_renewal_offers.generation(),
        before_record: state.house_renewal_offers.get(row.operation).cloned(),
        before_contract,
        before_accounts,
        before_titles,
        before_instrument,
        before_open_counts: offered.then(|| open_counts(state, terms)),
        row,
        service,
        contract: after,
        next_contract,
        next_book_generation,
    });
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
        let (mut action, mut commit, mut at, mut utc) = self.row.closure.as_ref().map_or(
            (
                &self.row.action,
                self.row.operation,
                &self.row.created_at_ic,
                self.row.created_at_utc,
            ),
            |c| (&c.action, c.commit, &c.at_ic, c.at_utc),
        );
        if let Some(record) = &self.service {
            action = record.closure.action();
            commit = record.closure.commit();
            at = record.closure.at_ic();
            utc = record.closure.posted_at_utc();
        }
        let now = context.world.world_ic_now();
        if context.action != action
            || context.commit != commit
            || now.clock != at.clock
            || now.at < at.at
            || context.world.now() < utc
        {
            return Err(refused());
        }
        if let Some(record) = &self.service {
            service_close_current(
                state,
                context,
                self.before_record.as_ref().ok_or_else(refused)?,
                record.closure.reason(),
            )?;
        } else if let Some(close) = &self.row.closure {
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
            OfferBook::require_holder_origin(&self.row, &state.fold.paid_resale_receipts)?;
            state.house_renewal_offers.admits(&self.row)?;
            state.house_renewal_offers.require_retry(
                &self.row,
                &state.snapshot,
                &state.fold.house_renewal_receipts,
            )?;
        }
        if state.frozen || !self.before_meta.matches(state)
            || state.house_renewal_offers.generation()!=self.before_book_generation
            || state.house_renewal_offers.get(self.row.operation)!=self.before_record.as_ref()
            || state.house_renewal_offers.service(self.row.operation).is_some()
            || state.snapshot.contract(self.row.contract)!=self.before_contract.as_ref()
            || state.snapshot.holds().any(|h|matches!(h.purpose,HoldPurpose::Escrow{contract} if contract==self.row.contract))
            || self.before_accounts.iter().any(|a|state.snapshot.account(a.id)!=Some(a))
            || self.before_titles.iter().any(|t|state.snapshot.asset(t.object)!=Some(t))
            || self.before_instrument.as_ref().is_some_and(|i|state.snapshot.instrument(&i.id)!=Some(i))
            || self.before_open_counts.is_some_and(|n|open_counts(state,&self.row.offer.terms)!=n) {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self: Box<Self>,
        state: &mut EconomyStore,
    ) -> Result<BoundOffer<'_>, ParticipantError> {
        self.check_local(state)?;
        Ok(BoundOffer {
            snapshot: &mut state.snapshot,
            book: &mut state.house_renewal_offers,
            next_contract: &mut state.next_contract,
            row: self.row,
            service: self.service,
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
    service: Option<super::service::ServiceRecord>,
    contract: EconomicContract,
    after_contract: u64,
    after_generation: u64,
}
impl BoundParticipant for BoundOffer<'_> {
    fn apply(self) {
        self.snapshot.put_contract(self.contract);
        self.book.install(self.row, self.after_generation);
        if let Some(record) = self.service {
            self.book.install_service(record);
        }
        *self.next_contract = self.after_contract;
    }
}
