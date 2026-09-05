//! Durable, bounded delayed-decision records.
//!
//! Public prompts and responses contain projection DTOs only. Exact truth
//! subjects and revision witnesses remain in the separate private pending
//! record and are never accepted from a decision provider.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use thiserror::Error;

use crate::action::{
    ActionOfferId, CanonicalHash, IdempotencyKeyText, MAX_ACTION_REVISIONS, MAX_ACTION_TARGETS,
    PerceivedRef, PrincipalId, RequestUid, RevisionStamp, ScopedIdempotencyKey, SubjectKey,
};
use crate::clock::ClockStamp;
use crate::durable_schedule::{
    DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId, ScheduleRestartPolicy,
};
use crate::mechanics::{CanonicalId, DefinitionRef, OwnerId, canonical_json_bytes_with_limit};
use crate::obj::ObjId;
use crate::offer::{
    MAX_PROJECTED_ACTION_CHOICES, ProjectedLegalAction, ProjectedTargetSchema, ProjectionScopeId,
};
use crate::{
    ActionTxn, DurableOutboxIntent, StateFamilyDescriptor, StateFamilyDescriptorError,
    StateFamilyHandle, StateFamilyId, StateKeyCodec, StateValueDecoder, StateValueSerializer,
    StateValueValidator,
};

const DECISION_TOKEN_HMAC_DOMAIN: &[u8] = b"rustmush.decision-token.v1\0";
const DECISION_PROMPT_HASH_DOMAIN: &str = "decision_prompt_body.v1";

pub const MAX_DECISION_OFFERS: usize = 32;
pub const MAX_NONTERMINAL_DECISIONS: usize = 512;
pub const MAX_NONTERMINAL_DECISIONS_PER_CONTROLLER: usize = 8;
pub const MAX_NONTERMINAL_DECISIONS_PER_ACTOR: usize = 2;
pub const MAX_DECISION_TARGETS: usize = MAX_ACTION_TARGETS;
pub const MAX_DECISION_TARGET_WITNESSES: usize = MAX_PROJECTED_ACTION_CHOICES;
pub const MAX_DECISION_PROMPT_BYTES: usize = 192 * 1024;
pub const MAX_DECISION_PENDING_BYTES: usize = 256 * 1024;
pub const MAX_DECISION_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_DECISION_WAKE_BYTES: usize = 1024;
pub const MAX_DECISION_CONTEXT_BYTES: usize = 64 * 1024;
pub const MAX_DECISION_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DECISION_MAILBOX_IDS: usize = 128;
pub const MAX_DECISION_PROVIDER_WORKERS: usize = 32;
pub const MAX_DECISION_ATTEMPTS: u8 = 8;
pub const MAX_DECISION_RECOVERY_PAGE: usize = 128;
pub const MAX_DECISION_SECRET_VERSIONS: usize = 8;
pub const MAX_DECISION_BRIDGE_DEFINITIONS: usize = MAX_ACTION_REVISIONS;

/// Stable identity for one logical delayed decision across retries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DecisionId(pub CanonicalHash);

impl DecisionId {
    pub fn derive(
        controller: &DecisionControllerId,
        cause: &DecisionCause,
        action_idempotency: &ScopedIdempotencyKey,
    ) -> Result<Self, DecisionError> {
        CanonicalHash::for_canonical_value(
            "decision_id.v1",
            1,
            &json!({
                "controller": controller,
                "cause": cause,
                "action_idempotency": action_idempotency,
            }),
            32 * 1024,
        )
        .map(Self)
        .map_err(|error| DecisionError::Canonical(error.to_string()))
    }

    #[must_use]
    pub const fn canonical_hash(self) -> CanonicalHash {
        self.0
    }
}

/// One stable controller instance inside a composed owner.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionControllerId {
    pub owner: OwnerId,
    pub key: CanonicalId,
}

/// Deterministic cause and the only opt-in coalescing identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCause {
    pub kind: CanonicalId,
    pub instance: IdempotencyKeyText,
    /// `None` means this decision may never displace another pending record.
    pub replaceable_key: Option<IdempotencyKeyText>,
}

/// Exact E-owned fallback definition selected when async choice cannot finish.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FallbackPolicyRef(pub DefinitionRef);

/// Public-safe, ruleset-shaped projected decision context.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "ProjectedDecisionContextWire")]
pub struct ProjectedDecisionContext {
    schema: CanonicalId,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectedDecisionContextWire {
    schema: CanonicalId,
    value: Value,
}

impl ProjectedDecisionContext {
    pub fn new(schema: CanonicalId, value: Value) -> Result<Self, DecisionError> {
        let context = Self { schema, value };
        context.validate()?;
        Ok(context)
    }

    fn validate(&self) -> Result<(), DecisionError> {
        canonical_json_bytes_with_limit(&self.value, MAX_DECISION_CONTEXT_BYTES)
            .map_err(|_| DecisionError::ContextBytes)?;
        Ok(())
    }

    #[must_use]
    pub const fn schema(&self) -> &CanonicalId {
        &self.schema
    }

    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }
}

impl TryFrom<ProjectedDecisionContextWire> for ProjectedDecisionContext {
    type Error = DecisionError;

    fn try_from(value: ProjectedDecisionContextWire) -> Result<Self, Self::Error> {
        Self::new(value.schema, value.value)
    }
}

/// Full authentication tag over the exact persisted prompt identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionToken {
    pub id: DecisionId,
    pub revision: u64,
    pub prompt_digest: CanonicalHash,
    pub key_version: u32,
    pub tag: CanonicalHash,
}

/// One durable decision-token secret. Key material is never formatted.
#[derive(Clone)]
pub struct DecisionSecret {
    version: u32,
    key: [u8; 32],
}

