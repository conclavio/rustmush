//! The bay refit's member of `crate::refit_twin`: its identity strings, its own records and
//! checks forwarded, and the two records whose serialized field names are the bay's own.
use super::authority::{self, CurrentFacts};
use super::plans::FundingIntent;
use super::{
    BayCancellationRequest, BayInstallationRequest, BayOfferAttribution, BayOrderAttribution,
    BayPaymentPolicy, BayRefitConfig, BayRefitError, BaySubject,
};
use crate::engine::ValidateEnv;
use crate::fold::FoldState;
use crate::module::EconomyStore;
use crate::plan::PlanResult;
use crate::refit_twin::{self, Bay, Leg, plans::Phase};
use crate::{AccountId, Amount, EconomySnapshot, InstrumentId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_org::SystemPrincipalId;
use serde::{Deserialize, Serialize};

impl Leg for Bay {
    const ACCEPT_ACTION: &'static str = super::ACCEPT_ACTION;
    const INSTALL_ACTION: &'static str = super::INSTALL_ACTION;
    const CANCEL_ACTION: &'static str = super::CANCEL_ACTION;
    const NAMESPACE: &'static str = super::NAMESPACE;
    const HOLD_PURPOSE: &'static str = super::HOLD_PURPOSE;
    const MAX_ORDERS: usize = super::MAX_ORDERS;
    const HISTORY_BYTES: usize = super::HISTORY_BYTES;
    const REFUSED: &'static str = "bay refit money or accepted authority disagrees";
    const PLAN_DESCRIPTION: &'static str = "Bay refit";
    const LINE_MEMO: &'static str = "bay refit";
    const HISTORIES_EXPECTING: &'static str = "bounded distinct bay refit histories";
    const HISTORY_LIMIT: &'static str = "bay history limit";
    const HISTORY_DUPLICATE: &'static str = "bay history duplicate or limit";
    const HISTORY_KEY: &'static str = "bay history key";

    type Error = BayRefitError;
    const INVALID: BayRefitError = BayRefitError::Invalid;
    const INTEGRITY: BayRefitError = BayRefitError::Integrity;
    const INCOMPLETE: BayRefitError = BayRefitError::Incomplete;
    const CAPACITY: BayRefitError = BayRefitError::Capacity;

    type Subject = BaySubject;
    type Policy = BayPaymentPolicy;
    type Supplier = BayOfferAttribution;
    type Order = BayOrderAttribution;
    type Config = BayRefitConfig;
    type Proof = Proof;
    type Result = BayTerminalResult;
    type Installation = BayInstallationRequest;
    type Cancellation = BayCancellationRequest;

    fn receipts(fold: &FoldState) -> &refit_twin::receipt::ReceiptAudit<Self> {
        &fold.bay_refit_receipts
    }
    fn receipts_mut(fold: &mut FoldState) -> &mut refit_twin::receipt::ReceiptAudit<Self> {
        &mut fold.bay_refit_receipts
    }
    fn validate(
        proof: &refit_twin::money::ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult {
        crate::engine::validate_bay_refit(proof, snapshot, env)
    }
    fn funding(
        config: &BayRefitConfig,
        state: &EconomyStore,
        world: &World,
        subject: &BaySubject,
        initial: bool,
    ) -> Result<(FundingIntent, CurrentFacts), BayRefitError> {
        authority::funding(config, state, world, subject, initial)
    }
    fn installation(
        config: &BayRefitConfig,
        state: &EconomyStore,
        world: &World,
        intent: &FundingIntent,
        acceptance: CommitToken,
    ) -> Result<CurrentFacts, BayRefitError> {
        authority::installation(config, state, world, intent, acceptance)
    }
    fn policy(config: &BayRefitConfig) -> &BayPaymentPolicy {
        &config.policy
    }
    fn cleanup_ready(config: &BayRefitConfig, world: &World) -> Result<(), BayRefitError> {
        config.cleanup_ready(world)
    }
    fn matches_policy(
        subject: &BaySubject,
        policy: &BayPaymentPolicy,
    ) -> Result<(), BayRefitError> {
        subject.matches_policy(policy)
    }
    fn supplier_validate(
        supplier: &BayOfferAttribution,
        policy: &BayPaymentPolicy,
        subject: &BaySubject,
    ) -> Result<(), BayRefitError> {
        supplier.validate(policy, subject)
    }
    fn action_ref(policy: &BayPaymentPolicy, action: &str) -> Result<DefinitionRef, BayRefitError> {
        policy.action_ref(action)
    }
    fn bytes<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, BayRefitError> {
        super::subject::bytes(value, max)
    }
    fn installation_acceptance(request: &BayInstallationRequest) -> CommitToken {
        request.acceptance
    }
    fn cancellation_acceptance(request: &BayCancellationRequest) -> CommitToken {
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
                refit: part,
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
                refit,
            } => Phase::Install {
                format,
                acceptance,
                commit,
                action,
                at,
                part: refit,
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
    fn installed(part: DefinitionDigest) -> BayTerminalResult {
        BayTerminalResult::Installed { refit: part }
    }
    fn cancelled() -> BayTerminalResult {
        BayTerminalResult::Cancelled {}
    }
    fn result_part(result: &BayTerminalResult) -> Option<DefinitionDigest> {
        result.refit()
    }
    fn part(order: &BayOrderAttribution) -> DefinitionDigest {
        order.refit
    }

    /// The quoted fee.
    fn price(intent: &FundingIntent) -> Result<Amount, BayRefitError> {
        Ok(intent.subject.price())
    }
    /// The account the subject's bay pays.
    fn provider_account(intent: &FundingIntent) -> Result<AccountId, BayRefitError> {
        Ok(intent
            .policy
            .site(intent.subject.facility)?
            .provider
            .account)
    }
    fn commit_reused(_: &BaySubject, _: CommitToken) -> bool {
        false
    }
    /// One open order a hull.
    fn exclusive(history: &refit_twin::receipt::PaymentHistory<Self>) -> Option<ObjId> {
        history
            .terminal
            .is_none()
            .then_some(history.intent.subject.hull)
    }
    fn conflicts(_: &refit_twin::receipt::PaymentHistory<Self>, _: &FundingIntent) -> bool {
        false
    }
}

/// The correlation a bay fee plan carries.
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
        refit: DefinitionDigest,
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
pub enum BayTerminalResult {
    Installed { refit: DefinitionDigest },
    Cancelled {},
}
impl BayTerminalResult {
    pub const fn refit(&self) -> Option<DefinitionDigest> {
        match self {
            Self::Installed { refit } => Some(*refit),
            Self::Cancelled {} => None,
        }
    }
}

impl refit_twin::Subject for BaySubject {
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
impl refit_twin::Policy for BayPaymentPolicy {
    fn issuer(&self) -> &SystemPrincipalId {
        &self.issuer
    }
    fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
}
impl refit_twin::Supplier for BayOfferAttribution {
    fn observed_utc(&self) -> i64 {
        self.observed_utc
    }
    fn expires_utc(&self) -> i64 {
        self.expires_utc
    }
}
