//! Closed paired Roll publication. Plain records never establish authority:
//! publication consumes exact opaque Economy output and same-preparation owner
//! facts, then retains the actual Root participant fence through custody.
use super::super::{
    LicenceId,
    families::CommerceFamilies,
    house_purchase::HouseTermEconomicsV1,
    paid::PersonalPaidLicence,
    participant::{RootParticipant, RootRequest},
    purchase::PurchaseParticipants,
    records::{PublicationError, Record},
};
use super::{
    RenewalAttemptV1, RenewalClosureV1, RenewalHeadV1, RenewalOperationV1, RenewalStateV1,
    protocol::{self, Transition},
    reader::{RenewalFacts, RenewalReader, RenewalRequest},
    roots::{Publication, RenewalRoots},
};
use mush_core::PrepareCtx;
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, Registrar};
use mush_economy::{
    EconomyStore,
    managed_house_renewal::{
        RenewalFundingOutput, RenewalOffer, RenewalOfferOutput, RenewalOutcome,
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
struct Registration {
    families: CommerceFamilies,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    actions: [DefinitionRef; 5],
    service_close: Option<DefinitionRef>,
}
/// Shared trusted composition cell. Only Roll installs its real families and
/// fixed grants; no public state/image or current-rights constructor exists.
#[derive(Clone, Default)]
pub struct RenewalWriter(
    Arc<OnceLock<Registration>>,
    Option<Arc<dyn super::super::house_purchase::HouseBuyerAdmission>>,
);
impl std::fmt::Debug for RenewalWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenewalWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl RenewalWriter {
    pub fn new() -> Self {
        Self::default()
    }
    /// Carry the actual purchase composition's refusal-only policy into fresh
    /// next-term admission. Refund/Close never evaluate current buyer eligibility.
    pub fn with_purchase_admission(
        mut self,
        purchase: &super::super::house_purchase::writer::HousePurchaseWriter,
    ) -> Result<Self, ModError> {
        if self.0.get().is_some() || self.1.is_some() {
            return Err(error("renewal buyer policy already configured"));
        }
        self.1 = purchase.buyer_admission();
        Ok(self)
    }

    /// Market's mandatory final paired audit reads actual retained Roll rows.
    /// No caller list substitutes for reverse completeness, and the Roll gate
    /// must already have proved the complete generation/attempt/E4/root graph.
    pub fn operations_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<RenewalOperationV1>, PublicationError> {
        let r = self.boot_context(ctx)?;
        let mut rows = Vec::new();
        super::super::audit::scan(ctx, &r.families.house_renewal_operations, |row| {
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
    ) -> Result<RenewalOperationV1, PublicationError> {
        let r = self.boot_context(ctx)?;
        let row = super::super::audit::read(
            ctx,
            &r.families.house_renewal_operations,
            operation.to_hex(),
        )?;
        let economy = ctx
            .state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let actual = economy.house_renewal_audit().map_err(invalid)?;
        super::history::verify(row, &actual.get(operation).map_err(invalid)?)?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        for root in super::audit::required(row)? {
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
        protocol::validate_refs(&manifest, &participants, &actions)?;
        if let Some(service) = &service_close {
            if service != &protocol::service_reference(&manifest, &participants)? {
                return Err(ModError::msg("renewal service definition differs"));
            }
        }
        reg.delegate_state_family(&families.house_renewal_operations, "market")?;
        reg.delegate_state_family(&families.house_renewal_heads, "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        reg.delegate_state_family(&families.paid_terms, "market")?;
        reg.delegate_state_family(&families.house_economics, "market")?;
        // `absent_next` proves, inside Market's transaction, that the next
        // generation has no entitlement economics yet; Market only reads it
        // (a House-only composition delegates it nowhere): a read-only grant.
        reg.grant_state_family_read(&families.term_economics, "market")?;
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
        protocol::validate_refs(manifest, participants, actions)?;
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
        reader: OwnerReadHandle<RenewalReader>,
    ) -> Result<RegisteredRenewalWriter, ModError> {
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
pub struct PreparedRenewal {
    action: DefinitionRef,
    commit: CommitToken,
    facts: RenewalFacts,
}
impl PreparedRenewal {
    pub fn facts(&self) -> &RenewalFacts {
        &self.facts
    }
}
pub struct RegisteredRenewalWriter {
    action: DefinitionRef,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    registration: RenewalWriter,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<RenewalReader>,
}
impl RegisteredRenewalWriter {
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
    ) -> Result<PreparedRenewal, PublicationError> {
        let r = self.registration()?;
        if ![Transition::Draft, Transition::Fund, Transition::Grant]
            .into_iter()
            .any(|t| t.select(&r.actions) == &self.action)
        {
            return Err(PublicationError::Mismatch);
        }
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, RenewalRequest::own(licence))
            .and_then(|v| v.into_facts())
            .map_err(invalid)?;
        if let Some(policy) = &self.registration.1 {
            policy.check(
                ctx.world().map_err(invalid)?,
                facts.sale().issuer(),
                facts.binding().holder(),
            )?;
        }
        let account = ctx.personal_account().map_err(invalid)?;
        if account.requester() != facts.binding().holder() {
            return Err(PublicationError::Mismatch);
        }
        Ok(PreparedRenewal {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    fn observed<'a>(
        &self,
        ctx: &PrepareCtx<'_>,
        value: &'a PreparedRenewal,
    ) -> Result<&'a RenewalFacts, PublicationError> {
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
    ) -> Result<RenewalOperationV1, PublicationError> {
        let f = &self.registration()?.families;
        let row = txn
            .get(&f.house_renewal_operations, &operation.to_hex())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        if row.operation() != operation {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(invalid)?;
        let actual = world
            .module_state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?
            .house_renewal_audit()
            .map_err(invalid)?;
        super::history::verify(&row, &actual.get(operation).map_err(invalid)?)?;
        Ok(row)
    }
    fn predecessor(
        &self,
        txn: &mut ActionTxn<'_>,
        binding: &super::RenewalBindingV1,
    ) -> Result<PersonalPaidLicence, PublicationError> {
        let f = &self.registration()?.families;
        let licence = txn
            .get(&f.paid_licences, &binding.licence.key())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let key = format!("{}/{}", binding.licence.key(), binding.predecessor.key());
        let term = txn
            .get(&f.paid_terms, &key)
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let economics = txn
            .get(&f.house_economics, &key)
            .map_err(invalid)?
            .ok_or(PublicationError::Missing)?;
        term.matches_current(&licence)?;
        binding.check_predecessor(&licence, &term, economics)?;
        Ok(licence)
    }
    fn absent_next(
        &self,
        txn: &mut ActionTxn<'_>,
        binding: &super::RenewalBindingV1,
    ) -> Result<(), PublicationError> {
        let f = &self.registration()?.families;
        let key = format!("{}/{}", binding.licence.key(), binding.next.key());
        if txn.get(&f.paid_terms, &key).map_err(invalid)?.is_some()
            || txn
                .get(&f.house_economics, &key)
                .map_err(invalid)?
                .is_some()
            || txn.get(&f.term_economics, &key).map_err(invalid)?.is_some()
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
        observation: &PreparedRenewal,
    ) -> Result<(), PublicationError> {
        let facts = self.observed(ctx, observation)?;
        self.predecessor(txn, facts.binding())?;
        self.absent_next(txn, facts.binding())?;
        let b = facts.binding();
        let row = HouseTermEconomicsV1::capture(
            b.licence,
            b.next,
            ctx.commit_token(),
            b.product_digest,
            b.grant.clone(),
        )?;
        super::super::house_purchase::admission_capacity::check(
            &self.registration()?.families.house_economics,
            &row,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        offer: &RenewalOffer,
        attempt: RenewalAttemptV1,
        observation: &PreparedRenewal,
        output: &RenewalOfferOutput,
    ) -> Result<RenewalOperationV1, PublicationError> {
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
        let row = RenewalOperationV1 {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            attempt,
            binding: facts.binding().clone(),
            offer_digest: output.offer_digest(),
            contract: output.contract(),
            consent_hash: output.consent_hash(),
            created_at_ic: output.at_ic().clone(),
            created_at_utc: output.posted_at_utc(),
            state: RenewalStateV1::Drafted {},
        };
        check_offer(&row, offer, facts)?;
        super::capacity::check(&self.registration()?.families, &row)?;
        let licence = self.predecessor(txn, &row.binding)?;
        let f = &self.registration()?.families;
        if txn
            .get(&f.house_renewal_operations, &row.key())
            .map_err(invalid)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let head = txn
            .get(&f.house_renewal_heads, &licence.key())
            .map_err(invalid)?
            .cloned();
        let previous = head
            .as_ref()
            .map(|h| self.operation(ctx, txn, h.last_operation()))
            .transpose()?;
        super::chain::check_draft(&licence, &row, head.as_ref(), previous.as_ref())?;
        let next = RenewalHeadV1::for_operation(&row)?;
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
        txn.insert(&f.house_renewal_operations, &row.key(), row.clone())
            .map_err(invalid)?;
        if head.is_some() {
            txn.replace(&f.house_renewal_heads, &next.key(), next.clone())
                .map_err(invalid)?;
        } else {
            txn.insert(&f.house_renewal_heads, &next.key(), next.clone())
                .map_err(invalid)?;
        }
        self.readback(txn, &row, &next)?;
        ctx.prepare_owner(&self.roots, RootRequest::HouseRenewal(Box::new(roots)))
            .map_err(invalid)?;
        Ok(row)
    }
    pub fn fund(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &RenewalOperationV1,
        offer: &RenewalOffer,
        observation: &PreparedRenewal,
        output: &RenewalFundingOutput,
    ) -> Result<RenewalOperationV1, PublicationError> {
        self.require_action(Transition::Fund)?;
        let facts = self.observed(ctx, observation)?;
        self.preflight(ctx, txn, observation)?;
        check_offer(before, offer, facts)?;
        if !matches!(before.state(), RenewalStateV1::Drafted {})
            || output.operation() != before.operation()
            || output.receipt().commit() != ctx.commit_token()
            || output.receipt().action() != &self.action
        {
            return Err(PublicationError::Mismatch);
        }
        let mut after = before.clone();
        after.state = RenewalStateV1::Funded {
            funding: output.receipt().clone(),
        };
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
        before: &RenewalOperationV1,
        offer: &RenewalOffer,
        observation: Option<&PreparedRenewal>,
        output: &RenewalTerminalOutput,
    ) -> Result<RenewalOperationV1, PublicationError> {
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
            check_offer(before, offer, facts)?;
            self.preflight(ctx, txn, observation.ok_or(PublicationError::Missing)?)?;
        } else {
            super::history::check_offer(before, offer)?;
        }
        let RenewalStateV1::Funded { funding } = before.state() else {
            return Err(PublicationError::Mismatch);
        };
        let mut after = before.clone();
        after.state = match transition {
            Transition::Grant => RenewalStateV1::Granted {
                funding: funding.clone(),
                terminal: output.receipt().clone(),
            },
            Transition::Refund => RenewalStateV1::Refunded {
                funding: funding.clone(),
                terminal: output.receipt().clone(),
            },
            _ => return Err(PublicationError::Mismatch),
        };
        self.publish(ctx, txn, transition.into(), before, after, facts)
    }
    pub fn close(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &RenewalOperationV1,
        output: &RenewalOfferOutput,
    ) -> Result<RenewalOperationV1, PublicationError> {
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
        let mut after = before.clone();
        after.state = RenewalStateV1::UnfundedClosed {
            closure: RenewalClosureV1 {
                commit: output.commit(),
                action: output.action().clone(),
                at_ic: output.at_ic().clone(),
                posted_at_utc: output.posted_at_utc(),
                reason: output.closed().ok_or(PublicationError::Mismatch)?,
            },
        };
        self.publish(ctx, txn, Transition::Close.into(), before, after, None)
    }
    pub fn service_close(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &RenewalOperationV1,
        output: &RenewalOfferOutput,
    ) -> Result<RenewalOperationV1, PublicationError> {
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
        let mut after = before.clone();
        after.state = RenewalStateV1::UnfundedServiceClosed {
            closure: closure.clone(),
        };
        self.publish(ctx, txn, Publication::ServiceClose, before, after, None)
    }
    fn publish(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        transition: Publication,
        before: &RenewalOperationV1,
        after: RenewalOperationV1,
        facts: Option<&RenewalFacts>,
    ) -> Result<RenewalOperationV1, PublicationError> {
        if self.operation(ctx, txn, before.operation())? != *before {
            return Err(PublicationError::Mismatch);
        }
        after.follows(before)?;
        let current_licence = self.predecessor(txn, before.binding())?;
        self.absent_next(txn, before.binding())?;
        let f = &self.registration()?.families;
        let old = RenewalHeadV1::for_operation(before)?;
        if txn
            .get(&f.house_renewal_heads, &old.key())
            .map_err(invalid)?
            != Some(&old)
        {
            return Err(PublicationError::Mismatch);
        }
        let next = RenewalHeadV1::for_operation(&after)?;
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
            let licence = current_licence.with_next_term(after.binding().next())?;
            if licence != expected_licence {
                return Err(PublicationError::Mismatch);
            }
            super::super::house_purchase::admission_capacity::check(
                &f.house_economics,
                &economics,
            )?;
            txn.replace(&f.paid_licences, &licence.key(), licence.clone())
                .map_err(invalid)?;
            txn.insert(&f.paid_terms, &term.key(), term.clone())
                .map_err(invalid)?;
            txn.insert(&f.house_economics, &economics.key(), economics.clone())
                .map_err(invalid)?;
            if txn.get(&f.paid_licences, &licence.key()).map_err(invalid)? != Some(&licence)
                || txn.get(&f.paid_terms, &term.key()).map_err(invalid)? != Some(&term)
                || txn
                    .get(&f.house_economics, &economics.key())
                    .map_err(invalid)?
                    != Some(&economics)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        txn.replace(&f.house_renewal_operations, &before.key(), after.clone())
            .map_err(invalid)?;
        // Fund retains the reservation and exact head without a third typed
        // mutation; core permits only the two paired operation progress rows.
        if transition != Publication::Ordinary(Transition::Fund) {
            txn.replace(&f.house_renewal_heads, &next.key(), next.clone())
                .map_err(invalid)?;
        } else if next != old {
            return Err(PublicationError::Mismatch);
        }
        self.readback(txn, &after, &next)?;
        ctx.prepare_owner(&self.roots, RootRequest::HouseRenewal(Box::new(roots)))
            .map_err(invalid)?;
        Ok(after)
    }
    fn readback(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &RenewalOperationV1,
        head: &RenewalHeadV1,
    ) -> Result<(), PublicationError> {
        let f = &self.registration()?.families;
        if txn
            .get(&f.house_renewal_operations, &row.key())
            .map_err(invalid)?
            != Some(row)
            || txn
                .get(&f.house_renewal_heads, &head.key())
                .map_err(invalid)?
                != Some(head)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
fn check_offer(
    row: &RenewalOperationV1,
    offer: &RenewalOffer,
    facts: &RenewalFacts,
) -> Result<(), PublicationError> {
    super::history::check_offer(row, offer)?;
    let sale = facts.sale();
    let terms = &offer.terms;
    if row.binding() != facts.binding()
        || offer.basis.prior_quote != facts.prior_quote()
        || offer.basis.prior_quote_digest != facts.prior_quote_digest()
        || terms.buyer != sale.requester()
        || terms.organization != sale.issuer().organization()
        || terms.scope != sale.issuer().scope()
        || terms.title_revision != sale.title_revision()
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
