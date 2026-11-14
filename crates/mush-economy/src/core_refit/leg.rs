//! The core refit's member of `crate::refit_twin`: its identity strings, its own records and
//! checks forwarded, and the two records whose serialized field names are the core refit's own.
use super::authority::{self, CurrentFacts};
use super::plans::FundingIntent;
use super::{
    CoreRefitConfig, RefitCancellationRequest, RefitError, RefitInstallationRequest,
    RefitOfferAttribution, RefitOrderAttribution, RefitPaymentPolicy, RefitSubject,
};
use crate::engine::ValidateEnv;
use crate::fold::FoldState;
use crate::module::EconomyStore;
use crate::plan::PlanResult;
use crate::refit_twin::{self, Core, Leg, plans::Phase};
use crate::{AccountId, Amount, EconomySnapshot, InstrumentId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_org::SystemPrincipalId;
use serde::{Deserialize, Serialize};

impl Leg for Core {
    const ACCEPT_ACTION: &'static str = super::ACCEPT_ACTION;
    const INSTALL_ACTION: &'static str = super::INSTALL_ACTION;
    const CANCEL_ACTION: &'static str = super::CANCEL_ACTION;
    const NAMESPACE: &'static str = super::NAMESPACE;
    const HOLD_PURPOSE: &'static str = super::HOLD_PURPOSE;
    const MAX_ORDERS: usize = super::MAX_ORDERS;
    const HISTORY_BYTES: usize = super::HISTORY_BYTES;
    const REFUSED: &'static str = "core refit money or accepted authority disagrees";
    const PLAN_DESCRIPTION: &'static str = "Certified core fitting";
    const LINE_MEMO: &'static str = "core fitting";
    const HISTORIES_EXPECTING: &'static str = "bounded distinct core refit histories";
    const HISTORY_LIMIT: &'static str = "core history limit";
    const HISTORY_DUPLICATE: &'static str = "core history duplicate or limit";
    const HISTORY_KEY: &'static str = "core history key";

    type Error = RefitError;
    const INVALID: RefitError = RefitError::Invalid;
    const INTEGRITY: RefitError = RefitError::Integrity;
    const INCOMPLETE: RefitError = RefitError::Incomplete;
    const CAPACITY: RefitError = RefitError::Capacity;

    type Subject = RefitSubject;
    type Policy = RefitPaymentPolicy;
    type Supplier = RefitOfferAttribution;
    type Order = RefitOrderAttribution;
    type Config = CoreRefitConfig;
    type Proof = Proof;
    type Result = RefitTerminalResult;
    type Installation = RefitInstallationRequest;
    type Cancellation = RefitCancellationRequest;

    fn receipts(fold: &FoldState) -> &refit_twin::receipt::ReceiptAudit<Self> {
        &fold.refit_receipts
    }
    fn receipts_mut(fold: &mut FoldState) -> &mut refit_twin::receipt::ReceiptAudit<Self> {
        &mut fold.refit_receipts
    }
    fn validate(
        proof: &refit_twin::money::ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult {
        crate::engine::validate_core_refit(proof, snapshot, env)
    }
    fn funding(
        config: &CoreRefitConfig,
        state: &EconomyStore,
        world: &World,
        subject: &RefitSubject,
        initial: bool,
    ) -> Result<(FundingIntent, CurrentFacts), RefitError> {
        authority::funding(config, state, world, subject, initial)
    }
    fn installation(
        config: &CoreRefitConfig,
        state: &EconomyStore,
        world: &World,
        intent: &FundingIntent,
        acceptance: CommitToken,
    ) -> Result<CurrentFacts, RefitError> {
        authority::installation(config, state, world, intent, acceptance)
    }
    fn policy(config: &CoreRefitConfig) -> &RefitPaymentPolicy {
        &config.policy
    }
    fn cleanup_ready(config: &CoreRefitConfig, world: &World) -> Result<(), RefitError> {
        config.cleanup_ready(world)
    }
    fn matches_policy(
        subject: &RefitSubject,
        policy: &RefitPaymentPolicy,
    ) -> Result<(), RefitError> {
        subject.matches_policy(policy)
    }
    fn supplier_validate(
        supplier: &RefitOfferAttribution,
        policy: &RefitPaymentPolicy,
        subject: &RefitSubject,
    ) -> Result<(), RefitError> {
        supplier.validate(policy, subject)
    }
    fn action_ref(policy: &RefitPaymentPolicy, action: &str) -> Result<DefinitionRef, RefitError> {
        policy.action_ref(action)
    }
    fn bytes<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, RefitError> {
        super::subject::bytes(value, max)
    }
    fn installation_acceptance(request: &RefitInstallationRequest) -> CommitToken {
        request.acceptance
    }
    fn cancellation_acceptance(request: &RefitCancellationRequest) -> CommitToken {
        request.acceptance
    }

    fn seal(phase: Phase<Self>) -> Proof {
        match phase {
            Phase::Accept {
                format,
                commit,
                action,
                intent,
            } => Proof::Accept {
                format,
                commit,
                action,
                intent,
            },
            Phase::Install {
                format,
                acceptance,
                commit,
                action,
                at,
                part,
            } => Proof::Install {
                format,
                acceptance,
                commit,
                action,
                at,
                replacement: part,
            },
            Phase::Cancel {
                format,
                acceptance,
                commit,
                action,
                at,
            } => Proof::Cancel {
                format,
                acceptance,
                commit,
                action,
                at,
            },
        }
    }
    fn open(proof: Proof) -> Phase<Self> {
        match proof {
            Proof::Accept {
                format,
                commit,
                action,
                intent,
            } => Phase::Accept {
                format,
                commit,
                action,
                intent,
            },
            Proof::Install {
                format,
                acceptance,
                commit,
                action,
                at,
                replacement,
            } => Phase::Install {
                format,
                acceptance,
                commit,
                action,
                at,
                part: replacement,
            },
            Proof::Cancel {
                format,
                acceptance,
                commit,
                action,
                at,
            } => Phase::Cancel {
                format,
                acceptance,
                commit,
                action,
                at,
            },
        }
    }
    fn installed(part: DefinitionDigest) -> RefitTerminalResult {
        RefitTerminalResult::Installed { replacement: part }
    }
    fn cancelled() -> RefitTerminalResult {
        RefitTerminalResult::Cancelled {}
    }
    fn result_part(result: &RefitTerminalResult) -> Option<DefinitionDigest> {
        result.replacement()
    }
    fn part(order: &RefitOrderAttribution) -> DefinitionDigest {
        order.replacement
    }

    /// The subject's recipe's fee (doc 17h cut M8b C2: `core_ungovern`
    /// charges `ungovern_minor`); a recipe the policy does not compose is refused.
    fn price(intent: &FundingIntent) -> Result<Amount, RefitError> {
        intent.policy.price_for(intent.subject.kind)
    }
    /// The account the subject's recipe pays; refused as `price`.
    fn provider_account(intent: &FundingIntent) -> Result<AccountId, RefitError> {
        Ok(intent.policy.site(intent.subject.kind)?.provider.account)
    }
    /// The manufacture acceptance the incoming core came from is spent.
    fn commit_reused(subject: &RefitSubject, commit: CommitToken) -> bool {
        commit == subject.core_acceptance
    }
    /// One installation an incoming core.
    fn exclusive(history: &refit_twin::receipt::PaymentHistory<Self>) -> Option<ObjId> {
        history
            .terminal
            .as_ref()
            .is_some_and(|t| matches!(t.result, RefitTerminalResult::Installed { .. }))
            .then_some(history.intent.subject.incoming)
    }
    /// An incoming core is accepted again only after its last order was cancelled.
    fn conflicts(
        history: &refit_twin::receipt::PaymentHistory<Self>,
        intent: &FundingIntent,
    ) -> bool {
        history.intent.subject.incoming == intent.subject.incoming
            && !history
                .terminal
                .as_ref()
                .is_some_and(|t| matches!(t.result, RefitTerminalResult::Cancelled {}))
    }
}

/// The correlation a core fitting plan carries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum Proof {
    Accept {
        format: u16,
        commit: CommitToken,
        action: DefinitionRef,
        intent: Box<FundingIntent>,
    },
    Install {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
        replacement: DefinitionDigest,
    },
    Cancel {
        format: u16,
        acceptance: CommitToken,
        commit: CommitToken,
        action: DefinitionRef,
        at: ClockStamp,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RefitTerminalResult {
    Installed { replacement: DefinitionDigest },
    Cancelled {},
}
impl RefitTerminalResult {
    pub const fn replacement(&self) -> Option<DefinitionDigest> {
        match self {
            Self::Installed { replacement } => Some(*replacement),
            Self::Cancelled {} => None,
        }
    }
}

impl refit_twin::Subject for RefitSubject {
    fn order(&self) -> u64 {
        self.order
    }
    fn offer(&self) -> CommitToken {
        self.offer
    }
    fn requester(&self) -> ObjId {
        self.requester
    }
    fn payer(&self) -> AccountId {
        self.payer
    }
    fn hull(&self) -> ObjId {
        self.hull
    }
    fn accepted_at(&self) -> &ClockStamp {
        &self.accepted_at
    }
    fn expires_at(&self) -> &ClockStamp {
        &self.expires_at
    }
}
impl refit_twin::Policy for RefitPaymentPolicy {
    fn issuer(&self) -> &SystemPrincipalId {
        &self.issuer
    }
    fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
}
impl refit_twin::Supplier for RefitOfferAttribution {
    fn observed_utc(&self) -> i64 {
        self.observed_utc
    }
    fn expires_utc(&self) -> i64 {
        self.expires_utc
    }
}
