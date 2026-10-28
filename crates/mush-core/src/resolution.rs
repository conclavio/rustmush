//! Deterministic, domain-separated authoritative randomness.
//!
//! Providers receive a short-lived [`ResolutionRng`], never root key bytes.
//! Each named stream has an independent counter, so adding a cosmetic draw
//! cannot shift hit-location or damage results. The resulting transcript is
//! stored in a [`ResolutionRecord`]; historical replay applies the recorded
//! outcome rather than running formulas again.

use std::collections::BTreeMap;
use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sha2::Sha256;
use thiserror::Error;

use crate::action::{CanonicalHash, CanonicalHashError, CommitToken, ScopedIdempotencyKey};
use crate::mechanics::{CanonicalId, DefinitionRef, RulesValue};

const RESOLUTION_KDF_DOMAIN: &[u8] = b"rustmush.resolution.kdf.v1\0";
const RESOLUTION_DRAW_DOMAIN: &[u8] = b"rustmush.resolution.draw.v1\0";

/// Maximum root-key versions retained during rotation.
pub const MAX_RESOLUTION_SECRET_VERSIONS: usize = 4;
/// Maximum independent labels used by one resolution.
pub const MAX_RESOLUTION_STREAMS: usize = 64;
/// Maximum logical draws recorded by one resolution.
pub const MAX_RESOLUTION_DRAWS: usize = 1_024;
/// Maximum exact definitions named by one resolution record.
pub const MAX_RESOLUTION_DEFINITIONS: usize = 64;
/// Defensive bound on rejection samples for one unbiased bounded draw.
pub const MAX_RNG_REJECTION_ATTEMPTS: u32 = 128;

/// Exact deterministic algorithm/KDF contract used by a resolution record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RngAlgorithmVersion {
    HmacSha256CounterV1,
}

/// One root-secret version loaded from durable secret storage. Key bytes are
/// deliberately neither serializable nor printable.
#[derive(Clone)]
pub struct ResolutionSecret {
    version: u32,
    key: [u8; 32],
}

impl fmt::Debug for ResolutionSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionSecret")
            .field("version", &self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl ResolutionSecret {
    pub fn new(version: u32, key: [u8; 32]) -> Result<Self, ResolutionSecretError> {
        if version == 0 {
            return Err(ResolutionSecretError::ZeroVersion);
        }
        if key == [0; 32] {
            return Err(ResolutionSecretError::ZeroKey);
        }
        Ok(Self { version, key })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

/// Current plus retained KDF secrets. Rotation must retain any version named
/// by unresolved accepted work; callers select that exact version on resume.
#[derive(Clone)]
pub struct ResolutionKeyRing {
    current: ResolutionSecret,
    retained: BTreeMap<u32, ResolutionSecret>,
}

impl fmt::Debug for ResolutionKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionKeyRing")
            .field("current_version", &self.current.version)
            .field("retained_versions", &self.retained.keys())
            .finish()
    }
}

impl ResolutionKeyRing {
    pub fn new(
        current: ResolutionSecret,
        retained: impl IntoIterator<Item = ResolutionSecret>,
    ) -> Result<Self, ResolutionSecretError> {
        let mut retained_by_version = BTreeMap::new();
        for secret in retained {
            if secret.version >= current.version {
                return Err(ResolutionSecretError::NonMonotonicVersion);
            }
            if retained_by_version.insert(secret.version, secret).is_some() {
                return Err(ResolutionSecretError::DuplicateVersion);
            }
        }
        if retained_by_version.len().saturating_add(1) > MAX_RESOLUTION_SECRET_VERSIONS {
            return Err(ResolutionSecretError::VersionLimit {
                max: MAX_RESOLUTION_SECRET_VERSIONS,
            });
        }
        Ok(Self {
            current,
            retained: retained_by_version,
        })
    }

    #[must_use]
    pub const fn current_version(&self) -> u32 {
        self.current.version
    }

    /// Start new work under the current durable root-key version.
    pub fn start(
        &self,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, ResolutionRngError> {
        ResolutionRng::from_secret(&self.current, idempotency, domain, inputs_hash)
    }

    /// Resume accepted work under the exact secret version it recorded.
    pub fn resume(
        &self,
        secret_version: u32,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, ResolutionRngError> {
        let secret = if self.current.version == secret_version {
            &self.current
        } else {
            self.retained
                .get(&secret_version)
                .ok_or(ResolutionRngError::UnknownSecretVersion {
                    version: secret_version,
                })?
        };
        ResolutionRng::from_secret(secret, idempotency, domain, inputs_hash)
    }
}

/// Invalid durable resolution key ring.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionSecretError {
    #[error("resolution secret version zero is reserved")]
    ZeroVersion,
    #[error("an all-zero resolution secret is not accepted")]
    ZeroKey,
    #[error("retained resolution-secret versions must precede the current version")]
    NonMonotonicVersion,
    #[error("resolution secret version appears more than once")]
    DuplicateVersion,
    #[error("resolution key ring exceeds its hard limit of {max} versions")]
    VersionLimit { max: usize },
}

/// Short-lived capability passed only while one authoritative provider is
/// preparing an action. It exposes no root key bytes and can mint at most one
/// resolution-scoped generator for the invocation's exact idempotency scope.
pub struct PrepareCtx<'a> {
    idempotency: &'a ScopedIdempotencyKey,
    commit_token: CommitToken,
    keys: Option<&'a ResolutionKeyRing>,
    resolution_opened: bool,
    participants: Option<crate::participant::ParticipantPreparation<'a>>,
    owner_error: Option<crate::participant::ParticipantError>,
    carry_hooks: Option<&'a [crate::module::HookReg]>,
    move_hooks: Option<&'a [crate::module::HookReg]>,
    owner_reads: Option<crate::read_service::PrepareReadContext<'a>>,
    accounts: Option<crate::account::observation::AccountPrepareContext<'a>>,
    registered_transaction: Option<(
        &'a crate::action_state::ActionReadBase,
        &'a crate::action::ProviderRegistrationOrdinal,
    )>,
}

impl fmt::Debug for PrepareCtx<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrepareCtx")
            .field("idempotency", self.idempotency)
            .field("commit_token", &self.commit_token)
            .field(
                "current_secret_version",
                &self.keys.map(ResolutionKeyRing::current_version),
            )
            .field("resolution_opened", &self.resolution_opened)
            .finish()
    }
}

