//! Real two-owner composed transitions. Every selector is self-attributed;
//! no caller supplies a wallet, price, receipt, term image or royalty schedule.
use super::{Families, MindRenewalOperationV1, RenewalConfig};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::managed_renewal::{RenewalMoneyRequest, UnfundedCloseReason};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_roll::commerce::{
    LicenceId,
    renewal::{
        RenewalAttemptV1, RenewalStateV1, protocol::Transition, writer::RegisteredRenewalWriter,
    },
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    licence: Option<u64>,
    previous_operation: Option<CommitToken>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Select {
    operation: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Close {
    operation: CommitToken,
    reason: UnfundedCloseReason,
}
pub(super) enum Request {
    Draft(LicenceId),
    Retry(CommitToken),
    Operation(CommitToken),
    Close(CommitToken, UnfundedCloseReason),
}
pub(super) fn decode(kind: Transition, value: &Value) -> Result<Request, String> {
    let bad = |_| super::records::invalid();
    match kind {
        Transition::Draft => {
            if !value.as_object().is_some_and(|row| row.len() == 1) {
                return Err(super::records::invalid());
            }
            let r: Draft = serde_json::from_value(value.clone()).map_err(bad)?;
            match (r.licence, r.previous_operation) {
                (Some(id), None) if id <= i64::MAX as u64 => {
                    Ok(Request::Draft(LicenceId::new(id).map_err(invalid)?))
                }
                (None, Some(id)) => Ok(Request::Retry(id)),
                _ => Err(super::records::invalid()),
            }
        }
        Transition::Close => {
            let r: Close = serde_json::from_value(value.clone()).map_err(bad)?;
            Ok(Request::Close(r.operation, r.reason))
        }
        _ => {
            let r: Select = serde_json::from_value(value.clone()).map_err(bad)?;
            Ok(Request::Operation(r.operation))
        }
    }
}
fn invalid(_: impl std::fmt::Display) -> String {
    super::records::invalid()
}
pub(super) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub kind: Transition,
    pub config: RenewalConfig,
    pub families: Families,
    pub economy: ParticipantHandle<InitialFilingParticipant>,
    pub writer: RegisteredRenewalWriter,
    pub refusal: ActionError,
}
pub(super) fn require_due(
    now: &mush_core::ClockStamp,
    end: mush_roll::commerce::WorldIcSeconds,
) -> Result<(), String> {
    let now = mush_roll::commerce::WorldIcSeconds::from_stamp(now).map_err(invalid)?;
    if now < end {
        return Err(super::records::invalid());
    }
    Ok(())
}
impl Provider {
    fn observed(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<mush_roll::commerce::renewal::writer::PreparedRenewal, String> {
        let value = self.writer.observe(ctx, txn, licence).map_err(invalid)?;
        require_due(
            &ctx.world().map_err(invalid)?.world_ic_now(),
            value.facts().predecessor_term_end(),
        )?;
        self.writer.preflight(ctx, txn, &value).map_err(invalid)?;
        Ok(value)
    }
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(super::records::invalid());
        }
        decode(self.kind, inv.parameters())
    }
    fn before(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
        actor: mush_core::ObjId,
    ) -> Result<
        (
            MindRenewalOperationV1,
            mush_roll::commerce::renewal::RenewalOperationV1,
        ),
        String,
    > {
        let row = txn
            .get(&self.families.operations, &operation)
            .map_err(invalid)?
            .cloned()
            .ok_or_else(super::records::invalid)?;
        row.validate()?;
        // M8c B3: a character acting for the holding organisation runs its
        // steps (`org_holder::acts_as`).
        // Doc 17h §3 M8f P2a: the steps that move money (Fund, Grant,
        // Refund) ask an organisation's grant with the renewal's price, the
        // others (a retry's read, the unfunded close) as a read.
        let amount = if matches!(
            self.kind,
            Transition::Fund | Transition::Grant | Transition::Refund
        ) {
            Some(row.binding().price_minor())
        } else {
            mush_roll::commerce::org_holder::READ_AMOUNT
        };
        if row.operation() != operation
            || mush_roll::commerce::org_holder::acts_as_amount(
                ctx.world().map_err(invalid)?,
                row.binding().holder(),
                actor,
                amount,
            )
            .is_err()
        {
            return Err(super::records::invalid());
        }
        let roll = self
            .writer
            .operation(ctx, txn, operation)
            .map_err(invalid)?;
        row.matches_roll(&roll)?;
        if txn
            .get(&self.families.receipts, &operation)
            .map_err(invalid)?
            .is_some()
            != !row.state().pending()
        {
            return Err(super::records::invalid());
        }
        if !row.state().pending() {
            txn.get(&self.families.receipts, &operation)
                .map_err(invalid)?
                .ok_or_else(super::records::invalid)?
                .matches_operation(&row)?;
        }
        Ok((row, roll))
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let request = self.request(inv)?;
        let (operation, kind) = match request {
            Request::Draft(_) | Request::Retry(_) => {
                self.prepare_draft(inv, txn, ctx, request, commit)?
            }
            Request::Operation(operation) => self.prepare_operation(inv, txn, ctx, operation)?,
            Request::Close(operation, reason) => {
                self.prepare_close(inv, txn, ctx, operation, reason)?
            }
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"kind":kind,"operation":operation,"commit":commit}),
        )
        .map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.config.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
    #[inline(never)]
    fn prepare_draft(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        request: Request,
        commit: CommitToken,
    ) -> Result<(CommitToken, &'static str), String> {
        let (licence, attempt) = match request {
            Request::Draft(licence) => (licence, RenewalAttemptV1::Automatic {}),
            Request::Retry(previous_operation) => {
                let (before, _) = self.before(ctx, txn, previous_operation, inv.actor())?;
                if !before.state().retryable() {
                    return Err(super::records::invalid());
                }
                (
                    before.binding().licence(),
                    RenewalAttemptV1::BuyerRetry { previous_operation },
                )
            }
            _ => return Err(super::records::invalid()),
        };
        let observed = self.observed(ctx, txn, licence)?;
        let attempt = match (attempt, observed.facts().binding().holder_basis()) {
            (RenewalAttemptV1::Automatic {}, Some(basis)) => {
                RenewalAttemptV1::HolderAutomatic { basis }
            }
            (RenewalAttemptV1::BuyerRetry { previous_operation }, Some(basis)) => {
                RenewalAttemptV1::HolderRetry {
                    basis,
                    previous_operation,
                }
            }
            (attempt, None) => attempt,
            _ => return Err(super::records::invalid()),
        };
        let offer = super::offer::build(
            &self.config,
            ctx.world().map_err(invalid)?,
            observed.facts(),
            inv.actor(),
        )?;
        let request = match attempt {
            RenewalAttemptV1::HolderAutomatic { basis } => EconomyRequest::RenewalHolderDraft {
                offer: offer.clone(),
                basis,
            },
            RenewalAttemptV1::HolderRetry {
                basis,
                previous_operation,
            } => EconomyRequest::RenewalHolderRetry {
                offer: offer.clone(),
                basis,
                previous_operation,
            },
            RenewalAttemptV1::Automatic {} => EconomyRequest::RenewalDraft(offer.clone()),
            RenewalAttemptV1::BuyerRetry { previous_operation } => EconomyRequest::RenewalRetry {
                offer: offer.clone(),
                previous_operation,
            },
        };
        let output = ctx
            .prepare_owner(&self.economy, request)
            .and_then(|o| o.renewal_offer())
            .map_err(invalid)?;
        let roll = self
            .writer
            .draft(ctx, txn, &offer, attempt, &observed, &output)
            .map_err(invalid)?;
        let row = MindRenewalOperationV1::captured(&self.config, offer, &roll)?;
        super::capacity::check(&self.families, &row)?;
        txn.insert(&self.families.operations, &commit, row.clone())
            .map_err(invalid)?;
        self.readback(txn, &row)?;
        ctx.reserve_mind_renewal_completion(txn).map_err(invalid)?;
        Ok((commit, "drafted"))
    }
    #[inline(never)]
    fn prepare_operation(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        operation: CommitToken,
    ) -> Result<(CommitToken, &'static str), String> {
        let (before, roll) = self.before(ctx, txn, operation, inv.actor())?;
        let fresh = matches!(self.kind, Transition::Fund | Transition::Grant);
        if fresh
            && (before.manifest() != &self.config.manifest
                || before.participants() != &self.config.participants.ordered()
                || before.expected_action(self.kind)? != self.action)
        {
            return Err(super::records::invalid());
        }
        if (self.kind == Transition::Fund && !matches!(before.state(), RenewalStateV1::Drafted {}))
            || (self.kind != Transition::Fund
                && !matches!(before.state(), RenewalStateV1::Funded { .. }))
        {
            return Err(super::records::invalid());
        }
        let observed = if fresh {
            let value = self.observed(ctx, txn, before.binding().licence())?;
            Some(value)
        } else {
            None
        };
        let money = RenewalMoneyRequest {
            operation,
            contract: before.contract(),
            consent_hash: *before.consent_hash(),
        };
        let after_roll = if self.kind == Transition::Fund {
            let output = ctx
                .prepare_owner(&self.economy, EconomyRequest::RenewalFunding(money))
                .and_then(|o| o.renewal_funding())
                .map_err(invalid)?;
            self.writer
                .fund(
                    ctx,
                    txn,
                    &roll,
                    before.offer(),
                    observed.as_ref().ok_or_else(super::records::invalid)?,
                    &output,
                )
                .map_err(invalid)?
        } else {
            let request = if self.kind == Transition::Grant {
                EconomyRequest::RenewalGrant(money)
            } else {
                EconomyRequest::RenewalRefund(money)
            };
            let output = ctx
                .prepare_owner(&self.economy, request)
                .and_then(|o| o.renewal_terminal())
                .map_err(invalid)?;
            self.writer
                .terminal(ctx, txn, &roll, before.offer(), observed.as_ref(), &output)
                .map_err(invalid)?
        };
        let after = before.progressed(&after_roll)?;
        txn.replace(&self.families.operations, &operation, after.clone())
            .map_err(invalid)?;
        if self.kind != Transition::Fund {
            let receipt = after.receipt()?;
            txn.insert(&self.families.receipts, &operation, receipt)
                .map_err(invalid)?;
        }
        self.readback(txn, &after)?;
        ctx.advance_mind_renewal_completion(txn, operation)
            .map_err(invalid)?;
        Ok((
            operation,
            match self.kind {
                Transition::Fund => "funded",
                Transition::Grant => "granted",
                _ => "refunded",
            },
        ))
    }
    #[inline(never)]
    fn prepare_close(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        operation: CommitToken,
        reason: UnfundedCloseReason,
    ) -> Result<(CommitToken, &'static str), String> {
        let (before, roll) = self.before(ctx, txn, operation, inv.actor())?;
        if !matches!(before.state(), RenewalStateV1::Drafted {}) {
            return Err(super::records::invalid());
        }
        let output = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::RenewalClose { operation, reason },
            )
            .and_then(|o| o.renewal_offer())
            .map_err(invalid)?;
        let after_roll = self
            .writer
            .close(ctx, txn, &roll, &output)
            .map_err(invalid)?;
        let after = before.progressed(&after_roll)?;
        txn.replace(&self.families.operations, &operation, after.clone())
            .map_err(invalid)?;
        txn.insert(&self.families.receipts, &operation, after.receipt()?)
            .map_err(invalid)?;
        self.readback(txn, &after)?;
        ctx.advance_mind_renewal_completion(txn, operation)
            .map_err(invalid)?;
        Ok((operation, "closed"))
    }
    fn readback(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &MindRenewalOperationV1,
    ) -> Result<(), String> {
        if txn
            .get(&self.families.operations, &row.operation())
            .map_err(invalid)?
            != Some(row)
        {
            return Err(super::records::invalid());
        }
        let receipt = txn
            .get(&self.families.receipts, &row.operation())
            .map_err(invalid)?;
        if row.state().pending() {
            if receipt.is_some() {
                return Err(super::records::invalid());
            }
        } else {
            receipt
                .ok_or_else(super::records::invalid)?
                .matches_operation(row)?;
        }
        Ok(())
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        super::actions::offer(self, action, query, engine)
    }
}