impl fmt::Debug for DecisionSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecisionSecret")
            .field("version", &self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl DecisionSecret {
    pub fn new(version: u32, key: [u8; 32]) -> Result<Self, DecisionSecretError> {
        if version == 0 {
            return Err(DecisionSecretError::ZeroVersion);
        }
        if key == [0; 32] {
            return Err(DecisionSecretError::ZeroKey);
        }
        Ok(Self { version, key })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug)]
pub struct DecisionKeyRing {
    current: DecisionSecret,
    retained: BTreeMap<u32, DecisionSecret>,
}

impl DecisionKeyRing {
    pub fn new(
        current: DecisionSecret,
        retained: impl IntoIterator<Item = DecisionSecret>,
    ) -> Result<Self, DecisionSecretError> {
        let mut retained_by_version = BTreeMap::new();
        for secret in retained {
            if secret.version >= current.version {
                return Err(DecisionSecretError::NonMonotonicVersion);
            }
            if retained_by_version.insert(secret.version, secret).is_some() {
                return Err(DecisionSecretError::DuplicateVersion);
            }
        }
        if retained_by_version.len() + 1 > MAX_DECISION_SECRET_VERSIONS {
            return Err(DecisionSecretError::VersionLimit {
                max: MAX_DECISION_SECRET_VERSIONS,
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

    #[must_use]
    pub fn sign(
        &self,
        id: DecisionId,
        revision: u64,
        prompt_digest: CanonicalHash,
    ) -> DecisionToken {
        let tag = decision_mac(&self.current, id, revision, prompt_digest)
            .finalize()
            .into_bytes();
        DecisionToken {
            id,
            revision,
            prompt_digest,
            key_version: self.current.version,
            tag: CanonicalHash::from_bytes(tag.into()),
        }
    }

    #[must_use]
    pub fn verify(&self, token: &DecisionToken) -> bool {
        let secret = if token.key_version == self.current.version {
            Some(&self.current)
        } else {
            self.retained.get(&token.key_version)
        };
        secret.is_some_and(|secret| {
            decision_mac(secret, token.id, token.revision, token.prompt_digest)
                .verify_slice(token.tag.as_bytes())
                .is_ok()
        })
    }
}

fn decision_mac(
    secret: &DecisionSecret,
    id: DecisionId,
    revision: u64,
    prompt_digest: CanonicalHash,
) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret.key)
        .expect("HMAC-SHA256 accepts a fixed 32-byte key");
    mac.update(DECISION_TOKEN_HMAC_DOMAIN);
    mac.update(&secret.version.to_be_bytes());
    mac.update(id.0.as_bytes());
    mac.update(&revision.to_be_bytes());
    mac.update(prompt_digest.as_bytes());
    mac
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingTargetWitness {
    pub perceived: PerceivedRef,
    pub revisions: Vec<RevisionStamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOfferWitness {
    pub offer: ActionOfferId,
    pub common_revisions: Vec<RevisionStamp>,
    pub target_revisions: Vec<PendingTargetWitness>,
    pub bridge_definitions: Vec<DefinitionRef>,
}

/// Provider-declared prompt-time revision coordinates. Providers name keys;
/// the kernel supplies current generations and exact bridge definitions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DecisionWitnessKeys {
    pub common: Vec<crate::action::RevisionKey>,
    pub targets: Vec<DecisionTargetWitnessKeys>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionTargetWitnessKeys {
    pub perceived: PerceivedRef,
    pub revisions: Vec<crate::action::RevisionKey>,
}

/// Pre-staging admission result. A replacement is legal only when both
/// records share the exact controller and explicit replaceable cause key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecisionAdmission {
    admitted: DecisionId,
    replaced: Option<DecisionId>,
    resulting_nonterminal: usize,
    resulting_bytes: usize,
}

impl DecisionAdmission {
    #[must_use]
    pub const fn admitted(&self) -> DecisionId {
        self.admitted
    }

    #[must_use]
    pub const fn replaced(&self) -> Option<DecisionId> {
        self.replaced
    }

    #[must_use]
    pub const fn resulting_nonterminal(&self) -> usize {
        self.resulting_nonterminal
    }

    #[must_use]
    pub const fn resulting_bytes(&self) -> usize {
        self.resulting_bytes
    }
}

/// Bounded index assembled from canonical family-scan pages. It retains only
/// identities and accounting data, never prompt or private witness bodies.
#[derive(Clone, Debug, Default)]
pub struct DecisionAdmissionIndex {
    entries: BTreeMap<DecisionId, DecisionAdmissionEntry>,
    controller_counts: BTreeMap<DecisionControllerId, usize>,
    actor_counts: BTreeMap<ObjId, usize>,
    total_bytes: usize,
}

#[derive(Clone, Debug)]
struct DecisionAdmissionEntry {
    controller: DecisionControllerId,
    actor: ObjId,
    replaceable_key: Option<IdempotencyKeyText>,
    bytes: usize,
}

impl DecisionAdmissionIndex {
    /// Construct the bounded non-terminal view used by capture admission.
    /// Passing more than the hard process ceilings is an integrity failure,
    /// not an invitation to truncate the scan.
    pub fn from_pairs<'a>(
        pairs: impl IntoIterator<Item = (&'a PendingDecision, &'a DecisionPromptRecord)>,
        keys: &DecisionKeyRing,
    ) -> Result<Self, DecisionError> {
        let mut index = Self::default();
        for (pending, prompt) in pairs {
            pending.validate_prompt_pair(prompt, keys)?;
            if pending.status.is_terminal() {
                continue;
            }
            let bytes = decision_pair_bytes(pending, prompt)?;
            if index
                .entries
                .insert(
                    pending.id,
                    DecisionAdmissionEntry {
                        controller: pending.controller.clone(),
                        actor: pending.actor,
                        replaceable_key: pending.cause.replaceable_key.clone(),
                        bytes,
                    },
                )
                .is_some()
            {
                return Err(DecisionError::DuplicateDecisionId);
            }
            *index
                .controller_counts
                .entry(pending.controller.clone())
                .or_default() += 1;
            *index.actor_counts.entry(pending.actor).or_default() += 1;
            index.total_bytes = index
                .total_bytes
                .checked_add(bytes)
                .ok_or(DecisionError::Overflow)?;
            index.validate_hard_totals()?;
        }
        Ok(index)
    }

    /// Decide admission before opening an authoritative transaction.
    pub fn admit(
        &self,
        pending: &PendingDecision,
        prompt: &DecisionPromptRecord,
        keys: &DecisionKeyRing,
    ) -> Result<DecisionAdmission, DecisionError> {
        pending.validate_prompt_pair(prompt, keys)?;
        if pending.status.is_terminal() {
            return Err(DecisionError::TerminalCapture);
        }
        if self.entries.contains_key(&pending.id) {
            return Err(DecisionError::DuplicateDecisionId);
        }
        let replacement = pending
            .cause
            .replaceable_key
            .as_ref()
            .map(|replaceable| {
                self.entries
                    .iter()
                    .filter(|(_, entry)| {
                        entry.controller == pending.controller
                            && entry.replaceable_key.as_ref() == Some(replaceable)
                    })
                    .map(|(id, entry)| (*id, entry))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if replacement.len() > 1 {
            return Err(DecisionError::AmbiguousCoalescingCause);
        }
        let replaced = replacement.first().map(|(id, _)| *id);
        let replaced_entry = replacement.first().map(|(_, entry)| *entry);
        let candidate_bytes = decision_pair_bytes(pending, prompt)?;
        let resulting_nonterminal = self
            .entries
            .len()
            .checked_add(1)
            .and_then(|count| count.checked_sub(usize::from(replaced.is_some())))
            .ok_or(DecisionError::Overflow)?;
        let resulting_bytes = self
            .total_bytes
            .checked_add(candidate_bytes)
            .and_then(|bytes| bytes.checked_sub(replaced_entry.map_or(0, |entry| entry.bytes)))
            .ok_or(DecisionError::Overflow)?;
        if resulting_nonterminal > MAX_NONTERMINAL_DECISIONS {
            return Err(DecisionError::GlobalAdmissionLimit);
        }
        if resulting_bytes > MAX_DECISION_TOTAL_BYTES {
            return Err(DecisionError::StorageAdmissionLimit);
        }
        let replaced_same_controller =
            replaced_entry.is_some_and(|entry| entry.controller == pending.controller);
        let controller_count = self
            .controller_counts
            .get(&pending.controller)
            .copied()
            .unwrap_or(0)
            + 1
            - usize::from(replaced_same_controller);
        if controller_count > MAX_NONTERMINAL_DECISIONS_PER_CONTROLLER {
            return Err(DecisionError::ControllerAdmissionLimit);
        }
        let replaced_same_actor = replaced_entry.is_some_and(|entry| entry.actor == pending.actor);
        let actor_count = self.actor_counts.get(&pending.actor).copied().unwrap_or(0) + 1
            - usize::from(replaced_same_actor);
        if actor_count > MAX_NONTERMINAL_DECISIONS_PER_ACTOR {
            return Err(DecisionError::ActorAdmissionLimit);
        }
        Ok(DecisionAdmission {
            admitted: pending.id,
            replaced,
            resulting_nonterminal,
            resulting_bytes,
        })
    }

    fn validate_hard_totals(&self) -> Result<(), DecisionError> {
        if self.entries.len() > MAX_NONTERMINAL_DECISIONS {
            return Err(DecisionError::GlobalAdmissionLimit);
        }
        if self.total_bytes > MAX_DECISION_TOTAL_BYTES {
            return Err(DecisionError::StorageAdmissionLimit);
        }
        if self
            .controller_counts
            .values()
            .any(|count| *count > MAX_NONTERMINAL_DECISIONS_PER_CONTROLLER)
        {
            return Err(DecisionError::ControllerAdmissionLimit);
        }
        if self
            .actor_counts
            .values()
            .any(|count| *count > MAX_NONTERMINAL_DECISIONS_PER_ACTOR)
        {
            return Err(DecisionError::ActorAdmissionLimit);
        }
        Ok(())
    }
}

fn decision_pair_bytes(
    pending: &PendingDecision,
    prompt: &DecisionPromptRecord,
) -> Result<usize, DecisionError> {
    let pending = serde_json::to_value(pending)
        .map_err(|error| DecisionError::Serialization(error.to_string()))?;
    let prompt = serde_json::to_value(prompt)
        .map_err(|error| DecisionError::Serialization(error.to_string()))?;
    let pending = canonical_json_bytes_with_limit(&pending, MAX_DECISION_PENDING_BYTES)
        .map_err(|_| DecisionError::PendingBytes)?;
    let prompt = canonical_json_bytes_with_limit(&prompt, MAX_DECISION_PROMPT_BYTES)
        .map_err(|_| DecisionError::PromptBytes)?;
    pending
        .len()
        .checked_add(prompt.len())
        .ok_or(DecisionError::Overflow)
}

/// Composition-branded owner-local families required for one atomic capture.
#[derive(Clone)]
pub struct DecisionCaptureFamilies {
    pending: StateFamilyHandle<DecisionId, PendingDecision>,
    prompts: StateFamilyHandle<DecisionId, DecisionPromptRecord>,
    deadlines: StateFamilyHandle<ScheduleId, DurableSchedule>,
}

impl fmt::Debug for DecisionCaptureFamilies {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecisionCaptureFamilies")
            .field("owner", self.pending.owner())
            .field("pending", self.pending.descriptor().id())
            .field("prompts", self.prompts.descriptor().id())
            .field("deadlines", self.deadlines.descriptor().id())
            .finish()
    }
}

impl DecisionCaptureFamilies {
    pub fn new(
        pending: StateFamilyHandle<DecisionId, PendingDecision>,
        prompts: StateFamilyHandle<DecisionId, DecisionPromptRecord>,
        deadlines: StateFamilyHandle<ScheduleId, DurableSchedule>,
    ) -> Result<Self, DecisionError> {
        if pending.owner() != prompts.owner() || pending.owner() != deadlines.owner() {
            return Err(DecisionError::CaptureOwnerMismatch);
        }
        Ok(Self {
            pending,
            prompts,
            deadlines,
        })
    }

    #[must_use]
    pub fn owner(&self) -> &OwnerId {
        self.pending.owner()
    }

    #[must_use]
    pub(crate) const fn pending(&self) -> &StateFamilyHandle<DecisionId, PendingDecision> {
        &self.pending
    }

    #[must_use]
    pub(crate) const fn prompts(&self) -> &StateFamilyHandle<DecisionId, DecisionPromptRecord> {
        &self.prompts
    }

    #[must_use]
    pub(crate) const fn deadlines(&self) -> &StateFamilyHandle<ScheduleId, DurableSchedule> {
        &self.deadlines
    }
}

/// Fully validated material staged by a controller-owned authoritative action.
/// Provider I/O is structurally absent from this type.
#[derive(Clone, Debug)]
pub struct DecisionCapturePlan {
    pending: PendingDecision,
    prompt: DecisionPromptRecord,
    deadline: DurableSchedule,
    wake: DecisionWake,
    admission: DecisionAdmission,
    wake_adapter: CanonicalId,
    outbox_ordinal: u16,
}

impl DecisionCapturePlan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: &OwnerId,
        pending: PendingDecision,
        prompt: DecisionPromptRecord,
        admission: DecisionAdmission,
        keys: &DecisionKeyRing,
        wake_adapter: CanonicalId,
        outbox_ordinal: u16,
    ) -> Result<Self, DecisionError> {
        if &pending.controller.owner != owner {
            return Err(DecisionError::CaptureOwnerMismatch);
        }
        if admission.admitted != pending.id {
            return Err(DecisionError::AdmissionIdentityMismatch);
        }
        pending.validate_prompt_pair(&prompt, keys)?;
        let wake = DecisionWake {
            id: pending.id,
            revision: pending.revision,
            prompt_digest: pending.prompt_digest,
        };
        pending.validate_wake(&wake)?;
        let local_id = CanonicalId::parse_canonical(pending.id.0.to_hex())
            .map_err(|error| DecisionError::Canonical(error.to_string()))?;
        let schedule_id = ScheduleId::derive(owner, &local_id)
            .map_err(|error| DecisionError::Canonical(error.to_string()))?;
        let deadline = DurableSchedule::new(DurableScheduleDraft {
            id: schedule_id,
            owner: owner.clone(),
            clock: pending.deadline.clock.clone(),
            next_due: pending.deadline.clone(),
            cadence: None,
            generation: pending.revision,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .map_err(|error| DecisionError::Schedule(error.to_string()))?;
        Ok(Self {
            pending,
            prompt,
            deadline,
            wake,
            admission,
            wake_adapter,
            outbox_ordinal,
        })
    }

    #[must_use]
    pub const fn pending(&self) -> &PendingDecision {
        &self.pending
    }

    #[must_use]
    pub const fn prompt(&self) -> &DecisionPromptRecord {
        &self.prompt
    }

    #[must_use]
    pub const fn deadline(&self) -> &DurableSchedule {
        &self.deadline
    }

    #[must_use]
    pub const fn wake(&self) -> DecisionWake {
        self.wake
    }

    /// Add every authoritative capture post-image to one owner transaction.
    /// If admission selected a predecessor, its terminal transition is staged
    /// before the replacement inserts but commits in the same final bundle.
    pub fn stage(
        self,
        txn: &mut ActionTxn<'_>,
        families: &DecisionCaptureFamilies,
    ) -> Result<DurableOutboxIntent, DecisionError> {
        if txn.owner() != families.owner() || txn.owner() != &self.pending.controller.owner {
            return Err(DecisionError::CaptureOwnerMismatch);
        }
        if let Some(replaced) = self.admission.replaced {
            let mut old = txn
                .get(&families.pending, &replaced)
                .map_err(|error| DecisionError::State(error.to_string()))?
                .cloned()
                .ok_or(DecisionError::CoalescingTargetMissing)?;
            let replacement_key = self
                .pending
                .cause
                .replaceable_key
                .as_ref()
                .ok_or(DecisionError::CoalescingTargetChanged)?;
            if old.controller != self.pending.controller
                || old.cause.replaceable_key.as_ref() != Some(replacement_key)
                || old.status.is_terminal()
            {
                return Err(DecisionError::CoalescingTargetChanged);
            }
            old.finish(old.revision, DecisionTerminalOutcome::Stale, None)?;
            txn.replace(&families.pending, &replaced, old)
                .map_err(|error| DecisionError::State(error.to_string()))?;
        }
        let pending_id = self.pending.id;
        let prompt_id = self.prompt.id;
        txn.insert(&families.pending, &pending_id, self.pending)
            .map_err(|error| DecisionError::State(error.to_string()))?;
        txn.insert(&families.prompts, &prompt_id, self.prompt)
            .map_err(|error| DecisionError::State(error.to_string()))?;
        txn.insert(&families.deadlines, &self.deadline.id(), self.deadline)
            .map_err(|error| DecisionError::State(error.to_string()))?;
        let payload = serde_json::to_value(self.wake)
            .map_err(|error| DecisionError::Serialization(error.to_string()))?;
        DurableOutboxIntent::new(
            self.outbox_ordinal,
            families.owner().clone(),
            self.wake_adapter,
            payload,
        )
        .map_err(|error| DecisionError::Outbox(error.to_string()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionActionBinding {
    pub request: RequestUid,
    pub request_hash: CanonicalHash,
}

/// Public-safe worker input fenced to one exact durable lease. It owns only
/// the persisted projected prompt; no world or private witness reference can
/// cross the async boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionWorkerLease {
    controller: DecisionControllerId,
    decision: DecisionId,
    revision: u64,
    attempt: u8,
    lease_until: ClockStamp,
    prompt: DecisionPrompt,
}

impl DecisionWorkerLease {
    #[must_use]
    pub const fn controller(&self) -> &DecisionControllerId {
        &self.controller
    }

    #[must_use]
    pub const fn decision(&self) -> DecisionId {
        self.decision
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    #[must_use]
    pub const fn lease_until(&self) -> &ClockStamp {
        &self.lease_until
    }

    #[must_use]
    pub const fn prompt(&self) -> &DecisionPrompt {
        &self.prompt
    }
}

/// Exact Ready-to-Leased post-image and the worker custody released only
/// after that post-image receives its authoritative commit acknowledgement.
#[must_use = "the leased post-image must commit before worker custody is released"]
#[derive(Clone, Debug)]
pub struct DecisionLeaseTransition {
    post_image: PendingDecision,
    worker: DecisionWorkerLease,
}

impl DecisionLeaseTransition {
    #[must_use]
    pub const fn post_image(&self) -> &PendingDecision {
        &self.post_image
    }

    #[must_use]
    pub fn into_parts(self) -> (PendingDecision, DecisionWorkerLease) {
        (self.post_image, self.worker)
    }

    /// Stage the exact lease post-image. The returned worker value remains
    /// caller-owned custody and must not be spawned before transaction commit.
    pub fn stage(
        self,
        txn: &mut ActionTxn<'_>,
        families: &DecisionCaptureFamilies,
    ) -> Result<DecisionWorkerLease, DecisionError> {
        if txn.owner() != families.owner() || txn.owner() != &self.post_image.controller.owner {
            return Err(DecisionError::CaptureOwnerMismatch);
        }
        let decision = self.post_image.id;
        txn.replace(&families.pending, &decision, self.post_image)
            .map_err(|error| DecisionError::State(error.to_string()))?;
        Ok(self.worker)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTerminalOutcome {
    Executed,
    Fallback,
    Cancelled,
    Exhausted,
    Stale,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PendingDecisionStatus {
    Ready {
        attempt: u8,
    },
    Leased {
        attempt: u8,
        lease_until: ClockStamp,
    },
    Resolving {
        attempt: u8,
        lease_until: ClockStamp,
        action_binding: DecisionActionBinding,
    },
    Terminal {
        outcome: DecisionTerminalOutcome,
        action_binding: Option<DecisionActionBinding>,
    },
}

impl PendingDecisionStatus {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal { .. })
    }

    fn validate(&self, deadline: &ClockStamp) -> Result<(), DecisionError> {
        let (attempt, lease_until) = match self {
            Self::Ready { attempt } => (*attempt, None),
            Self::Leased {
                attempt,
                lease_until,
            }
            | Self::Resolving {
                attempt,
                lease_until,
                ..
            } => (*attempt, Some(lease_until)),
            Self::Terminal { .. } => return Ok(()),
        };
        if !(1..=MAX_DECISION_ATTEMPTS).contains(&attempt) {
            return Err(DecisionError::Attempt { attempt });
        }
        if let Some(lease_until) = lease_until
            && (lease_until.checked_cmp(deadline).is_err() || lease_until.at > deadline.at)
        {
            return Err(DecisionError::LeaseDeadlineMismatch);
        }
        Ok(())
    }
}

/// Private, trusted persistence record. It deliberately contains no GuardSet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PendingDecision {
    pub id: DecisionId,
    pub revision: u64,
    pub controller: DecisionControllerId,
    pub principal: PrincipalId,
    pub actor: ObjId,
    pub truth_subject: SubjectKey,
    pub viewer_scope: ProjectionScopeId,
    pub cause: DecisionCause,
    pub offered: Vec<PendingOfferWitness>,
    pub prompt_digest: CanonicalHash,
    pub token_key_version: u32,
    pub deadline: ClockStamp,
    pub fallback: FallbackPolicyRef,
    pub action_idempotency: ScopedIdempotencyKey,
    pub malformed_attempts: u8,
    pub status: PendingDecisionStatus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingDecisionWire {
    id: DecisionId,
    revision: u64,
    controller: DecisionControllerId,
    principal: PrincipalId,
    actor: ObjId,
    truth_subject: SubjectKey,
    viewer_scope: ProjectionScopeId,
    cause: DecisionCause,
    offered: Vec<PendingOfferWitness>,
    prompt_digest: CanonicalHash,
    token_key_version: u32,
    deadline: ClockStamp,
    fallback: FallbackPolicyRef,
    action_idempotency: ScopedIdempotencyKey,
    malformed_attempts: u8,
    status: PendingDecisionStatus,
}

impl<'de> Deserialize<'de> for PendingDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let wire = PendingDecisionWire::deserialize(deserializer)?;
        let value = Self {
            id: wire.id,
            revision: wire.revision,
            controller: wire.controller,
            principal: wire.principal,
            actor: wire.actor,
            truth_subject: wire.truth_subject,
            viewer_scope: wire.viewer_scope,
            cause: wire.cause,
            offered: wire.offered,
            prompt_digest: wire.prompt_digest,
            token_key_version: wire.token_key_version,
            deadline: wire.deadline,
            fallback: wire.fallback,
            action_idempotency: wire.action_idempotency,
            malformed_attempts: wire.malformed_attempts,
            status: wire.status,
        };
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

impl PendingDecision {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.revision == 0 {
            return Err(DecisionError::ZeroRevision);
        }
        if self.token_key_version == 0 {
            return Err(DecisionError::ZeroKeyVersion);
        }
        if self.offered.is_empty() || self.offered.len() > MAX_DECISION_OFFERS {
            return Err(DecisionError::OfferCount {
                actual: self.offered.len(),
            });
        }
        if self.malformed_attempts > MAX_DECISION_ATTEMPTS {
            return Err(DecisionError::MalformedAttempts {
                actual: self.malformed_attempts,
            });
        }
        self.status.validate(&self.deadline)?;
        validate_offer_witnesses(&self.offered)?;
        let value = serde_json::to_value(self)
            .map_err(|error| DecisionError::Serialization(error.to_string()))?;
        canonical_json_bytes_with_limit(&value, MAX_DECISION_PENDING_BYTES)
            .map_err(|_| DecisionError::PendingBytes)?;
        Ok(())
    }

    pub fn lease(
        &mut self,
        expected_revision: u64,
        lease_until: ClockStamp,
    ) -> Result<u8, DecisionError> {
        if self.revision != expected_revision {
            return Err(DecisionError::StaleRevision);
        }
        let PendingDecisionStatus::Ready { attempt } = self.status else {
            return Err(DecisionError::InvalidTransition);
        };
        let status = PendingDecisionStatus::Leased {
            attempt,
            lease_until,
        };
        status.validate(&self.deadline)?;
        self.status = status;
        Ok(attempt)
    }

    /// Plan one dispatcher lease from an exact durable wake and prompt pair.
    /// The lease is clipped to the decision deadline and never crosses clock
    /// generations. No async work is started by this method.
    pub fn plan_worker_lease(
        &self,
        prompt: &DecisionPromptRecord,
        wake: &DecisionWake,
        keys: &DecisionKeyRing,
        now: &ClockStamp,
        lease_duration: i64,
    ) -> Result<DecisionLeaseTransition, DecisionError> {
        self.validate_prompt_pair(prompt, keys)?;
        self.validate_wake(wake)?;
        if lease_duration <= 0 {
            return Err(DecisionError::InvalidLeaseDuration);
        }
        self.deadline
            .checked_cmp(now)
            .map_err(|_| DecisionError::LeaseDeadlineMismatch)?;
        if now.at >= self.deadline.at {
            return Err(DecisionError::DeadlineReached);
        }
        let proposed = now
            .at
            .checked_add(lease_duration)
            .ok_or(DecisionError::Overflow)?;
        let lease_until = ClockStamp {
            clock: now.clock.clone(),
            at: proposed.min(self.deadline.at),
        };
        let mut post_image = self.clone();
        let attempt = post_image.lease(self.revision, lease_until.clone())?;
        Ok(DecisionLeaseTransition {
            post_image,
            worker: DecisionWorkerLease {
                controller: self.controller.clone(),
                decision: self.id,
                revision: self.revision,
                attempt,
                lease_until,
                prompt: prompt.prompt.clone(),
            },
        })
    }

    pub fn begin_resolving(
        &mut self,
        expected_revision: u64,
        expected_attempt: u8,
        expected_lease_until: &ClockStamp,
        binding: DecisionActionBinding,
    ) -> Result<(), DecisionError> {
        if self.revision != expected_revision {
            return Err(DecisionError::StaleRevision);
        }
        let PendingDecisionStatus::Leased {
            attempt,
            ref lease_until,
        } = self.status
        else {
            return Err(DecisionError::InvalidTransition);
        };
        if attempt != expected_attempt {
            return Err(DecisionError::StaleAttempt);
        }
        if lease_until != expected_lease_until {
            return Err(DecisionError::StaleLease);
        }
        self.status = PendingDecisionStatus::Resolving {
            attempt,
            lease_until: lease_until.clone(),
            action_binding: binding,
        };
        Ok(())
    }

    pub fn retry_expired_lease(
        &mut self,
        expected_revision: u64,
        now: &ClockStamp,
    ) -> Result<u8, DecisionError> {
        if self.revision != expected_revision {
            return Err(DecisionError::StaleRevision);
        }
        let PendingDecisionStatus::Leased {
            attempt,
            ref lease_until,
        } = self.status
        else {
            return Err(DecisionError::InvalidTransition);
        };
        if lease_until.checked_cmp(now).is_err() || now.at < lease_until.at {
            return Err(DecisionError::LeaseNotExpired);
        }
        let next = attempt.checked_add(1).ok_or(DecisionError::Overflow)?;
        if next > MAX_DECISION_ATTEMPTS {
            return Err(DecisionError::AttemptsExhausted);
        }
        self.status = PendingDecisionStatus::Ready { attempt: next };
        Ok(next)
    }

    pub fn finish(
        &mut self,
        expected_revision: u64,
        outcome: DecisionTerminalOutcome,
        action_binding: Option<DecisionActionBinding>,
    ) -> Result<(), DecisionError> {
        if self.revision != expected_revision {
            return Err(DecisionError::StaleRevision);
        }
        if self.status.is_terminal() {
            return Err(DecisionError::InvalidTransition);
        }
        self.status = PendingDecisionStatus::Terminal {
            outcome,
            action_binding,
        };
        Ok(())
    }

    /// Validate the exact private/public pair before capture, dispatch, or
    /// recovery. The prompt revision is semantic and remains stable across
    /// operational lease attempts.
    pub fn validate_prompt_pair(
        &self,
        prompt: &DecisionPromptRecord,
        keys: &DecisionKeyRing,
    ) -> Result<(), DecisionError> {
        self.validate()?;
        prompt.validate()?;
        if self.id != prompt.id
            || self.revision != prompt.revision
            || self.prompt_digest != prompt.prompt_digest
            || self.token_key_version != prompt.prompt.token.key_version
            || !keys.verify(&prompt.prompt.token)
            || self.offered.len() != prompt.prompt.actions.len()
            || self
                .offered
                .iter()
                .zip(&prompt.prompt.actions)
                .any(|(private, public)| private.offer != public.offer())
        {
            return Err(DecisionError::PromptLinkage);
        }
        for (private, public) in self.offered.iter().zip(&prompt.prompt.actions) {
            match public.targets() {
                ProjectedTargetSchema::None => {
                    if !private.target_revisions.is_empty() {
                        return Err(DecisionError::PromptTargetLinkage);
                    }
                }
                ProjectedTargetSchema::Choices {
                    maximum, choices, ..
                } => {
                    if private.target_revisions.len() != choices.len()
                        || private
                            .target_revisions
                            .iter()
                            .map(|witness| &witness.perceived)
                            .collect::<BTreeSet<_>>()
                            != choices.iter().collect::<BTreeSet<_>>()
                    {
                        return Err(DecisionError::PromptTargetLinkage);
                    }
                    let mut largest: Vec<usize> = private
                        .target_revisions
                        .iter()
                        .map(|target| target.revisions.len())
                        .collect();
                    largest.sort_unstable_by(|left, right| right.cmp(left));
                    if private.common_revisions.len()
                        + largest.into_iter().take(*maximum).sum::<usize>()
                        > MAX_ACTION_REVISIONS
                    {
                        return Err(DecisionError::SelectedWitnessCount);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn validate_wake(&self, wake: &DecisionWake) -> Result<(), DecisionError> {
        wake.validate()?;
        if self.id != wake.id
            || self.revision != wake.revision
            || self.prompt_digest != wake.prompt_digest
        {
            return Err(DecisionError::WakeLinkage);
        }
        Ok(())
    }
}

fn validate_offer_witnesses(offered: &[PendingOfferWitness]) -> Result<(), DecisionError> {
    if offered
        .iter()
        .map(|witness| witness.offer)
        .collect::<BTreeSet<_>>()
        .len()
        != offered.len()
    {
        return Err(DecisionError::DuplicateOffer);
    }
    for witness in offered {
        validate_revision_order(&witness.common_revisions)?;
        if witness.target_revisions.len() > MAX_DECISION_TARGET_WITNESSES {
            return Err(DecisionError::TargetCount {
                actual: witness.target_revisions.len(),
            });
        }
        if witness
            .target_revisions
            .windows(2)
            .any(|pair| pair[0].perceived >= pair[1].perceived)
        {
            return Err(DecisionError::TargetOrder);
        }
        for target in &witness.target_revisions {
            validate_revision_order(&target.revisions)?;
            if witness.common_revisions.len() + target.revisions.len() > MAX_ACTION_REVISIONS {
                return Err(DecisionError::SelectedWitnessCount);
            }
        }
        if witness.bridge_definitions.len() > MAX_DECISION_BRIDGE_DEFINITIONS {
            return Err(DecisionError::BridgeCount);
        }
        if witness
            .bridge_definitions
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(DecisionError::BridgeOrder);
        }
    }
    Ok(())
}

fn validate_revision_order(revisions: &[RevisionStamp]) -> Result<(), DecisionError> {
    if revisions.len() > MAX_ACTION_REVISIONS {
        return Err(DecisionError::SelectedWitnessCount);
    }
    if revisions.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(DecisionError::RevisionOrder);
    }
    Ok(())
}

/// Exact public prompt. It contains no private truth or revision coordinates.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionPrompt {
    pub token: DecisionToken,
    pub actions: Vec<ProjectedLegalAction>,
    pub context: ProjectedDecisionContext,
}

impl DecisionPrompt {
    pub fn digest(&self) -> Result<CanonicalHash, DecisionError> {
        if self.actions.is_empty() || self.actions.len() > MAX_DECISION_OFFERS {
            return Err(DecisionError::OfferCount {
                actual: self.actions.len(),
            });
        }
        if self
            .actions
            .iter()
            .map(ProjectedLegalAction::offer)
            .collect::<BTreeSet<_>>()
            .len()
            != self.actions.len()
        {
            return Err(DecisionError::DuplicateOffer);
        }
        self.context.validate()?;
        CanonicalHash::for_canonical_value(
            DECISION_PROMPT_HASH_DOMAIN,
            1,
            &json!({ "actions": self.actions, "context": self.context }),
            MAX_DECISION_PROMPT_BYTES,
        )
        .map_err(|error| DecisionError::Canonical(error.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionPromptRecord {
    pub id: DecisionId,
    pub revision: u64,
    pub prompt_digest: CanonicalHash,
    pub prompt: DecisionPrompt,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionPromptRecordWire {
    id: DecisionId,
    revision: u64,
    prompt_digest: CanonicalHash,
    prompt: DecisionPrompt,
}

impl<'de> Deserialize<'de> for DecisionPromptRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let wire = DecisionPromptRecordWire::deserialize(deserializer)?;
        let value = Self {
            id: wire.id,
            revision: wire.revision,
            prompt_digest: wire.prompt_digest,
            prompt: wire.prompt,
        };
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

impl DecisionPromptRecord {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(prompt: DecisionPrompt) -> Result<Self, DecisionError> {
        let value = Self {
            id: prompt.token.id,
            revision: prompt.token.revision,
            prompt_digest: prompt.token.prompt_digest,
            prompt,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.revision == 0 {
            return Err(DecisionError::ZeroRevision);
        }
        if self.id != self.prompt.token.id
            || self.revision != self.prompt.token.revision
            || self.prompt_digest != self.prompt.token.prompt_digest
            || self.prompt.digest()? != self.prompt_digest
        {
            return Err(DecisionError::PromptLinkage);
        }
        let value = serde_json::to_value(self)
            .map_err(|error| DecisionError::Serialization(error.to_string()))?;
        canonical_json_bytes_with_limit(&value, MAX_DECISION_PROMPT_BYTES)
            .map_err(|_| DecisionError::PromptBytes)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionWake {
    pub id: DecisionId,
    pub revision: u64,
    pub prompt_digest: CanonicalHash,
}

impl DecisionWake {
    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.revision == 0 {
            return Err(DecisionError::ZeroRevision);
        }
        let value = serde_json::to_value(self)
            .map_err(|error| DecisionError::Serialization(error.to_string()))?;
        canonical_json_bytes_with_limit(&value, MAX_DECISION_WAKE_BYTES)
            .map_err(|_| DecisionError::WakeBytes)?;
        Ok(())
    }
}

/// Untrusted provider result. Truth identities and guards are unrepresentable.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionResponse {
    pub token: DecisionToken,
    pub offer: ActionOfferId,
    pub targets: Vec<PerceivedRef>,
    pub parameters: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionResponseWire {
    token: DecisionToken,
    offer: ActionOfferId,
    targets: Vec<PerceivedRef>,
    parameters: Value,
}

impl<'de> Deserialize<'de> for DecisionResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let wire = DecisionResponseWire::deserialize(deserializer)?;
        let value = Self {
            token: wire.token,
            offer: wire.offer,
            targets: wire.targets,
            parameters: wire.parameters,
        };
        value.validate().map_err(D::Error::custom)?;
        Ok(value)
    }
}

impl DecisionResponse {
    pub fn validate(&self) -> Result<(), DecisionError> {
        if self.targets.len() > MAX_DECISION_TARGETS {
            return Err(DecisionError::TargetCount {
                actual: self.targets.len(),
            });
        }
        let targets: BTreeSet<_> = self.targets.iter().collect();
        if targets.len() != self.targets.len() {
            return Err(DecisionError::DuplicateTarget);
        }
        let value = serde_json::to_value(self)
            .map_err(|error| DecisionError::Serialization(error.to_string()))?;
        canonical_json_bytes_with_limit(&value, MAX_DECISION_RESPONSE_BYTES)
            .map_err(|_| DecisionError::ResponseBytes)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PendingDecisionStateCodec;

impl StateKeyCodec<DecisionId> for PendingDecisionStateCodec {
    fn encode_key(&self, key: &DecisionId) -> Result<String, String> {
        Ok(key.0.to_hex())
    }
}

impl StateValueDecoder<PendingDecision> for PendingDecisionStateCodec {
    fn decode_value(&self, value: &Value) -> Result<PendingDecision, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<PendingDecision> for PendingDecisionStateCodec {
    fn validate_value(&self, value: &PendingDecision) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<PendingDecision> for PendingDecisionStateCodec {
    fn serialize_value(&self, value: &PendingDecision) -> Result<Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DecisionPromptStateCodec;

impl StateKeyCodec<DecisionId> for DecisionPromptStateCodec {
    fn encode_key(&self, key: &DecisionId) -> Result<String, String> {
        Ok(key.0.to_hex())
    }
}

impl StateValueDecoder<DecisionPromptRecord> for DecisionPromptStateCodec {
    fn decode_value(&self, value: &Value) -> Result<DecisionPromptRecord, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<DecisionPromptRecord> for DecisionPromptStateCodec {
    fn validate_value(&self, value: &DecisionPromptRecord) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<DecisionPromptRecord> for DecisionPromptStateCodec {
    fn serialize_value(&self, value: &DecisionPromptRecord) -> Result<Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

pub fn pending_decision_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("decision_id.v1").expect("static canonical id"),
        CanonicalId::new("pending_decision.v1").expect("static canonical id"),
        PendingDecision::SCHEMA_VERSION,
        CanonicalId::new("pending_decision_decode.v1").expect("static canonical id"),
        CanonicalId::new("pending_decision_validate.v1").expect("static canonical id"),
        CanonicalId::new("pending_decision_serialize.v1").expect("static canonical id"),
    )?
    .with_limits(64, MAX_DECISION_PENDING_BYTES)
}

pub fn decision_prompt_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("decision_id.v1").expect("static canonical id"),
        CanonicalId::new("decision_prompt_record.v1").expect("static canonical id"),
        DecisionPromptRecord::SCHEMA_VERSION,
        CanonicalId::new("decision_prompt_decode.v1").expect("static canonical id"),
        CanonicalId::new("decision_prompt_validate.v1").expect("static canonical id"),
        CanonicalId::new("decision_prompt_serialize.v1").expect("static canonical id"),
    )?
    .with_limits(64, MAX_DECISION_PROMPT_BYTES)
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecisionSecretError {
    #[error("decision secret version zero is reserved")]
    ZeroVersion,
    #[error("an all-zero decision secret is not accepted")]
    ZeroKey,
    #[error("retained decision-secret versions must precede the current version")]
    NonMonotonicVersion,
    #[error("decision secret version appears more than once")]
    DuplicateVersion,
    #[error("decision key ring exceeds its hard limit of {max} versions")]
    VersionLimit { max: usize },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecisionError {
    #[error("decision revision zero is reserved")]
    ZeroRevision,
    #[error("decision token key version zero is reserved")]
    ZeroKeyVersion,
    #[error("decision contains {actual} offers; expected 1..={MAX_DECISION_OFFERS}")]
    OfferCount { actual: usize },
    #[error("decision contains a duplicate offer")]
    DuplicateOffer,
    #[error("decision identity already exists")]
    DuplicateDecisionId,
    #[error("more than one pending decision has the replaceable cause")]
    AmbiguousCoalescingCause,
    #[error("a terminal decision cannot be admitted as new work")]
    TerminalCapture,
    #[error("capture plan does not match its pre-staging admission identity")]
    AdmissionIdentityMismatch,
    #[error("global non-terminal decision limit reached")]
    GlobalAdmissionLimit,
    #[error("per-controller non-terminal decision limit reached")]
    ControllerAdmissionLimit,
    #[error("per-actor non-terminal decision limit reached")]
    ActorAdmissionLimit,
    #[error("total non-terminal decision storage limit reached")]
    StorageAdmissionLimit,
    #[error(
        "decision capture families, transaction, controller, or schedule have different owners"
    )]
    CaptureOwnerMismatch,
    #[error("coalescing predecessor no longer exists")]
    CoalescingTargetMissing,
    #[error("coalescing predecessor is no longer the admitted replaceable decision")]
    CoalescingTargetChanged,
    #[error("decision contains {actual} targets; maximum is {MAX_DECISION_TARGETS}")]
    TargetCount { actual: usize },
    #[error("decision target witnesses must be strictly perceived-ref sorted")]
    TargetOrder,
    #[error("decision response contains a duplicate target")]
    DuplicateTarget,
    #[error("revision witnesses must be strictly revision-key sorted")]
    RevisionOrder,
    #[error("common plus selected-target witnesses exceed {MAX_ACTION_REVISIONS}")]
    SelectedWitnessCount,
    #[error("bridge-definition witnesses exceed {MAX_DECISION_BRIDGE_DEFINITIONS}")]
    BridgeCount,
    #[error("bridge definitions must be strictly definition-ref sorted")]
    BridgeOrder,
    #[error("malformed attempt count {actual} exceeds {MAX_DECISION_ATTEMPTS}")]
    MalformedAttempts { actual: u8 },
    #[error("attempt {attempt} is outside 1..={MAX_DECISION_ATTEMPTS}")]
    Attempt { attempt: u8 },
    #[error("lease deadline uses a different clock or exceeds the decision deadline")]
    LeaseDeadlineMismatch,
    #[error("decision context exceeds {MAX_DECISION_CONTEXT_BYTES} canonical bytes")]
    ContextBytes,
    #[error("pending decision exceeds {MAX_DECISION_PENDING_BYTES} canonical bytes")]
    PendingBytes,
    #[error("decision prompt exceeds {MAX_DECISION_PROMPT_BYTES} canonical bytes")]
    PromptBytes,
    #[error("decision response exceeds {MAX_DECISION_RESPONSE_BYTES} canonical bytes")]
    ResponseBytes,
    #[error("decision wake exceeds {MAX_DECISION_WAKE_BYTES} canonical bytes")]
    WakeBytes,
    #[error("wake id, revision, or digest linkage is invalid")]
    WakeLinkage,
    #[error("prompt id, revision, digest, or token linkage is invalid")]
    PromptLinkage,
    #[error("private target witnesses do not match the projected target choices")]
    PromptTargetLinkage,
    #[error("decision revision changed before transition")]
    StaleRevision,
    #[error("decision attempt changed before transition")]
    StaleAttempt,
    #[error("decision lease changed before transition")]
    StaleLease,
    #[error("decision lifecycle transition is invalid")]
    InvalidTransition,
    #[error("decision lease has not expired on its exact clock")]
    LeaseNotExpired,
    #[error("decision worker lease duration must be positive")]
    InvalidLeaseDuration,
    #[error("decision deadline has already been reached")]
    DeadlineReached,
    #[error("decision provider attempts are exhausted")]
    AttemptsExhausted,
    #[error("decision counter overflow")]
    Overflow,
    #[error("decision canonical encoding failed: {0}")]
    Canonical(String),
    #[error("decision serialization failed: {0}")]
    Serialization(String),
    #[error("decision typed-state staging failed: {0}")]
    State(String),
    #[error("decision deadline schedule is invalid: {0}")]
    Schedule(String),
    #[error("decision wake outbox intent is invalid: {0}")]
    Outbox(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{IdempotencyKey, IdempotencyScope};
    use crate::clock::ClockRef;
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
        MechanicalOwnerReadiness,
    };
    use crate::{ActionStateStore, DurableScheduleStateCodec, StateFamilyRegistryBuilder};

    fn principal() -> PrincipalId {
        PrincipalId::Object { id: ObjId(7) }
    }

    fn controller() -> DecisionControllerId {
        DecisionControllerId {
            owner: OwnerId::new("test_decisions").unwrap(),
            key: CanonicalId::new("guard_7").unwrap(),
        }
    }

    fn cause() -> DecisionCause {
        DecisionCause {
            kind: CanonicalId::new("npc_turn").unwrap(),
            instance: IdempotencyKeyText::parse_canonical("scene-4/turn-11").unwrap(),
            replaceable_key: None,
        }
    }

    fn idempotency() -> ScopedIdempotencyKey {
        ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal(),
                owner: controller().owner,
                namespace: CanonicalId::new("decision_action").unwrap(),
            },
            3,
            IdempotencyKey::Deterministic(CanonicalHash::from_bytes([0x22; 32])),
        )
    }

    fn deadline(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: 4,
                generation: 2,
            },
            at,
        }
    }

    fn fallback() -> FallbackPolicyRef {
        let mut definitions = DefinitionRegistryBuilder::new();
        let reference = definitions
            .register(
                controller().owner,
                DefinitionDescriptor::new(
                    DefinitionKind::new("fallback").unwrap(),
                    DefinitionId::new("withdraw").unwrap(),
                    1,
                    json!({ "policy": "withdraw" }),
                )
                .unwrap(),
            )
            .unwrap();
        FallbackPolicyRef(reference)
    }

    fn projected_action(offer: ActionOfferId) -> ProjectedLegalAction {
        serde_json::from_value(json!({
            "offer": offer,
            "subject": { "kind": "self_actor" },
            "parameters": [],
            "targets": { "kind": "none" },
            "required_channels": [],
            "expires": null,
            "presentation": {
                "verb": "withdraw",
                "label": "Withdraw",
                "help": "Move toward safety."
            }
        }))
        .unwrap()
    }

    fn pair() -> (DecisionKeyRing, PendingDecision, DecisionPromptRecord) {
        let controller = controller();
        let cause = cause();
        let idempotency = idempotency();
        let id = DecisionId::derive(&controller, &cause, &idempotency).unwrap();
        let offer = ActionOfferId(CanonicalHash::from_bytes([0x44; 32]));
        let key_ring = DecisionKeyRing::new(
            DecisionSecret::new(2, [0xA5; 32]).unwrap(),
            [DecisionSecret::new(1, [0x5A; 32]).unwrap()],
        )
        .unwrap();
        let context = ProjectedDecisionContext::new(
            CanonicalId::new("tactical_turn.v1").unwrap(),
            json!({ "situation": "under fire" }),
        )
        .unwrap();
        let placeholder = DecisionToken {
            id,
            revision: 1,
            prompt_digest: CanonicalHash::from_bytes([1; 32]),
            key_version: 2,
            tag: CanonicalHash::from_bytes([1; 32]),
        };
        let mut prompt = DecisionPrompt {
            token: placeholder,
            actions: vec![projected_action(offer)],
            context,
        };
        let digest = prompt.digest().unwrap();
        prompt.token = key_ring.sign(id, 1, digest);
        let prompt = DecisionPromptRecord::new(prompt).unwrap();
        let pending = PendingDecision {
            id,
            revision: 1,
            controller,
            principal: principal(),
            actor: ObjId(7),
            truth_subject: SubjectKey::Object { id: ObjId(7) },
            viewer_scope: ProjectionScopeId::derive(&principal(), 9).unwrap(),
            cause,
            offered: vec![PendingOfferWitness {
                offer,
                common_revisions: Vec::new(),
                target_revisions: Vec::new(),
                bridge_definitions: Vec::new(),
            }],
            prompt_digest: digest,
            token_key_version: 2,
            deadline: deadline(20),
            fallback: fallback(),
            action_idempotency: idempotency,
            malformed_attempts: 0,
            status: PendingDecisionStatus::Ready { attempt: 1 },
        };
        pending.validate().unwrap();
        (key_ring, pending, prompt)
    }

    fn derived_pair(
        keys: &DecisionKeyRing,
        template: &PendingDecision,
        prompt_template: &DecisionPromptRecord,
        salt: u8,
        actor: ObjId,
        replaceable_key: Option<&str>,
    ) -> (PendingDecision, DecisionPromptRecord) {
        let mut pending = template.clone();
        pending.actor = actor;
        pending.cause.instance =
            IdempotencyKeyText::parse_canonical(format!("turn-{salt}")).unwrap();
        pending.cause.replaceable_key = replaceable_key
            .map(IdempotencyKeyText::parse_canonical)
            .transpose()
            .unwrap();
        pending.action_idempotency = ScopedIdempotencyKey::new(
            pending.action_idempotency.scope().clone(),
            pending.action_idempotency.epoch(),
            IdempotencyKey::Deterministic(CanonicalHash::from_bytes([salt; 32])),
        );
        pending.id = DecisionId::derive(
            &pending.controller,
            &pending.cause,
            &pending.action_idempotency,
        )
        .unwrap();
        let mut prompt = prompt_template.prompt.clone();
        prompt.token = keys.sign(pending.id, pending.revision, pending.prompt_digest);
        let prompt = DecisionPromptRecord::new(prompt).unwrap();
        (pending, prompt)
    }

    fn capture_store() -> (ActionStateStore, DecisionCaptureFamilies) {
        let owner = controller().owner;
        let mut builder = StateFamilyRegistryBuilder::new();
        let pending = builder
            .register(
                owner.clone(),
                pending_decision_state_family_descriptor(
                    StateFamilyId::new("pending_decisions").unwrap(),
                )
                .unwrap(),
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
                PendingDecisionStateCodec,
            )
            .unwrap();
        let prompts = builder
            .register(
                owner.clone(),
                decision_prompt_state_family_descriptor(
                    StateFamilyId::new("decision_prompts").unwrap(),
                )
                .unwrap(),
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
                DecisionPromptStateCodec,
            )
            .unwrap();
        let deadlines = builder
            .register(
                owner.clone(),
                crate::durable_schedule_state_family_descriptor(
                    StateFamilyId::new("decision_deadlines").unwrap(),
                )
                .unwrap(),
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
            )
            .unwrap();
        let owners = BTreeSet::from([owner]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let registry = builder
            .freeze_for_owners_with_readiness(&owners, readiness)
            .unwrap();
        (
            ActionStateStore::new(registry),
            DecisionCaptureFamilies::new(pending, prompts, deadlines).unwrap(),
        )
    }

    #[test]
    fn token_authenticates_exact_decision_revision_and_prompt_digest() {
        let (keys, _, prompt) = pair();
        assert!(keys.verify(&prompt.prompt.token));
        let mut tampered = prompt.prompt.token;
        tampered.revision += 1;
        assert!(!keys.verify(&tampered));
        tampered = prompt.prompt.token;
        tampered.prompt_digest = CanonicalHash::from_bytes([0x77; 32]);
        assert!(!keys.verify(&tampered));

        let debug = format!("{:?}", DecisionSecret::new(4, [0xAB; 32]).unwrap());
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("171"));
    }

    #[test]
    fn strict_codecs_round_trip_and_reject_unknown_or_tampered_prompt() {
        let (keys, pending, prompt) = pair();
        pending.validate_prompt_pair(&prompt, &keys).unwrap();
        pending
            .validate_wake(&DecisionWake {
                id: pending.id,
                revision: pending.revision,
                prompt_digest: pending.prompt_digest,
            })
            .unwrap();
        let pending_json = serde_json::to_value(&pending).unwrap();
        let decoded: PendingDecision = serde_json::from_value(pending_json.clone()).unwrap();
        assert_eq!(decoded, pending);

        let mut unknown = pending_json;
        unknown
            .as_object_mut()
            .unwrap()
            .insert("private_guards".to_owned(), json!([]));
        assert!(serde_json::from_value::<PendingDecision>(unknown).is_err());

        let mut tampered = serde_json::to_value(&prompt).unwrap();
        tampered["prompt"]["context"]["value"]["situation"] = json!("changed");
        assert!(serde_json::from_value::<DecisionPromptRecord>(tampered).is_err());

        assert_eq!(
            PendingDecisionStateCodec.encode_key(&pending.id).unwrap(),
            pending.id.0.to_hex()
        );
        assert_eq!(
            DecisionPromptStateCodec.encode_key(&prompt.id).unwrap(),
            prompt.id.0.to_hex()
        );
    }

    #[test]
    fn lease_transitions_are_revision_attempt_and_clock_bound() {
        let (_, mut pending, _) = pair();
        assert_eq!(pending.lease(1, deadline(12)).unwrap(), 1);
        assert_eq!(pending.revision, 1);
        assert_eq!(
            pending.lease(2, deadline(13)).unwrap_err(),
            DecisionError::StaleRevision
        );
        assert_eq!(
            pending.retry_expired_lease(1, &deadline(11)).unwrap_err(),
            DecisionError::LeaseNotExpired
        );
        assert_eq!(pending.retry_expired_lease(1, &deadline(12)).unwrap(), 2);
        assert_eq!(pending.revision, 1);
        assert_eq!(pending.lease(1, deadline(15)).unwrap(), 2);
        let binding = DecisionActionBinding {
            request: RequestUid::from_bytes([9; 16]),
            request_hash: CanonicalHash::from_bytes([8; 32]),
        };
        assert_eq!(
            pending
                .begin_resolving(1, 2, &deadline(14), binding.clone())
                .unwrap_err(),
            DecisionError::StaleLease
        );
        pending
            .begin_resolving(1, 2, &deadline(15), binding)
            .unwrap();
        assert!(matches!(
            pending.status,
            PendingDecisionStatus::Resolving { attempt: 2, .. }
        ));
    }

    #[test]
    fn worker_lease_is_public_safe_exact_and_clipped_to_deadline() {
        let (keys, pending, prompt) = pair();
        let wake = DecisionWake {
            id: pending.id,
            revision: pending.revision,
            prompt_digest: pending.prompt_digest,
        };
        let transition = pending
            .plan_worker_lease(&prompt, &wake, &keys, &deadline(18), 10)
            .unwrap();
        assert!(matches!(
            transition.post_image().status,
            PendingDecisionStatus::Leased { attempt: 1, .. }
        ));
        let (post_image, worker) = transition.into_parts();
        assert_eq!(worker.decision(), pending.id);
        assert_eq!(worker.revision(), pending.revision);
        assert_eq!(worker.attempt(), 1);
        assert_eq!(worker.lease_until(), &pending.deadline);
        assert_eq!(worker.prompt(), &prompt.prompt);
        assert_eq!(post_image.revision, prompt.revision);
        assert_eq!(
            pending
                .plan_worker_lease(&prompt, &wake, &keys, &deadline(20), 1)
                .unwrap_err(),
            DecisionError::DeadlineReached
        );
    }

    #[test]
    fn response_and_offer_bounds_fail_before_resolution() {
        let (_, _, prompt) = pair();
        let response = DecisionResponse {
            token: prompt.prompt.token,
            offer: prompt.prompt.actions[0].offer(),
            targets: (0..=MAX_DECISION_TARGETS)
                .map(|index| PerceivedRef::KnownObject {
                    id: ObjId(u32::try_from(index + 1).unwrap()),
                    revision: 1,
                })
                .collect(),
            parameters: json!({}),
        };
        assert!(matches!(
            response.validate(),
            Err(DecisionError::TargetCount { .. })
        ));

        let mut too_many = pair().1;
        let witness = too_many.offered[0].clone();
        too_many.offered = (0..=MAX_DECISION_OFFERS)
            .map(|index| PendingOfferWitness {
                offer: ActionOfferId(CanonicalHash::from_bytes(
                    [u8::try_from(index).unwrap(); 32],
                )),
                ..witness.clone()
            })
            .collect();
        assert!(matches!(
            too_many.validate(),
            Err(DecisionError::OfferCount { .. })
        ));
    }

    #[test]
    fn admission_refuses_actor_overflow_and_coalesces_only_explicit_causes() {
        let (keys, template, prompt_template) = pair();
        let (first, first_prompt) = derived_pair(
            &keys,
            &template,
            &prompt_template,
            10,
            ObjId(9),
            Some("patrol"),
        );
        let (second, second_prompt) =
            derived_pair(&keys, &template, &prompt_template, 11, ObjId(9), None);
        let index = DecisionAdmissionIndex::from_pairs(
            [(&first, &first_prompt), (&second, &second_prompt)],
            &keys,
        )
        .unwrap();

        let (overflow, overflow_prompt) =
            derived_pair(&keys, &template, &prompt_template, 12, ObjId(9), None);
        assert_eq!(
            index.admit(&overflow, &overflow_prompt, &keys).unwrap_err(),
            DecisionError::ActorAdmissionLimit
        );

        let (replacement, replacement_prompt) = derived_pair(
            &keys,
            &template,
            &prompt_template,
            13,
            ObjId(9),
            Some("patrol"),
        );
        let admission = index
            .admit(&replacement, &replacement_prompt, &keys)
            .unwrap();
        assert_eq!(admission.replaced(), Some(first.id));
        assert_eq!(admission.resulting_nonterminal(), 2);
    }

    #[test]
    fn capture_stages_pending_prompt_deadline_and_wake_together() {
        let (keys, pending, prompt) = pair();
        let admission = DecisionAdmissionIndex::default()
            .admit(&pending, &prompt, &keys)
            .unwrap();
        let plan = DecisionCapturePlan::new(
            &pending.controller.owner,
            pending.clone(),
            prompt.clone(),
            admission,
            &keys,
            CanonicalId::new("decision_wake").unwrap(),
            4,
        )
        .unwrap();
        assert_eq!(plan.deadline().next_due(), &pending.deadline);
        assert_eq!(plan.deadline().cadence(), None);
        let (mut store, families) = capture_store();
        let mut txn = store.transaction(&families.pending).unwrap();
        let outbox = plan.stage(&mut txn, &families).unwrap();
        assert_eq!(outbox.ordinal(), 4);
        assert_eq!(
            serde_json::from_value::<DecisionWake>(outbox.payload().clone()).unwrap(),
            DecisionWake {
                id: pending.id,
                revision: pending.revision,
                prompt_digest: pending.prompt_digest,
            }
        );
        let prepared = txn.prepare().unwrap();
        let (writes, installation) = prepared.into_parts();
        assert_eq!(writes.len(), 3);
        store.install(installation);
        assert_eq!(
            store.get(&families.pending, &pending.id).unwrap(),
            Some(&pending)
        );
        assert_eq!(
            store.get(&families.prompts, &prompt.id).unwrap(),
            Some(&prompt)
        );
    }
}
