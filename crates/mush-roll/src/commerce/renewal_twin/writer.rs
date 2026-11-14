//! Closed paired Roll publication. Plain records never establish authority:
//! publication consumes exact opaque Economy output and same-preparation owner
//! facts, then retains the actual Root participant fence through custody.
use super::super::{
    LicenceId,
    families::CommerceFamilies,
    paid::PersonalPaidLicence,
    participant::RootParticipant,
    purchase::PurchaseParticipants,
    records::{PublicationError, Record},
};
use super::history::Offer;
use super::records::{RenewalAttemptV1, RenewalHeadV1, RenewalStateV1};
use super::{
    Binding, Closure, Facts, Issuer, Operation, Rules,
    protocol::{self, Step as Transition},
    roots::{Publication, RenewalRoots},
};
use mush_core::PrepareCtx;
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, Registrar};
use mush_economy::{
    EconomyStore,
    renewal_twin::{
        Basis as _, Offer as _, RenewalFundingOutput, RenewalOfferOutput, RenewalOutcome,
        RenewalTerminalOutput,
    },
};
use std::sync::{Arc, OnceLock};
fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

#[derive(Clone)]
pub(crate) struct Registration {
    families: CommerceFamilies,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    actions: [DefinitionRef; 5],
    service_close: Option<DefinitionRef>,
}
/// Shared trusted composition cell. Only Roll installs its real families and
/// fixed grants; no public state/image or current-rights constructor exists.
///
/// The second field is what a fresh renewal's admission carries (`Issuer::Admission`): nothing
/// for the personal issuer, the House purchase composition's buyer admission for the House.
#[derive(Clone, Default)]
pub struct RenewalWriter<I: Issuer>(
    pub(crate) Arc<OnceLock<Registration>>,
    pub(crate) I::Admission,
);
impl<I: Issuer> std::fmt::Debug for RenewalWriter<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenewalWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
#[expect(
    private_bounds,
    reason = "doc 46 CT.5: the issuer's rules are private to the crate, as the records they name are"
)]
impl<I: Rules> RenewalWriter<I> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Market's mandatory final paired audit reads actual retained Roll rows.
    /// No caller list substitutes for reverse completeness, and the Roll gate
    /// must already have proved the complete generation/attempt/E4/root graph.
    pub fn operations_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<I::Operation>, PublicationError> {
        let r = self.boot_context(ctx)?;
        let mut rows = Vec::new();
        super::super::audit::scan(ctx, I::operations(&r.families), |row| {
            if rows.len() == super::records::MAX_OPERATIONS {
                return Err(PublicationError::Capacity);
            }
            rows.push(row.clone());
            Ok(())
        })?;
        Ok(rows)
    }
    pub fn operation_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        operation: CommitToken,
    ) -> Result<I::Operation, PublicationError> {
        let r = self.boot_context(ctx)?;
        let row = super::super::audit::read(ctx, I::operations(&r.families), operation.to_hex())?;
        let economy = ctx
            .state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let actual = I::audit(economy).map_err(invalid)?;
        super::history::verify::<I>(row, &actual.get(operation).map_err(invalid)?)?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        for root in super::audit::required::<I>(row)? {
            roll.check_required_root(&root).map_err(invalid)?;
        }
        Ok(row.clone())
    }
    fn boot_context(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<&Registration, PublicationError> {
        if ctx.module.as_deref() != Some("market") {
            return Err(PublicationError::Unavailable);
        }
        ctx.state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(invalid)?;
        self.0.get().ok_or(PublicationError::Unavailable)
    }
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        actions: [DefinitionRef; 5],
        service_close: Option<DefinitionRef>,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS || self.0.get().is_some() {
            return Err(ModError::msg(
                "renewal writer requires one Roll registration",
            ));
        }
        protocol::validate_refs::<I>(&manifest, &participants, &actions)?;
        if let Some(service) = &service_close
            && service != &protocol::service_reference::<I>(&manifest, &participants)?
        {
            return Err(ModError::msg("renewal service definition differs"));
        }
        reg.delegate_state_family(I::operations(&families), "market")?;
        reg.delegate_state_family(I::heads(&families), "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        reg.delegate_state_family(&families.paid_terms, "market")?;
        reg.delegate_state_family(I::economics(&families), "market")?;
        I::install_extra(reg, &families)?;
        self.0
            .set(Registration {
                families,
                manifest,
                participants,
                actions,
                service_close,
            })
            .map_err(|_| ModError::msg("renewal writer registration conflicts"))
    }
    pub fn validate_registration(
        &self,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 5],
    ) -> Result<(), ModError> {
        protocol::validate_refs::<I>(manifest, participants, actions)?;
        let actual = self
            .0
            .get()
            .ok_or_else(|| ModError::msg("Roll renewal writer is unavailable"))?;
        if &actual.manifest != manifest
            || &actual.participants != participants
            || &actual.actions != actions
        {
            return Err(ModError::msg("renewal writer composition differs"));
        }
        Ok(())
    }
    pub fn validate_service_registration(&self, service: &DefinitionRef) -> Result<(), ModError> {
        if self.0.get().and_then(|r| r.service_close.as_ref()) != Some(service) {
            return Err(ModError::msg("renewal service writer is unavailable"));
        }
        Ok(())
    }
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        reader: OwnerReadHandle<I::Reader>,
    ) -> Result<RegisteredRenewalWriter<I>, ModError> {
        if reg.module() != "market" {
            return Err(ModError::msg("renewal writer requires Market registration"));
        }
        let value = RegisteredRenewalWriter {
            action: action.clone(),
            manifest: manifest.clone(),
            participants: participants.clone(),
            registration: self.clone(),
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
            reader,
        };
        if self.0.get().is_some() {
            value.registration().map_err(error)?;
        }
        Ok(value)
    }
}
/// No Clone/Deserialize/public constructor. An observation is inseparable from
/// its actual preparation action+commit; serialized facts are only data.
pub struct PreparedRenewal<I: Issuer> {
    action: DefinitionRef,
    commit: CommitToken,
    facts: I::Facts,
}
impl<I: Issuer> PreparedRenewal<I> {
    pub fn facts(&self) -> &I::Facts {
        &self.facts
    }
}
pub struct RegisteredRenewalWriter<I: Issuer> {
    action: DefinitionRef,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    registration: RenewalWriter<I>,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<I::Reader>,
}
#[expect(
    private_bounds,
    reason = "doc 46 CT.5: the issuer's rules are private to the crate, as the records they name are"
)]
impl<I: Rules> RegisteredRenewalWriter<I> {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if self.manifest != r.manifest
            || self.participants != r.participants
            || (!r.actions.contains(&self.action) && r.service_close.as_ref() != Some(&self.action))
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    fn require_action(&self, transition: Transition) -> Result<(), PublicationError> {
        if transition.select(&self.registration()?.actions) != &self.action {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub fn observe(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<PreparedRenewal<I>, PublicationError> {
        let r = self.registration()?;
        if ![Transition::Draft, Transition::Fund, Transition::Grant]
            .into_iter()
            .any(|t| t.select(&r.actions) == &self.action)
        {
            return Err(PublicationError::Mismatch);
        }
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, I::own(licence))
            .and_then(I::into_facts)
            .map_err(invalid)?;
        // M8c F4: the renewing actor (the sale's requester, acting for the
        // holder: `check_fresh`), not the character who bought.
        I::observe_accounts(ctx, &facts, &self.registration.1)?;
        Ok(PreparedRenewal {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    fn observed<'a>(
        &self,
        ctx: &PrepareCtx<'_>,
        value: &'a PreparedRenewal<I>,
    ) -> Result<&'a I::Facts, PublicationError> {
        self.registration()?;
        if value.action != self.action || value.commit != ctx.commit_token() {
            return Err(PublicationError::Mismatch);
        }
        Ok(&value.facts)
    }
    /// Actual acknowledged immutable operation and actual Economy replay proof.
    /// This deliberately makes no fresh issuer/parent eligibility query, so
    /// Refund/Close remain possible after those rights lapse.
    pub fn operation(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<I::Operation, PublicationError> {
        let f = &self.registration()?.families;
        let row = txn
            .get(I::operations(f), &operation.to_hex())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        if row.operation() != operation {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(invalid)?;
        let actual = I::audit(
            world
                .module_state::<EconomyStore>()
                .ok_or(PublicationError::Unavailable)?,
        )
        .map_err(invalid)?;
        super::history::verify::<I>(&row, &actual.get(operation).map_err(invalid)?)?;
        Ok(row)
    }
    fn predecessor(
        &self,
        txn: &mut ActionTxn<'_>,
        binding: &I::Binding,
    ) -> Result<PersonalPaidLicence, PublicationError> {
        let f = &self.registration()?.families;
        let licence = txn
            .get(&f.paid_licences, &binding.licence().key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let key = format!(
            "{}/{}",
            binding.licence().key(),
            binding.predecessor().key()
        );
        let term = txn
            .get(&f.paid_terms, &key)
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let economics = txn
            .get(I::economics(f), &key)
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?;
        term.matches_current(&licence)?;
        binding.check_predecessor(&licence, &term, economics)?;
        Ok(licence)
    }
    fn absent_next(
        &self,
        txn: &mut ActionTxn<'_>,
        binding: &I::Binding,
    ) -> Result<(), PublicationError> {
        let f = &self.registration()?.families;
        let key = format!("{}/{}", binding.licence().key(), binding.next().key());
        if txn.get(&f.paid_terms, &key).map_err(invalid)?.is_some()
            || txn.get(&f.term_economics, &key).map_err(invalid)?.is_some()
            || I::next_present(txn, f, &key)?
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    /// Call before preparing any new offer or money. Use the actual registered
    /// complete economics envelope at maximum generation, preserving all old
    /// decoders and accepted templates.
    pub fn preflight(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedRenewal<I>,
    ) -> Result<(), PublicationError> {
        let facts = self.observed(ctx, observation)?;
        self.predecessor(txn, facts.binding())?;
        self.absent_next(txn, facts.binding())?;
        let b = facts.binding();
        let row = b.economics_at(b.next(), ctx.commit_token())?;
        I::admission_capacity(I::economics(&self.registration()?.families), &row)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        offer: &Offer<I>,
        attempt: RenewalAttemptV1,
        observation: &PreparedRenewal<I>,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        self.require_action(Transition::Draft)?;
        let facts = self.observed(ctx, observation)?;
        // The actual E4 producer fixes the attempt in its immutable origin;
        // a caller-supplied retry link cannot reclassify an automatic output.
        if attempt != RenewalAttemptV1::from_economy(output.attempt()) {
            return Err(PublicationError::Mismatch);
        }
        self.preflight(ctx, txn, observation)?;
        if output.operation() != ctx.commit_token()
            || output.commit() != ctx.commit_token()
            || output.action() != &self.action
            || output.closed().is_some()
            || output.service_closed().is_some()
            || output.offer_digest() != offer.digest().map_err(invalid)?
        {
            return Err(PublicationError::Mismatch);
        }
        let row = I::Operation::drafted(
            ctx.commit_token(),
            self.action.clone(),
            attempt,
            facts.binding().clone(),
            output.offer_digest(),
            output.contract(),
            output.consent_hash(),
            output.at_ic().clone(),
            output.posted_at_utc(),
        );
        check_offer::<I>(&row, offer, facts)?;
        super::capacity::check::<I>(&self.registration()?.families, &row)?;
        let licence = self.predecessor(txn, row.binding())?;
        let f = &self.registration()?.families;
        if txn
            .get(I::operations(f), &row.key())
            .map_err(invalid)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let head = txn
            .get(I::heads(f), &licence.key())
            .map_err(invalid)?
            .cloned();
        let previous = head
            .as_ref()
            .map(|h| self.operation(ctx, txn, h.last_operation()))
            .transpose()?;
        super::chain::check_draft::<I>(&licence, &row, head.as_ref(), previous.as_ref())?;
        let next = RenewalHeadV1::<I>::for_operation(&row)?;
        let prior_retry = (attempt.previous_operation().is_some()
            || (matches!(attempt, RenewalAttemptV1::HolderAutomatic { .. })
                && previous.as_ref().is_some_and(|p| {
                    p.state().retryable() && p.binding().next() == row.binding().next()
                })))
        .then_some(previous.as_ref())
        .flatten();
        let roots = RenewalRoots::prepare(
            &self.action,
            ctx.commit_token(),
            Transition::Draft.into(),
            None,
            &row,
            prior_retry,
            Some(facts),
            self.registration.1.clone(),
        )?;
        txn.insert(I::operations(f), &row.key(), row.clone())
            .map_err(invalid)?;
        if head.is_some() {
            txn.replace(I::heads(f), &next.key(), next.clone())
                .map_err(invalid)?;
        } else {
            txn.insert(I::heads(f), &next.key(), next.clone())
                .map_err(invalid)?;
        }
        self.readback(txn, &row, &next)?;
        ctx.prepare_owner(&self.roots, I::root_request(roots))
            .map_err(invalid)?;
        Ok(row)
    }
    pub fn fund(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        offer: &Offer<I>,
        observation: &PreparedRenewal<I>,
        output: &RenewalFundingOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        self.require_action(Transition::Fund)?;
        let facts = self.observed(ctx, observation)?;
        self.preflight(ctx, txn, observation)?;
        check_offer::<I>(before, offer, facts)?;
        if !matches!(before.state(), RenewalStateV1::Drafted {})
            || output.operation() != before.operation()
            || output.receipt().commit() != ctx.commit_token()
            || output.receipt().action() != &self.action
        {
            return Err(PublicationError::Mismatch);
        }
        let after = before.with_state(RenewalStateV1::Funded {
            funding: output.receipt().clone(),
        });
        self.publish(
            ctx,
            txn,
            Transition::Fund.into(),
            before,
            after,
            Some(facts),
        )
    }
    pub fn terminal(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        offer: &Offer<I>,
        observation: Option<&PreparedRenewal<I>>,
        output: &RenewalTerminalOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        let transition = match output.receipt().outcome() {
            RenewalOutcome::Granted => Transition::Grant,
            RenewalOutcome::Refunded => Transition::Refund,
        };
        self.require_action(transition)?;
        if output.operation() != before.operation()
            || output.receipt().commit() != ctx.commit_token()
            || output.receipt().action() != &self.action
        {
            return Err(PublicationError::Mismatch);
        }
        let facts = observation.map(|v| self.observed(ctx, v)).transpose()?;
        if transition.fresh() != facts.is_some() {
            return Err(PublicationError::Mismatch);
        }
        if let Some(facts) = facts {
            check_offer::<I>(before, offer, facts)?;
            self.preflight(ctx, txn, observation.ok_or(PublicationError::Missing)?)?;
        } else {
            super::history::check_offer::<I>(before, offer)?;
        }
        let RenewalStateV1::Funded { funding } = before.state() else {
            return Err(PublicationError::Mismatch);
        };
        let after = before.with_state(match transition {
            Transition::Grant => RenewalStateV1::Granted {
                funding: funding.clone(),
                terminal: output.receipt().clone(),
            },
            Transition::Refund => RenewalStateV1::Refunded {
                funding: funding.clone(),
                terminal: output.receipt().clone(),
            },
            _ => return Err(PublicationError::Mismatch),
        });
        self.publish(ctx, txn, transition.into(), before, after, facts)
    }
    pub fn close(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        self.require_action(Transition::Close)?;
        if !matches!(before.state(), RenewalStateV1::Drafted {})
            || output.operation() != before.operation()
            || output.commit() != ctx.commit_token()
            || output.action() != &self.action
            || output.offer_digest() != before.offer_digest()
            || RenewalAttemptV1::from_economy(output.attempt()) != before.attempt()
            || output.contract() != before.contract()
            || &output.consent_hash() != before.consent_hash()
        {
            return Err(PublicationError::Mismatch);
        }
        let after = before.with_state(RenewalStateV1::UnfundedClosed {
            closure: I::Closure::new(
                output.commit(),
                output.action().clone(),
                output.at_ic().clone(),
                output.posted_at_utc(),
                output.closed().ok_or(PublicationError::Mismatch)?,
            ),
        });
        self.publish(ctx, txn, Transition::Close.into(), before, after, None)
    }
    pub fn service_close(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        if self.registration()?.service_close.as_ref() != Some(&self.action)
            || !matches!(before.state(), RenewalStateV1::Drafted {})
            || output.operation() != before.operation()
            || output.commit() != ctx.commit_token()
            || output.action() != &self.action
            || output.offer_digest() != before.offer_digest()
            || RenewalAttemptV1::from_economy(output.attempt()) != before.attempt()
            || output.contract() != before.contract()
            || &output.consent_hash() != before.consent_hash()
            || output.closed().is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let closure = output.service_closed().ok_or(PublicationError::Missing)?;
        if closure.commit() != ctx.commit_token()
            || closure.action() != &self.action
            || closure.at_ic() != output.at_ic()
            || closure.posted_at_utc() != output.posted_at_utc()
        {
            return Err(PublicationError::Mismatch);
        }
        let after = before.with_state(RenewalStateV1::UnfundedServiceClosed {
            closure: closure.clone(),
        });
        self.publish(ctx, txn, Publication::ServiceClose, before, after, None)
    }
    fn publish(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        transition: Publication,
        before: &I::Operation,
        after: I::Operation,
        facts: Option<&I::Facts>,
    ) -> Result<I::Operation, PublicationError> {
        if self.operation(ctx, txn, before.operation())? != *before {
            return Err(PublicationError::Mismatch);
        }
        after.follows(before)?;
        let current_licence = self.predecessor(txn, before.binding())?;
        self.absent_next(txn, before.binding())?;
        let f = &self.registration()?.families;
        let old = RenewalHeadV1::<I>::for_operation(before)?;
        if txn.get(I::heads(f), &old.key()).map_err(invalid)? != Some(&old) {
            return Err(PublicationError::Mismatch);
        }
        let next = RenewalHeadV1::<I>::for_operation(&after)?;
        let roots = RenewalRoots::prepare(
            &self.action,
            ctx.commit_token(),
            transition,
            Some(before),
            &after,
            None,
            facts,
            self.registration.1.clone(),
        )?;
        if transition == Publication::Ordinary(Transition::Grant) {
            let (expected_licence, term, economics) = after.granted_rows()?;
            // M8c F4: the renewed pointer records the renewing actor as
            // acting (`re_stamped`: the same principal, or `Mismatch`).
            let licence = current_licence
                .with_next_term(after.binding().next())?
                .re_stamped(expected_licence.holder())?;
            if licence != expected_licence {
                return Err(PublicationError::Mismatch);
            }
            I::admission_capacity(I::economics(f), &economics)?;
            // Doc 17h cut M8d fix round K5 (the lead's ruling): renewing an existing held term
            // passes the write bar — the same licence continues. Its use stays barred.
            txn.replace(&f.paid_licences, &licence.key(), licence.clone())
                .map_err(invalid)?;
            txn.insert(&f.paid_terms, &term.key(), term.clone())
                .map_err(invalid)?;
            txn.insert(I::economics(f), &economics.key(), economics.clone())
                .map_err(invalid)?;
            if txn.get(&f.paid_licences, &licence.key()).map_err(invalid)? != Some(&licence)
                || txn.get(&f.paid_terms, &term.key()).map_err(invalid)? != Some(&term)
                || txn
                    .get(I::economics(f), &economics.key())
                    .map_err(invalid)?
                    != Some(&economics)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        txn.replace(I::operations(f), &before.key(), after.clone())
            .map_err(invalid)?;
        // Fund retains the reservation and exact head without a third typed
        // mutation; core permits only the two paired operation progress rows.
        if transition != Publication::Ordinary(Transition::Fund) {
            txn.replace(I::heads(f), &next.key(), next.clone())
                .map_err(invalid)?;
        } else if next != old {
            return Err(PublicationError::Mismatch);
        }
        self.readback(txn, &after, &next)?;
        ctx.prepare_owner(&self.roots, I::root_request(roots))
            .map_err(invalid)?;
        Ok(after)
    }
    fn readback(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &I::Operation,
        head: &RenewalHeadV1<I>,
    ) -> Result<(), PublicationError> {
        let f = &self.registration()?.families;
        if txn.get(I::operations(f), &row.key()).map_err(invalid)? != Some(row)
            || txn.get(I::heads(f), &head.key()).map_err(invalid)? != Some(head)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
fn check_offer<I: Rules>(
    row: &I::Operation,
    offer: &Offer<I>,
    facts: &I::Facts,
) -> Result<(), PublicationError> {
    super::history::check_offer::<I>(row, offer)?;
    if row.binding() != facts.binding()
        || offer.basis().prior_quote() != facts.prior_quote()
        || offer.basis().prior_quote_digest() != facts.prior_quote_digest()
        || I::offer_differs_from_sale(offer, facts)
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
