//! Separate stack frames for the single Economy participant's closed requests.
//! N88 measured a1.28MB debug dispatch frame before any selected native owner
//! ran. Keep the branch body and typed authority unchanged while avoiding the
//! aggregate stack allocation for mutually exclusive requests.
use super::*;
use crate::managed_purchase::{PurchaseOutcome, install};

impl InitialFilingParticipant {
    #[inline(never)]
    pub(super) fn prepare_coverage_bind_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_coverage::CoverageBindRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_coverage(
            state,
            context,
            crate::managed_coverage::native::Request::Bind(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_coverage_terminal_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_coverage::CoverageTerminalRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_coverage(
            state,
            context,
            crate::managed_coverage::native::Request::Terminal(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_paid_resale_acceptance_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_paid_resale::PaidResaleAcceptanceRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_resale(
            state,
            context,
            crate::managed_paid_resale::native::Request::Accept(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_paid_resale_terminal_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_paid_resale::PaidResaleTerminalRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_resale(
            state,
            context,
            crate::managed_paid_resale::native::Request::Terminal(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_charter_pending_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_charter_transfer::CharterPendingFundingRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_pending(
            state,
            context,
            crate::managed_charter_transfer::native::Request::Fund(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_charter_pending_terminal_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_charter_transfer::CharterPendingTerminalRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_pending(
            state,
            context,
            crate::managed_charter_transfer::native::Request::Terminal(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_core_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_integration::CoreFundingRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_core(
            state,
            context,
            crate::core_integration::money::Request::Fund(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_core_completion_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_integration::CoreCompletionRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_core(
            state,
            context,
            crate::core_integration::money::Request::Complete(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_core_cancellation_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_integration::CoreCancellationRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_core(
            state,
            context,
            crate::core_integration::money::Request::Cancel(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_holder_draft_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::DRAFT_ACTION)?;
            if self.paid_resale.is_none() {
                return Err(ParticipantError::refused(
                    "Paid-holder renewal is unavailable.",
                ));
            }
            let (output, patch, writes) =
                crate::managed_renewal::prepare_holder_offer(state, context, offer, basis, None)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_holder_retry_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
        previous_operation: CommitToken,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::DRAFT_ACTION)?;
            if self.paid_resale.is_none() {
                return Err(ParticipantError::refused(
                    "Paid-holder renewal is unavailable.",
                ));
            }
            let (output, patch, writes) = crate::managed_renewal::prepare_holder_offer(
                state,
                context,
                offer,
                basis,
                Some(previous_operation),
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_refit_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_refit::RefitFundingRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_refit(
            state,
            context,
            crate::core_refit::money::Request::Fund(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_refit_installation_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_refit::RefitInstallationRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_refit(
            state,
            context,
            crate::core_refit::money::Request::Install(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_refit_cancellation_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::core_refit::RefitCancellationRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        self.prepare_refit(
            state,
            context,
            crate::core_refit::money::Request::Cancel(request),
        )
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_draft_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_renewal::RenewalOffer,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::DRAFT_ACTION)?;
            let (output, patch, writes) =
                crate::managed_renewal::prepare_offer(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_retry_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_renewal::RenewalOffer,
        previous_operation: CommitToken,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::DRAFT_ACTION)?;
            let (output, patch, writes) =
                crate::managed_renewal::prepare_retry(state, context, offer, previous_operation)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_service_close_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        operation: CommitToken,
        reason: crate::managed_renewal::RenewalServiceCloseReason,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::SERVICE_CLOSE_ACTION)?;
            let (output, patch, writes) =
                crate::managed_renewal::prepare_service_close(state, context, operation, reason)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_close_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        operation: CommitToken,
        reason: crate::managed_renewal::UnfundedCloseReason,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::CLOSE_ACTION)?;
            let (output, patch, writes) =
                crate::managed_renewal::prepare_close(state, context, operation, reason)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalOffer(output),
                EconomyPatch::RenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::FUND_ACTION)?;
            let (output, patch, writes) =
                crate::managed_renewal::prepare_funding(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalFunding(output),
                EconomyPatch::RenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_grant_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::GRANT_ACTION)?;
            let (output, patch, writes) = crate::managed_renewal::prepare_terminal(
                state,
                context,
                request,
                crate::managed_renewal::RenewalOutcome::Granted,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalTerminal(output),
                EconomyPatch::RenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_renewal_refund_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_renewal(context, crate::managed_renewal::REFUND_ACTION)?;
            let (output, patch, writes) = crate::managed_renewal::prepare_terminal(
                state,
                context,
                request,
                crate::managed_renewal::RenewalOutcome::Refunded,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::RenewalTerminal(output),
                EconomyPatch::RenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_holder_draft_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_house_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::DRAFT_ACTION)?;
            if self.paid_resale.is_none() {
                return Err(ParticipantError::refused(
                    "Paid-holder renewal is unavailable.",
                ));
            }
            let (output, patch, writes) = crate::managed_house_renewal::prepare_holder_offer(
                state, context, offer, basis, None,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_holder_retry_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_house_renewal::RenewalOffer,
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
        previous_operation: CommitToken,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::DRAFT_ACTION)?;
            if self.paid_resale.is_none() {
                return Err(ParticipantError::refused(
                    "Paid-holder renewal is unavailable.",
                ));
            }
            let (output, patch, writes) = crate::managed_house_renewal::prepare_holder_offer(
                state,
                context,
                offer,
                basis,
                Some(previous_operation),
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_draft_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_renewal::RenewalOffer,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::DRAFT_ACTION)?;
            let (output, patch, writes) =
                crate::managed_house_renewal::prepare_offer(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_retry_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        offer: crate::managed_house_renewal::RenewalOffer,
        previous_operation: CommitToken,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::DRAFT_ACTION)?;
            let (output, patch, writes) = crate::managed_house_renewal::prepare_retry(
                state,
                context,
                offer,
                previous_operation,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_service_close_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        operation: CommitToken,
        reason: crate::managed_house_renewal::RenewalServiceCloseReason,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(
                context,
                crate::managed_house_renewal::SERVICE_CLOSE_ACTION,
            )?;
            let (output, patch, writes) = crate::managed_house_renewal::prepare_service_close(
                state, context, operation, reason,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_close_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        operation: CommitToken,
        reason: crate::managed_house_renewal::UnfundedCloseReason,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::CLOSE_ACTION)?;
            let (output, patch, writes) =
                crate::managed_house_renewal::prepare_close(state, context, operation, reason)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalOffer(output),
                EconomyPatch::HouseRenewalOffer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::FUND_ACTION)?;
            let (output, patch, writes) =
                crate::managed_house_renewal::prepare_funding(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalFunding(output),
                EconomyPatch::HouseRenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_grant_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::GRANT_ACTION)?;
            let (output, patch, writes) = crate::managed_house_renewal::prepare_terminal(
                state,
                context,
                request,
                crate::managed_house_renewal::RenewalOutcome::Granted,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalTerminal(output),
                EconomyPatch::HouseRenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_renewal_refund_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_renewal::RenewalMoneyRequest,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            self.require_house_renewal(context, crate::managed_house_renewal::REFUND_ACTION)?;
            let (output, patch, writes) = crate::managed_house_renewal::prepare_terminal(
                state,
                context,
                request,
                crate::managed_house_renewal::RenewalOutcome::Refunded,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseRenewalTerminal(output),
                EconomyPatch::HouseRenewalMoney(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_charter_transfer_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: charter_transfer::RegistryCharterTransfer,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let action = self.charter_transfer.as_ref().ok_or_else(refused)?;
            let (output, patch, writes) =
                charter_transfer::prepare(&self.policy, action, state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::CharterTransfer(output),
                EconomyPatch::CharterTransfer(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_compatible_update_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: update::RegistryCompatibleUpdate,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let action = self.compatible_update.as_ref().ok_or_else(refused)?;
            let (output, patch, writes) =
                update::prepare(&self.policy, action, state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::CompatibleUpdate(output),
                EconomyPatch::CompatibleUpdate(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_initial_filing_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: RegistryInitialFiling,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let action = context.action();
            if action.key().owner.as_str() != "roll"
                || action.key().kind.as_str() != "action"
                || action.key().id.as_str() != "initial_filing"
                || action.version() != 1
            {
                return Err(refused());
            }
            let (output, patch, writes) = self.prepare_images(
                state,
                context.world(),
                context.invocation().actor(),
                context.commit_token(),
                request,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::InitialFiling(output),
                patch.into(),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_dependent_initial_filing_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: RegistryInitialFiling,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let action = context.action();
            if action.key().owner.as_str() != "roll"
                || action.key().kind.as_str() != "action"
                || action.key().id.as_str() != "dependent_initial_filing"
                || action.version() != 1
            {
                return Err(refused());
            }
            let (output, patch, writes) = self.prepare_images(
                state,
                context.world(),
                context.invocation().actor(),
                context.commit_token(),
                request,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::InitialFiling(output),
                patch.into(),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_initial_filing_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: RegistryHouseFiling,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let action = context.action();
            if action.key().owner.as_str() != "roll"
                || action.key().kind.as_str() != "action"
                || action.key().id.as_str() != HOUSE_FILING_ACTION
                || action.version() != 1
            {
                return Err(refused());
            }
            let (request, house) = request.into_request(context.invocation().actor())?;
            let (output, patch, writes) = self.prepare_filing_images(
                state,
                context.world(),
                context.invocation().actor(),
                context.commit_token(),
                request,
                Some(house),
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::InitialFiling(output),
                patch.into(),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_purchase::HousePurchase,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) =
                crate::managed_house_purchase::prepare_funding(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseFunding(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_grant_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_purchase::HouseTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = crate::managed_house_purchase::prepare_terminal(
                state,
                context,
                request,
                PurchaseOutcome::Granted,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_house_refund_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_house_purchase::HouseTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = crate::managed_house_purchase::prepare_terminal(
                state,
                context,
                request,
                PurchaseOutcome::Refunded,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::HouseTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_entitlement_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_entitlement::EntitlementPurchase,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) =
                crate::managed_entitlement::prepare_funding(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::EntitlementFunding(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_entitlement_grant_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_entitlement::EntitlementTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = crate::managed_entitlement::prepare_terminal(
                state,
                context,
                request,
                PurchaseOutcome::Granted,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::EntitlementTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_entitlement_refund_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_entitlement::EntitlementTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = crate::managed_entitlement::prepare_terminal(
                state,
                context,
                request,
                PurchaseOutcome::Refunded,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::EntitlementTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_purchase_funding_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_purchase::OriginalPurchase,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = install::prepare_images(state, context, request)?;
            Ok(OwnerPreparation::new(
                EconomyOutput::PurchaseFunding(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_purchase_grant_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_purchase::PurchaseTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = install::prepare_terminal_images(
                state,
                context,
                request,
                PurchaseOutcome::Granted,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::PurchaseTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
    #[inline(never)]
    pub(super) fn prepare_purchase_refund_request(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
        request: crate::managed_purchase::PurchaseTerminal,
    ) -> Result<OwnerPreparation<EconomyOutput, EconomyPatch>, ParticipantError> {
        {
            let (output, patch, writes) = install::prepare_terminal_images(
                state,
                context,
                request,
                PurchaseOutcome::Refunded,
            )?;
            Ok(OwnerPreparation::new(
                EconomyOutput::PurchaseTerminal(output),
                EconomyPatch::Purchase(Box::new(patch)),
                writes,
            ))
        }
    }
}