impl<'a> PrepareCtx<'a> {
    pub(crate) const fn new(
        idempotency: &'a ScopedIdempotencyKey,
        commit_token: CommitToken,
        keys: Option<&'a ResolutionKeyRing>,
    ) -> Self {
        Self {
            idempotency,
            commit_token,
            keys,
            resolution_opened: false,
            participants: None,
            owner_error: None,
            carry_hooks: None,
            move_hooks: None,
            owner_reads: None,
            accounts: None,
            registered_transaction: None,
        }
    }

    pub(crate) fn with_registered_transaction(
        mut self,
        base: &'a crate::action_state::ActionReadBase,
        provider: &'a crate::action::ProviderRegistrationOrdinal,
    ) -> Self {
        self.registered_transaction = Some((base, provider));
        self
    }

    /// Reserve the closed Registry job's exact future completion keys. The job
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_registry_rating_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.registry_rating_completion(txn, None)
    }

    /// Retire this Registry job's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn finish_registry_rating_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        job: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.registry_rating_completion(txn, Some(job))
    }

    fn registry_rating_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.registry_rating_completion(base, provider, self.commit_token, terminal)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "Registry preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the closed original purchase's exact future completion keys. The job
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_original_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.original_mind_purchase_completion(txn, None)
    }

    /// Retire this original purchase's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn finish_original_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.original_mind_purchase_completion(txn, Some(operation))
    }

    fn original_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.original_mind_purchase_completion(base, provider, self.commit_token, terminal)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "purchase preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the fixed Roll claim's maximum Close image during actual IC Open.
    pub fn reserve_ic_mind_claim_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.ic_mind_claim_completion(txn, None)
    }

    /// Consume the acknowledged IC Open reservation using the current Close
    /// action. Actual physical event authority remains with the four owners.
    pub fn close_ic_mind_claim_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        open: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.ic_mind_claim_completion(txn, Some(open))
    }

    fn ic_mind_claim_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        open: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.ic_mind_claim_completion(base, provider, self.commit_token, open)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "IC claim preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the exact execution/result keys with the actual admitted job.
    pub fn reserve_conduct_trial_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
    ) -> Result<(), crate::ActionStateError> {
        self.conduct_trial_completion(txn, None)
    }
    /// Actual post-images determine whether progress retains or consumes capacity.
    pub fn advance_conduct_trial_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        job: crate::CommitToken,
    ) -> Result<(), crate::ActionStateError> {
        self.conduct_trial_completion(txn, Some(job))
    }
    fn conduct_trial_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        job: Option<crate::CommitToken>,
    ) -> Result<(), crate::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.conduct_trial_completion(base, provider, self.commit_token, job)
            }
            _ => Err(crate::ActionStateError::TerminalCapacity(
                "Conduct preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }
    /// Reserve six fixed completion keys before the native coverage Bind installs.
    pub fn reserve_hull_coverage_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
    ) -> Result<(), crate::ActionStateError> {
        self.hull_coverage_completion(txn, None)
    }
    /// Consume the original policy's reservation only with its paired terminal.
    pub fn finish_hull_coverage_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        binding: crate::CommitToken,
    ) -> Result<(), crate::ActionStateError> {
        self.hull_coverage_completion(txn, Some(binding))
    }
    fn hull_coverage_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        binding: Option<crate::CommitToken>,
    ) -> Result<(), crate::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.hull_coverage_completion(base, provider, self.commit_token, binding)
            }
            _ => Err(crate::ActionStateError::TerminalCapacity(
                "coverage preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    pub fn reserve_core_refit_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
    ) -> Result<(), crate::ActionStateError> {
        self.core_refit_completion(txn, order, true)
    }
    /// Retain through actual scheduled Advance; release only on Install/Cancel.
    pub fn advance_core_refit_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
    ) -> Result<(), crate::ActionStateError> {
        self.core_refit_completion(txn, order, false)
    }
    fn core_refit_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
        admission: bool,
    ) -> Result<(), crate::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.core_refit_completion(base, provider, self.commit_token, order, admission)
            }
            _ => Err(crate::ActionStateError::TerminalCapacity(
                "core refit preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the fixed actual Industry order's future terminal rows at Accept.
    pub fn reserve_core_integration_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
    ) -> Result<(), crate::ActionStateError> {
        self.core_integration_completion(txn, order, true)
    }
    /// Retain through Reserve/Advance; release only on actual Complete/Cancel.
    pub fn advance_core_integration_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
    ) -> Result<(), crate::ActionStateError> {
        self.core_integration_completion(txn, order, false)
    }
    fn core_integration_completion(
        &mut self,
        txn: &mut crate::ActionTxn<'_>,
        order: u64,
        admission: bool,
    ) -> Result<(), crate::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.core_integration_completion(base, provider, self.commit_token, order, admission)
            }
            _ => Err(crate::ActionStateError::TerminalCapacity(
                "core integration preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the exact seven completion targets for this renewal Draft.
    /// The action, operation, paired images and provider must be this invocation.
    pub fn reserve_mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.mind_renewal_completion(txn, None)
    }

    pub fn reserve_house_mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.house_mind_renewal_completion(txn, None)
    }

    /// Advance an acknowledged renewal using the actual registered action.
    /// Fund retains its reservation; Grant, Refund and Close consume it once.
    pub fn advance_mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.mind_renewal_completion(txn, Some(operation))
    }

    pub fn advance_house_mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.house_mind_renewal_completion(txn, Some(operation))
    }

    fn mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.mind_renewal_completion(base, provider, self.commit_token, operation)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "renewal preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    fn house_mind_renewal_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.house_mind_renewal_completion(base, provider, self.commit_token, operation)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "renewal preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the closed entitlement purchase's exact future completion keys. The job
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_mind_entitlement_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.mind_entitlement_purchase_completion(txn, None)
    }

    /// Retire this entitlement purchase's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn finish_mind_entitlement_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.mind_entitlement_purchase_completion(txn, Some(operation))
    }

    fn mind_entitlement_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => txn.mind_entitlement_purchase_completion(
                base,
                provider,
                self.commit_token,
                terminal,
            ),
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "entitlement purchase preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the closed pending charter transfer's exact future completion keys. The operation
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_charter_pending_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.charter_pending_completion(txn, None)
    }

    /// Retire this pending charter transfer's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn close_charter_pending_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.charter_pending_completion(txn, Some(operation))
    }

    fn charter_pending_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.charter_pending_completion(base, provider, self.commit_token, terminal)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "pending charter transfer preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the closed paid resale's exact future completion keys. The operation
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_paid_resale_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.paid_resale_completion(txn, None)
    }

    /// Retire this paid resale's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn close_paid_resale_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.paid_resale_completion(txn, Some(operation))
    }

    fn paid_resale_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.paid_resale_completion(base, provider, self.commit_token, terminal)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "paid resale preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Reserve the closed house purchase's exact future completion keys. The job
    /// and validated post-images must belong to this invocation and transaction.
    pub fn reserve_house_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.house_mind_purchase_completion(txn, None)
    }

    /// Retire this house purchase's reservation with its exact terminal images in
    /// the same acknowledgement. This does not accept caller-defined targets.
    pub fn finish_house_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), crate::action_state::ActionStateError> {
        self.house_mind_purchase_completion(txn, Some(operation))
    }

    fn house_mind_purchase_completion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        terminal: Option<CommitToken>,
    ) -> Result<(), crate::action_state::ActionStateError> {
        let result = match (self.owner_error.is_none(), self.registered_transaction) {
            (true, Some((base, provider))) => {
                txn.house_mind_purchase_completion(base, provider, self.commit_token, terminal)
            }
            _ => Err(crate::action_state::ActionStateError::TerminalCapacity(
                "house purchase preparation unavailable",
            )),
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    // Only the Engine-owned composed preparation supplies the actual frozen
    // hook registrations. Providers cannot substitute a hook list.
    pub(crate) fn with_carry_hooks(mut self, hooks: &'a [crate::module::HookReg]) -> Self {
        self.carry_hooks = Some(hooks);
        self
    }
    pub(crate) fn with_move_hooks(mut self, hooks: &'a [crate::module::HookReg]) -> Self {
        self.move_hooks = Some(hooks);
        self
    }

    pub(crate) fn with_participants(
        mut self,
        participants: crate::participant::ParticipantPreparation<'a>,
    ) -> Self {
        self.participants = Some(participants);
        self
    }

    pub(crate) fn with_owner_reads(
        mut self,
        reads: crate::read_service::PrepareReadContext<'a>,
    ) -> Self {
        self.owner_reads = Some(reads);
        self
    }

    pub(crate) fn with_account_observations(
        mut self,
        accounts: crate::account::observation::AccountPrepareContext<'a>,
    ) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Capture the actual invocation actor's personal account without exposing
    /// its identity. The engine retains backend and oracle-placement fences
    /// through custody and installation, sharing the pair-observation limit.
    /// Refusal poisons preparation even when the provider catches the error.
    pub fn personal_account(
        &mut self,
    ) -> Result<crate::account::PersonalAccountObservation, crate::participant::ParticipantError>
    {
        if let Some(error) = &self.owner_error {
            return Err(error.clone());
        }
        let preparation = self.commit_token();
        let result = match &mut self.accounts {
            Some(accounts) => accounts.personal_account(preparation),
            None => Err(crate::participant::ParticipantError::Unavailable),
        };
        if let Err(error) = &result {
            self.owner_error = Some(error.clone());
        }
        result
    }

    /// Observe the actual invocation actor and another live Player as belonging
    /// to distinct personal accounts. The engine retains map and oracle-placement
    /// fences through the single writer acknowledgement and installation.
    /// Refusal poisons preparation even when the provider catches the error.
    pub fn distinct_personal_accounts(
        &mut self,
        other: crate::ObjId,
    ) -> Result<crate::account::DistinctPersonalAccounts, crate::participant::ParticipantError>
    {
        if let Some(error) = &self.owner_error {
            return Err(error.clone());
        }
        let result = match &mut self.accounts {
            Some(accounts) => accounts.distinct(other),
            None => Err(crate::participant::ParticipantError::Unavailable),
        };
        if let Err(error) = &result {
            self.owner_error = Some(error.clone());
        }
        result
    }

    /// Read a registered owner's committed pre-image from this action's exact
    /// typed store and base revision. Staged puts/deletes are never visible.
    /// Request fields cannot change the actual provider, actor or principal.
    /// A refusal poisons the whole preparation even if provider code catches it.
    ///
    /// Service readiness and slot identity survive through installation. These
    /// checks do not track arbitrary World/document values: the closed effect
    /// must still fence current authority facts and validate its own post-images.
    pub fn read_owner_preimage<S: crate::read_service::OwnerReadService>(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        handle: &crate::read_service::OwnerReadHandle<S>,
        request: S::Request,
    ) -> Result<S::Output, crate::read_service::OwnerReadError> {
        let result = if self.owner_error.is_some() {
            Err(crate::read_service::OwnerReadError::Unavailable)
        } else {
            match &mut self.owner_reads {
                Some(reads) => reads.read(txn, handle, request),
                None => Err(crate::read_service::OwnerReadError::Unavailable),
            }
        };
        if result.is_err() {
            txn.poison_owner_read();
            self.owner_error
                .get_or_insert(crate::participant::ParticipantError::Authority);
        }
        result
    }

    /// Compare an opaque owner-preparation identity with this actual complete
    /// invocation/action/commit and the registered transaction pre-image.
    /// This grants no authority and does not replace the owner's live checks.
    /// A mismatch poisons both preparation and transaction even if caught.
    pub fn require_owner_preparation_binding(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        binding: &crate::participant::OwnerPreparationBinding,
    ) -> Result<(), crate::participant::ParticipantError> {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            txn.read_preimage(base, provider)
                .map_err(|_| crate::participant::ParticipantError::Authority)?;
            if !self
                .participants
                .as_ref()
                .is_some_and(|p| p.matches_preparation(binding, self.commit_token))
            {
                return Err(crate::participant::ParticipantError::Authority);
            }
            Ok(())
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Observe the actual actor's ordinary exit or HOME route using the installed
    /// locks and MoveAttempt hooks. The result is inert: it moves nothing and
    /// grants no publication capability or command-dispatch admission.
    ///
    /// The registered transaction supplies the committed typed-state lens; a
    /// caller cannot substitute a principal, actor, hook list or destination.
    /// Any refusal poisons preparation and transaction even if caught.
    pub fn admit_locomotion(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        request: crate::locomotion::LocomotionRequest,
    ) -> Result<crate::locomotion::LocomotionDecision, crate::participant::ParticipantError> {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            let lens = txn
                .read_preimage(base, provider)
                .map_err(|_| crate::participant::ParticipantError::Authority)?;
            let hooks = self
                .move_hooks
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            self.participants
                .as_ref()
                .ok_or(crate::participant::ParticipantError::Unavailable)?
                .admit_locomotion(self.commit_token, hooks, lens, request)
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Recheck one inert admission against this exact invocation/action/commit,
    /// its object preimages and the current registered MoveAttempt hooks.
    /// This does not retain an installation fence or publish a movement.
    /// Refusal poisons both preparation and transaction even if caught.
    pub fn require_locomotion_admission(
        &mut self,
        txn: &mut crate::action_state::ActionTxn<'_>,
        admission: &crate::locomotion::LocomotionAdmission,
    ) -> Result<(), crate::participant::ParticipantError> {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            let lens = txn
                .read_preimage(base, provider)
                .map_err(|_| crate::participant::ParticipantError::Authority)?;
            let hooks = self
                .move_hooks
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            self.participants
                .as_ref()
                .ok_or(crate::participant::ParticipantError::Unavailable)?
                .require_locomotion_admission(self.commit_token, hooks, lens, admission)
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Prepare one registered owner's closed effect in this exact action.
    /// A refused request poisons this preparation even if provider code catches it.
    pub fn prepare_owner<P: crate::participant::OwnerParticipant>(
        &mut self,
        handle: &crate::participant::ParticipantHandle<P>,
        request: P::Request,
    ) -> Result<P::Output, crate::participant::ParticipantError> {
        if let Some(error) = &self.owner_error {
            return Err(error.clone());
        }
        let result = match &mut self.participants {
            Some(participants) => participants.prepare(handle, self.commit_token, request),
            None => Err(crate::participant::ParticipantError::Unavailable),
        };
        if let Err(error) = &result {
            self.owner_error = Some(error.clone());
        }
        result
    }

    pub(crate) fn owner_error(&self) -> Option<&crate::participant::ParticipantError> {
        self.owner_error.as_ref()
    }

    /// Actual read-only world used by this authoritative preparation. Reads
    /// which must remain current still require a registered participant fence;
    /// this borrow by itself grants neither authority nor freshness tracking.
    pub fn world(&self) -> Result<&crate::World, crate::participant::ParticipantError> {
        self.participants
            .as_ref()
            .map(|p| p.world())
            .ok_or(crate::participant::ParticipantError::Unavailable)
    }

    /// Initialize the closed hardware reservation deployment alongside the
    /// actual Industry production marker. The owner policy proves empty history.
    pub fn initialize_core_outputs<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::participant::ParticipantError> {
        let facts = self
            .read_owner_preimage(
                txn,
                service.reader(),
                service.request(crate::core_output::OutputPhase::Initialize, None),
            )
            .map_err(|_| crate::participant::ParticipantError::Authority)?;
        self.with_shell(|p, c| p.initialize_core_outputs(service, txn, c, facts))
    }
    /// Resolve the actual accepted order through its registered owner. Core
    /// derives the fixed inert object and trusted-container inverse image.
    pub fn reserve_core_output<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        order: u64,
    ) -> Result<crate::core_output::ReservedCoreOutput, crate::participant::ParticipantError> {
        let facts = self
            .read_owner_preimage(
                txn,
                service.reader(),
                service.request(crate::core_output::OutputPhase::Reserve, Some(order)),
            )
            .map_err(|_| crate::participant::ParticipantError::Authority)?;
        self.with_shell(|p, c| p.reserve_core_output(service, txn, c, order, facts))
    }
    /// Verify and consume the acknowledged output for this exact actual order.
    /// The returned identity feeds the same-commit certificate/title producer;
    /// it grants neither by itself and never edits World at completion.
    pub fn consume_core_output<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        order: u64,
    ) -> Result<crate::core_output::ReservedCoreOutput, crate::participant::ParticipantError> {
        let facts = self
            .read_owner_preimage(
                txn,
                service.reader(),
                service.request(crate::core_output::OutputPhase::Complete, Some(order)),
            )
            .map_err(|_| crate::participant::ParticipantError::Authority)?;
        self.with_shell(|p, c| p.consume_core_output(service, txn, c, order, facts))
    }

    /// Deliver the actual completed output to this invocation's requester.
    /// Core evaluates the real carry hooks against committed pre-images, then
    /// retains the same hooks and exact physical/owner fences through custody.
    /// No provider-supplied World postimage or detached typed lens is accepted.
    pub fn deliver_core_output<
        P: crate::core_output::CoreOutputPolicy,
        D: crate::core_delivery::CoreDeliveryPolicy,
    >(
        &mut self,
        service: &crate::core_delivery::CoreOutputDeliveries<P, D>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        order: u64,
    ) -> Result<crate::core_delivery::DeliveredCoreOutput, crate::participant::ParticipantError>
    {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            let hooks = std::sync::Arc::from(
                self.carry_hooks
                    .ok_or(crate::participant::ParticipantError::Unavailable)?,
            );
            let facts = self
                .read_owner_preimage(
                    txn,
                    service.reader(),
                    crate::core_delivery::DeliveryRequest { order },
                )
                .map_err(|_| crate::participant::ParticipantError::Authority)?;
            self.with_shell(|p, c| {
                p.deliver_core_output(service, txn, c, order, facts, hooks, base, provider)
            })
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Exchange one incoming and optional displaced core only with the exact
    /// already-prepared Economy and Tactics owners. Core owns physical images.
    pub fn exchange_core_component<P: crate::core_exchange::CoreExchangePolicy>(
        &mut self,
        service: &crate::core_exchange::CoreComponentExchanges<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        order: u64,
    ) -> Result<crate::core_exchange::ExchangedCoreComponent, crate::participant::ParticipantError>
    {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            let hooks = std::sync::Arc::from(
                self.carry_hooks
                    .ok_or(crate::participant::ParticipantError::Unavailable)?,
            );
            let facts = self
                .read_owner_preimage(
                    txn,
                    service.reader(),
                    crate::core_exchange::ExchangeRequest { order },
                )
                .map_err(|_| crate::participant::ParticipantError::Authority)?;
            self.with_shell(|p, c| {
                p.exchange_core_component(service, txn, c, order, facts, hooks, base, provider)
            })
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Prepare the exact registered Sim participant and its Core-built object
    /// projections together. Any refusal poisons the complete preparation.
    pub fn prepare_physical_transition<
        P: crate::participant::OwnerParticipant<
                Output = crate::physical_transition::PhysicalTransitionDraft,
            >,
    >(
        &mut self,
        service: &crate::physical_transition::CorePhysicalTransitions<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        request: P::Request,
    ) -> Result<
        crate::physical_transition::PhysicalTransitionReceipt,
        crate::participant::ParticipantError,
    > {
        let result = (|| {
            if let Some(error) = &self.owner_error {
                return Err(error.clone());
            }
            let (base, provider) = self
                .registered_transaction
                .ok_or(crate::participant::ParticipantError::Unavailable)?;
            let hooks = std::sync::Arc::from(
                self.carry_hooks
                    .ok_or(crate::participant::ParticipantError::Unavailable)?,
            );
            let move_hooks = std::sync::Arc::from(
                self.move_hooks
                    .ok_or(crate::participant::ParticipantError::Unavailable)?,
            );
            self.with_shell(|p, c| {
                p.prepare_physical_transition(
                    service, txn, c, request, hooks, move_hooks, base, provider,
                )
            })
        })();
        if let Err(error) = &result {
            txn.poison_owner_read();
            self.owner_error.get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Reserve one inert charter object using this invocation's actual actor.
    /// The object and typed reservation share this action's single commit.
    pub fn reserve_initial_filing_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(crate::reserved_shell::ShellKey, crate::ObjId), crate::participant::ParticipantError>
    {
        self.with_shell(|participants, commit| participants.reserve_shell(service, txn, commit))
    }

    /// Initialize the closed shell family in explicit operator setup mode.
    /// The service verifies the actual GOD invocation and empty typed families.
    pub fn initialize_commerce_shells(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
    ) -> Result<(), crate::participant::ParticipantError> {
        self.with_shell(|participants, commit| participants.initialize_shells(service, txn, commit))
    }

    /// Read an acknowledged reservation through the exact initial-filing action.
    pub fn resolve_initial_filing_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        key: crate::reserved_shell::ShellKey,
    ) -> Result<crate::reserved_shell::ReservedShell, crate::participant::ParticipantError> {
        self.with_shell(|participants, _| participants.resolve_shell(service, txn, key))
    }

    /// Consume the exact acknowledged reservation, retaining its object fence
    /// until every owner is bound after the writer acknowledges the whole union.
    pub fn consume_initial_filing_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        shell: crate::reserved_shell::ReservedShell,
    ) -> Result<(), crate::participant::ParticipantError> {
        self.with_shell(|participants, commit| {
            participants.consume_shell(service, txn, shell, commit)
        })
    }

    fn with_shell<T>(
        &mut self,
        prepare: impl FnOnce(
            &mut crate::participant::ParticipantPreparation<'_>,
            CommitToken,
        ) -> Result<T, crate::participant::ParticipantError>,
    ) -> Result<T, crate::participant::ParticipantError> {
        if let Some(error) = &self.owner_error {
            return Err(error.clone());
        }
        let result = match &mut self.participants {
            Some(participants) => prepare(participants, self.commit_token),
            None => Err(crate::participant::ParticipantError::Unavailable),
        };
        if let Err(error) = &result {
            self.owner_error = Some(error.clone());
        }
        result
    }

    /// Coordinator-derived immutable identity of the bundle being prepared.
    /// Providers may reference it in exact post-images but cannot replace it.
    #[must_use]
    pub const fn commit_token(&self) -> CommitToken {
        self.commit_token
    }

    /// Current durable root-secret version. Long-lived work records this
    /// value before it can outlive a rotation boundary.
    pub fn current_resolution_secret_version(&self) -> Result<u32, PrepareContextError> {
        self.keys
            .map(ResolutionKeyRing::current_version)
            .ok_or(PrepareContextError::KeyRingUnavailable)
    }

    /// Open the only RNG stream set available to this action preparation.
    /// The provider supplies an action-specific domain and a canonical hash of
    /// every truth input used by its formula; the kernel supplies identity and
    /// secret custody.
    pub fn start_resolution(
        &mut self,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, PrepareContextError> {
        if self.resolution_opened {
            return Err(PrepareContextError::AlreadyOpened);
        }
        let keys = self.keys.ok_or(PrepareContextError::KeyRingUnavailable)?;
        let rng = keys.start(self.idempotency, domain, inputs_hash)?;
        self.resolution_opened = true;
        Ok(rng)
    }

    /// Resume/start a resolution under an exact version pinned by durable
    /// accepted work such as an action window.
    pub fn start_resolution_at(
        &mut self,
        secret_version: u32,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<ResolutionRng, PrepareContextError> {
        if self.resolution_opened {
            return Err(PrepareContextError::AlreadyOpened);
        }
        let keys = self.keys.ok_or(PrepareContextError::KeyRingUnavailable)?;
        let rng = keys.resume(secret_version, self.idempotency, domain, inputs_hash)?;
        self.resolution_opened = true;
        Ok(rng)
    }

    #[must_use]
    pub(crate) const fn resolution_opened(&self) -> bool {
        self.resolution_opened
    }
}

/// Failure to obtain a centrally scoped authoritative RNG.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PrepareContextError {
    #[error("durable resolution keys are not installed")]
    KeyRingUnavailable,
    #[error("an authoritative action may open at most one resolution RNG")]
    AlreadyOpened,
    #[error("cannot start the authoritative resolution RNG: {0}")]
    Rng(#[from] ResolutionRngError),
}

/// Shape and result of one logical deterministic draw.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedDrawValue {
    RawU32 { value: u32 },
    RangeU64 { upper_exclusive: u64, value: u64 },
}

/// One labeled draw retained in chronological call order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedDraw {
    label: CanonicalId,
    ordinal: u32,
    draw: RecordedDrawValue,
}

impl RecordedDraw {
    #[must_use]
    pub const fn label(&self) -> &CanonicalId {
        &self.label
    }

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn draw(&self) -> RecordedDrawValue {
        self.draw
    }
}

/// Non-cloneable resolution-scoped deterministic generator.
pub struct ResolutionRng {
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    seed: [u8; 32],
    inputs_hash: CanonicalHash,
    next_by_label: BTreeMap<CanonicalId, u32>,
    draws: Vec<RecordedDraw>,
}

impl fmt::Debug for ResolutionRng {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolutionRng")
            .field("algorithm", &self.algorithm)
            .field("secret_version", &self.secret_version)
            .field("seed", &"[REDACTED]")
            .field("inputs_hash", &self.inputs_hash)
            .field("streams", &self.next_by_label.len())
            .field("draws", &self.draws.len())
            .finish()
    }
}

impl ResolutionRng {
    fn from_secret(
        secret: &ResolutionSecret,
        idempotency: &ScopedIdempotencyKey,
        domain: &CanonicalId,
        inputs_hash: CanonicalHash,
    ) -> Result<Self, ResolutionRngError> {
        let context = CanonicalHash::for_canonical_value(
            "resolution_seed_context.v1",
            1,
            &json!({
                "idempotency": idempotency,
                "domain": domain,
                "inputs_hash": inputs_hash,
            }),
            64 * 1024,
        )?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret.key)
            .expect("HMAC-SHA256 accepts a fixed 32-byte key");
        mac.update(RESOLUTION_KDF_DOMAIN);
        mac.update(&secret.version.to_be_bytes());
        mac.update(context.as_bytes());
        let seed: [u8; 32] = mac.finalize().into_bytes().into();
        Ok(Self {
            algorithm: RngAlgorithmVersion::HmacSha256CounterV1,
            secret_version: secret.version,
            seed,
            inputs_hash,
            next_by_label: BTreeMap::new(),
            draws: Vec::new(),
        })
    }

    #[must_use]
    pub const fn algorithm(&self) -> RngAlgorithmVersion {
        self.algorithm
    }

    #[must_use]
    pub const fn secret_version(&self) -> u32 {
        self.secret_version
    }

    #[must_use]
    pub const fn inputs_hash(&self) -> CanonicalHash {
        self.inputs_hash
    }

    #[must_use]
    pub fn draws(&self) -> &[RecordedDraw] {
        &self.draws
    }

    /// Draw one raw word from a named independent stream.
    pub fn draw_u32(&mut self, label: &CanonicalId) -> Result<u32, ResolutionRngError> {
        let ordinal = self.reserve_draw(label)?;
        let value = u32::from_be_bytes(
            self.block(label, ordinal, 0)[..4]
                .try_into()
                .expect("four-byte slice"),
        );
        self.draws.push(RecordedDraw {
            label: label.clone(),
            ordinal,
            draw: RecordedDrawValue::RawU32 { value },
        });
        Ok(value)
    }

    /// Draw uniformly from `0..upper_exclusive` using bounded rejection
    /// sampling. One successful result consumes one logical draw ordinal.
    pub fn draw_bounded_u64(
        &mut self,
        label: &CanonicalId,
        upper_exclusive: u64,
    ) -> Result<u64, ResolutionRngError> {
        if upper_exclusive == 0 {
            return Err(ResolutionRngError::EmptyRange);
        }
        let ordinal = self.reserve_draw(label)?;
        let threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        let mut selected = None;
        for attempt in 0..MAX_RNG_REJECTION_ATTEMPTS {
            let raw = u64::from_be_bytes(
                self.block(label, ordinal, attempt)[..8]
                    .try_into()
                    .expect("eight-byte slice"),
            );
            if raw >= threshold {
                selected = Some(raw % upper_exclusive);
                break;
            }
        }
        let value = selected.ok_or(ResolutionRngError::RejectionLimit {
            max: MAX_RNG_REJECTION_ATTEMPTS,
        })?;
        self.draws.push(RecordedDraw {
            label: label.clone(),
            ordinal,
            draw: RecordedDrawValue::RangeU64 {
                upper_exclusive,
                value,
            },
        });
        Ok(value)
    }

    fn reserve_draw(&mut self, label: &CanonicalId) -> Result<u32, ResolutionRngError> {
        if self.draws.len() >= MAX_RESOLUTION_DRAWS {
            return Err(ResolutionRngError::DrawLimit {
                max: MAX_RESOLUTION_DRAWS,
            });
        }
        if !self.next_by_label.contains_key(label)
            && self.next_by_label.len() >= MAX_RESOLUTION_STREAMS
        {
            return Err(ResolutionRngError::StreamLimit {
                max: MAX_RESOLUTION_STREAMS,
            });
        }
        let ordinal = *self.next_by_label.get(label).unwrap_or(&0);
        let next = ordinal
            .checked_add(1)
            .ok_or(ResolutionRngError::OrdinalExhausted)?;
        self.next_by_label.insert(label.clone(), next);
        Ok(ordinal)
    }

    fn block(&self, label: &CanonicalId, ordinal: u32, attempt: u32) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.seed)
            .expect("HMAC-SHA256 accepts a fixed 32-byte key");
        mac.update(RESOLUTION_DRAW_DOMAIN);
        mac.update(&(label.as_str().len() as u64).to_be_bytes());
        mac.update(label.as_str().as_bytes());
        mac.update(&ordinal.to_be_bytes());
        mac.update(&attempt.to_be_bytes());
        mac.finalize().into_bytes().into()
    }

    /// Consume the generator into provider output that has no commit token.
    /// The central coordinator binds the token while building the bundle.
    pub fn into_prepared(
        self,
        mut definitions: Vec<DefinitionRef>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<PreparedResolution, ResolutionRecordError> {
        definitions.sort();
        PreparedResolution::new(
            self.algorithm,
            self.secret_version,
            definitions,
            self.inputs_hash,
            self.draws,
            modifiers,
            outcome,
        )
    }
}

/// Provider-produced resolution material before the coordinator assigns its
/// authoritative commit identity. This type is intentionally not serializable.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedResolution {
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

impl PreparedResolution {
    #[allow(clippy::too_many_arguments)]
    fn new(
        algorithm: RngAlgorithmVersion,
        secret_version: u32,
        definition_refs: Vec<DefinitionRef>,
        inputs_hash: CanonicalHash,
        draws: Vec<RecordedDraw>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<Self, ResolutionRecordError> {
        validate_resolution_parts(secret_version, &definition_refs, &draws)?;
        Ok(Self {
            algorithm,
            secret_version,
            definition_refs,
            inputs_hash,
            draws,
            modifiers,
            outcome,
        })
    }

    #[must_use]
    pub fn definition_refs(&self) -> &[DefinitionRef] {
        &self.definition_refs
    }

    #[must_use]
    pub const fn modifiers(&self) -> &RulesValue {
        &self.modifiers
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    /// Attach the coordinator-owned commit identity.
    #[must_use]
    pub fn bind(self, action: CommitToken) -> ResolutionRecord {
        ResolutionRecord {
            schema_version: ResolutionRecord::SCHEMA_VERSION,
            action,
            algorithm: self.algorithm,
            secret_version: self.secret_version,
            definition_refs: self.definition_refs,
            inputs_hash: self.inputs_hash,
            draws: self.draws,
            modifiers: self.modifiers,
            outcome: self.outcome,
        }
    }
}

/// Immutable authoritative resolution evidence and chosen outcome.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRecord {
    schema_version: u16,
    action: CommitToken,
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionRecordWire {
    schema_version: u16,
    action: CommitToken,
    algorithm: RngAlgorithmVersion,
    secret_version: u32,
    definition_refs: Vec<DefinitionRef>,
    inputs_hash: CanonicalHash,
    draws: Vec<RecordedDraw>,
    modifiers: RulesValue,
    outcome: RulesValue,
}

impl<'de> Deserialize<'de> for ResolutionRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ResolutionRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.schema_version,
            wire.action,
            wire.algorithm,
            wire.secret_version,
            wire.definition_refs,
            wire.inputs_hash,
            wire.draws,
            wire.modifiers,
            wire.outcome,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ResolutionRecord {
    pub const SCHEMA_VERSION: u16 = 1;

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        schema_version: u16,
        action: CommitToken,
        algorithm: RngAlgorithmVersion,
        secret_version: u32,
        definition_refs: Vec<DefinitionRef>,
        inputs_hash: CanonicalHash,
        draws: Vec<RecordedDraw>,
        modifiers: RulesValue,
        outcome: RulesValue,
    ) -> Result<Self, ResolutionRecordError> {
        if schema_version != Self::SCHEMA_VERSION {
            return Err(ResolutionRecordError::UnsupportedSchema {
                actual: schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        validate_resolution_parts(secret_version, &definition_refs, &draws)?;
        Ok(Self {
            schema_version,
            action,
            algorithm,
            secret_version,
            definition_refs,
            inputs_hash,
            draws,
            modifiers,
            outcome,
        })
    }

    #[must_use]
    pub const fn action(&self) -> CommitToken {
        self.action
    }

    #[must_use]
    pub const fn algorithm(&self) -> RngAlgorithmVersion {
        self.algorithm
    }

    #[must_use]
    pub const fn secret_version(&self) -> u32 {
        self.secret_version
    }

    #[must_use]
    pub fn definition_refs(&self) -> &[DefinitionRef] {
        &self.definition_refs
    }

    #[must_use]
    pub const fn inputs_hash(&self) -> CanonicalHash {
        self.inputs_hash
    }

    #[must_use]
    pub fn draws(&self) -> &[RecordedDraw] {
        &self.draws
    }

    #[must_use]
    pub const fn modifiers(&self) -> &RulesValue {
        &self.modifiers
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }
}

fn validate_resolution_parts(
    secret_version: u32,
    definition_refs: &[DefinitionRef],
    draws: &[RecordedDraw],
) -> Result<(), ResolutionRecordError> {
    if secret_version == 0 {
        return Err(ResolutionRecordError::ZeroSecretVersion);
    }
    if definition_refs.len() > MAX_RESOLUTION_DEFINITIONS {
        return Err(ResolutionRecordError::DefinitionLimit {
            max: MAX_RESOLUTION_DEFINITIONS,
        });
    }
    if definition_refs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ResolutionRecordError::NonCanonicalDefinitions);
    }
    validate_draws(draws)
}

fn validate_draws(draws: &[RecordedDraw]) -> Result<(), ResolutionRecordError> {
    if draws.len() > MAX_RESOLUTION_DRAWS {
        return Err(ResolutionRecordError::DrawLimit {
            max: MAX_RESOLUTION_DRAWS,
        });
    }
    let mut ordinals = BTreeMap::<&CanonicalId, u32>::new();
    for draw in draws {
        let expected = *ordinals.get(&draw.label).unwrap_or(&0);
        if draw.ordinal != expected {
            return Err(ResolutionRecordError::NonCanonicalOrdinal {
                label: draw.label.clone(),
                actual: draw.ordinal,
                expected,
            });
        }
        match draw.draw {
            RecordedDrawValue::RawU32 { .. } => {}
            RecordedDrawValue::RangeU64 {
                upper_exclusive,
                value,
            } => {
                if upper_exclusive == 0 || value >= upper_exclusive {
                    return Err(ResolutionRecordError::InvalidRangeDraw);
                }
            }
        }
        ordinals.insert(&draw.label, expected + 1);
    }
    if ordinals.len() > MAX_RESOLUTION_STREAMS {
        return Err(ResolutionRecordError::StreamLimit {
            max: MAX_RESOLUTION_STREAMS,
        });
    }
    Ok(())
}

/// Failure to construct or draw from an authoritative generator.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionRngError {
    #[error("resolution secret version {version} is unavailable")]
    UnknownSecretVersion { version: u32 },
    #[error("resolution uses at most {max} independent streams")]
    StreamLimit { max: usize },
    #[error("resolution uses at most {max} logical draws")]
    DrawLimit { max: usize },
    #[error("bounded draw range must not be empty")]
    EmptyRange,
    #[error("resolution draw ordinal is exhausted")]
    OrdinalExhausted,
    #[error("bounded draw exceeded {max} deterministic rejection attempts")]
    RejectionLimit { max: u32 },
    #[error("cannot hash resolution seed context: {0}")]
    Hash(#[from] CanonicalHashError),
}

/// Invalid immutable resolution evidence.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResolutionRecordError {
    #[error("unsupported resolution record schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("resolution secret version zero is reserved")]
    ZeroSecretVersion,
    #[error("resolution names more than {max} exact definitions")]
    DefinitionLimit { max: usize },
    #[error("resolution definitions must be strictly sorted and duplicate-free")]
    NonCanonicalDefinitions,
    #[error("resolution records more than {max} logical draws")]
    DrawLimit { max: usize },
    #[error("resolution records more than {max} independent streams")]
    StreamLimit { max: usize },
    #[error("resolution draw {label} ordinal {actual} should be {expected}")]
    NonCanonicalOrdinal {
        label: CanonicalId,
        actual: u32,
        expected: u32,
    },
    #[error("resolution bounded draw has an empty range or out-of-range value")]
    InvalidRangeDraw,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{
        IdempotencyKey, IdempotencyScope, PrincipalId, RequestUid, ScopedIdempotencyKey,
    };
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };

    fn key(request_byte: u8) -> ScopedIdempotencyKey {
        let request = RequestUid::from_bytes([request_byte; 16]);
        let principal = PrincipalId::Object { id: crate::GOD };
        ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: crate::OwnerId::new("toy").unwrap(),
                namespace: CanonicalId::new("attack").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        )
    }

    fn ring() -> ResolutionKeyRing {
        ResolutionKeyRing::new(
            ResolutionSecret::new(2, [0x22; 32]).unwrap(),
            [ResolutionSecret::new(1, [0x11; 32]).unwrap()],
        )
        .unwrap()
    }

    fn inputs() -> CanonicalHash {
        CanonicalHash::for_canonical_value("test_inputs.v1", 1, &json!({ "pool": 7 }), 1024)
            .unwrap()
    }

    fn definition(id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                crate::OwnerId::new("toy").unwrap(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new(id).unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn same_context_repeats_and_labels_do_not_shift_each_other() {
        let ring = ring();
        let domain = CanonicalId::new("tactical_attack").unwrap();
        let hit = CanonicalId::new("hit_location").unwrap();
        let flavor = CanonicalId::new("flavor").unwrap();

        let mut first = ring.start(&key(1), &domain, inputs()).unwrap();
        let expected_hit_1 = first.draw_bounded_u64(&hit, 12).unwrap();
        let expected_hit_2 = first.draw_bounded_u64(&hit, 12).unwrap();

        let mut repeated = ring.start(&key(1), &domain, inputs()).unwrap();
        assert_eq!(repeated.draw_bounded_u64(&hit, 12).unwrap(), expected_hit_1);
        repeated.draw_u32(&flavor).unwrap();
        assert_eq!(repeated.draw_bounded_u64(&hit, 12).unwrap(), expected_hit_2);
        assert_ne!(
            ring.start(&key(2), &domain, inputs())
                .unwrap()
                .draw_bounded_u64(&hit, 12)
                .unwrap(),
            expected_hit_1,
            "fixture keys should demonstrate idempotency-domain separation"
        );
    }

    #[test]
    fn retained_secret_replays_exact_version_and_debug_redacts_keys() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let label = CanonicalId::new("damage").unwrap();
        let old = ring
            .resume(1, &key(1), &domain, inputs())
            .unwrap()
            .draw_u32(&label)
            .unwrap();
        let repeated = ring
            .resume(1, &key(1), &domain, inputs())
            .unwrap()
            .draw_u32(&label)
            .unwrap();
        assert_eq!(old, repeated);
        assert!(matches!(
            ring.resume(99, &key(1), &domain, inputs()),
            Err(ResolutionRngError::UnknownSecretVersion { version: 99 })
        ));
        let debug = format!("{ring:?}");
        assert!(!debug.contains("34, 34"));
        assert!(!debug.contains("17, 17"));
    }

    #[test]
    fn draw_and_stream_caps_fail_closed() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        let label = CanonicalId::new("roll").unwrap();
        for _ in 0..MAX_RESOLUTION_DRAWS {
            rng.draw_u32(&label).unwrap();
        }
        assert_eq!(
            rng.draw_u32(&label).unwrap_err(),
            ResolutionRngError::DrawLimit {
                max: MAX_RESOLUTION_DRAWS
            }
        );

        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        for index in 0..MAX_RESOLUTION_STREAMS {
            rng.draw_u32(&CanonicalId::new(format!("stream_{index}")).unwrap())
                .unwrap();
        }
        assert_eq!(
            rng.draw_u32(&CanonicalId::new("one_more").unwrap())
                .unwrap_err(),
            ResolutionRngError::StreamLimit {
                max: MAX_RESOLUTION_STREAMS
            }
        );
    }

    #[test]
    fn resolution_record_roundtrips_and_rejects_tampering() {
        let ring = ring();
        let domain = CanonicalId::new("attack").unwrap();
        let mut rng = ring.start(&key(1), &domain, inputs()).unwrap();
        rng.draw_bounded_u64(&CanonicalId::new("damage").unwrap(), 6)
            .unwrap();
        let schema = definition("result");
        let record = rng
            .into_prepared(
                vec![definition("attack"), schema.clone()],
                RulesValue::new(schema.clone(), json!({ "pool": 7 })).unwrap(),
                RulesValue::new(schema, json!({ "successes": 3 })).unwrap(),
            )
            .unwrap()
            .bind(CommitToken::from_bytes([0x77; 16]));
        let restored: ResolutionRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(restored, record);

        let mut value = serde_json::to_value(&record).unwrap();
        value["draws"][0]["ordinal"] = json!(7);
        assert!(serde_json::from_value::<ResolutionRecord>(value).is_err());
        let mut value = serde_json::to_value(&record).unwrap();
        let upper = value["draws"][0]["draw"]["upper_exclusive"].clone();
        value["draws"][0]["draw"]["value"] = upper;
        assert!(serde_json::from_value::<ResolutionRecord>(value).is_err());
    }

    #[test]
    fn prepare_context_is_key_opaque_and_single_use() {
        let idempotency = key(1);
        let domain = CanonicalId::new("attack").unwrap();
        let token = CommitToken::from_bytes([0x44; 16]);
        let mut missing = PrepareCtx::new(&idempotency, token, None);
        assert_eq!(
            missing.current_resolution_secret_version().unwrap_err(),
            PrepareContextError::KeyRingUnavailable
        );
        assert_eq!(
            missing.start_resolution(&domain, inputs()).unwrap_err(),
            PrepareContextError::KeyRingUnavailable
        );

        let ring = ring();
        let mut context = PrepareCtx::new(&idempotency, token, Some(&ring));
        assert_eq!(context.commit_token(), token);
        assert_eq!(
            context.current_resolution_secret_version().unwrap(),
            ring.current_version()
        );
        assert_eq!(
            context
                .start_resolution(&domain, inputs())
                .unwrap()
                .secret_version(),
            ring.current_version()
        );
        assert_eq!(
            context.start_resolution(&domain, inputs()).unwrap_err(),
            PrepareContextError::AlreadyOpened
        );
        let mut retained = PrepareCtx::new(&idempotency, token, Some(&ring));
        assert_eq!(
            retained
                .start_resolution_at(1, &domain, inputs())
                .unwrap()
                .secret_version(),
            1
        );
        assert!(!format!("{context:?}").contains("34, 34"));
    }
}
