//! Rules-neutral action windows and durable simultaneous-intent collection.
//!
//! The kernel owns bounded lifecycle and deterministic ordering. Rulesets own
//! initiative semantics and provide the explicit lexicographic order key.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, AuthorityGuard, AuthorityGuardContext,
    AuthorityGuardValidator, CanonicalHash, CanonicalHashError, CommitToken, RequestUid,
    SubjectKey,
};
use crate::clock::{ClockMismatch, ClockRef, ClockStamp};
use crate::mechanics::{CanonicalId, OwnerId};
use crate::{
    StateFamilyDescriptor, StateFamilyDescriptorError, StateFamilyHandle, StateFamilyId,
    StateKeyCodec, StateValueDecoder, StateValueSerializer, StateValueValidator,
};

/// Hard cap on simultaneous intents retained by one window.
pub const MAX_WINDOW_CAPACITY: u32 = 256;
/// Hard cap on idempotency receipts retained through one window lifecycle.
pub const MAX_WINDOW_OPERATION_RECEIPTS: usize = 1_024;
/// Two receipt slots are reserved for close and consume/cancel so input churn
/// cannot prevent a window from reaching a terminal state.
pub const MAX_WINDOW_INPUT_OPERATIONS: usize = MAX_WINDOW_OPERATION_RECEIPTS - 2;
/// Hard cap on ruleset-provided deterministic ordering components.
pub const MAX_WINDOW_ORDER_KEY_PARTS: usize = 16;
/// Hard cap on provider catch-up work after a pause or restart.
pub const MAX_WINDOW_CATCH_UP: u32 = 1_024;

/// Stable identity of a logical action window across lifecycle revisions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ActionWindowId(pub CanonicalHash);

impl ActionWindowId {
    /// Derive a stable identity from owner-local campaign state rather than
    /// process allocation order.
    pub fn derive(owner: &OwnerId, local_id: &CanonicalId) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "action_window_id.v1",
            1,
            &json!({ "owner": owner, "local_id": local_id }),
            16 * 1024,
        )
        .map(Self)
    }

    #[must_use]
    pub const fn canonical_hash(self) -> CanonicalHash {
        self.0
    }
}

/// Canonical engine contract for owner-registered action-window state.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionWindowStateCodec;

impl StateKeyCodec<ActionWindowId> for ActionWindowStateCodec {
    fn encode_key(&self, key: &ActionWindowId) -> Result<String, String> {
        Ok(key.canonical_hash().to_hex())
    }
}

impl StateValueDecoder<ActionWindowRecord> for ActionWindowStateCodec {
    fn decode_value(&self, value: &serde_json::Value) -> Result<ActionWindowRecord, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<ActionWindowRecord> for ActionWindowStateCodec {
    fn validate_value(&self, value: &ActionWindowRecord) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<ActionWindowRecord> for ActionWindowStateCodec {
    fn serialize_value(&self, value: &ActionWindowRecord) -> Result<serde_json::Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

/// Build the exact reusable descriptor for an owner-local window family.
pub fn action_window_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("action_window_id.v1").expect("static canonical id"),
        CanonicalId::new("action_window_record.v1").expect("static canonical id"),
        ActionWindowRecord::SCHEMA_VERSION,
        CanonicalId::new("action_window_decode.v1").expect("static canonical id"),
        CanonicalId::new("action_window_validate.v1").expect("static canonical id"),
        CanonicalId::new("action_window_serialize.v1").expect("static canonical id"),
    )
}

/// Generic typed-family validator for a module-owned action-window guard.
/// A witness remains current only while the exact window generation is open;
/// closing, cancelling, consuming, deleting, or replacing the generation
/// invalidates every action admitted under it.
#[derive(Clone)]
pub struct ActionWindowGuardValidator {
    windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
}

impl std::fmt::Debug for ActionWindowGuardValidator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionWindowGuardValidator")
            .field("owner", self.windows.owner())
            .field("family", self.windows.descriptor().id())
            .finish()
    }
}

impl ActionWindowGuardValidator {
    #[must_use]
    pub const fn new(windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>) -> Self {
        Self { windows }
    }
}

impl AuthorityGuardValidator for ActionWindowGuardValidator {
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Window {
            owner,
            id,
            generation,
        } = guard
        else {
            return Ok(false);
        };
        if owner != self.windows.owner() {
            return Ok(false);
        }
        let current = context.action_state(&self.windows, id).map_err(|_| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "action-window authority state is unavailable",
            )
            .expect("static action-window read error is bounded")
        })?;
        Ok(current.is_some_and(|record| {
            record.phase() == ActionWindowPhase::Open
                && record.window().id() == *id
                && record.window().generation() == *generation
        }))
    }
}

/// Whether an eligible action resolves immediately or joins a frozen set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    Immediate,
    CollectSimultaneous,
}

/// Provider policy when boot discovers a still-open window.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRestartPolicy {
    Resume,
    CloseIfElapsed,
    CancelIfElapsed,
}

/// Provider policy when the closing deadline is reached normally.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowTimeoutPolicy {
    Close,
    Cancel,
}

/// Whether a window is being checked during normal running or boot recovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowDriveCause {
    Live,
    Restart,
}

/// Pure instruction for a coordinator that drives one durable window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowDriveDecision {
    /// The open window has not reached its exact closing stamp.
    Waiting { closes: ClockStamp },
    /// Submit the provider-owned idempotent close action.
    Close,
    /// Submit the provider-owned idempotent cancellation action.
    Cancel,
    /// Submit resolution for the already durably frozen intent set.
    Resolve,
    /// No work remains for this terminal record.
    Terminal(ActionWindowPhase),
}

/// Immutable definition of one exact action-window generation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionWindow {
    id: ActionWindowId,
    clock: ClockRef,
    opens: ClockStamp,
    closes: ClockStamp,
    generation: u64,
    mode: WindowMode,
    capacity: u32,
    restart: WindowRestartPolicy,
    timeout: WindowTimeoutPolicy,
    max_catch_up: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolution_secret_version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionWindowWire {
    id: ActionWindowId,
    clock: ClockRef,
    opens: ClockStamp,
    closes: ClockStamp,
    generation: u64,
    mode: WindowMode,
    capacity: u32,
    restart: WindowRestartPolicy,
    timeout: WindowTimeoutPolicy,
    max_catch_up: u32,
    #[serde(default)]
    resolution_secret_version: Option<u32>,
}

impl<'de> Deserialize<'de> for ActionWindow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ActionWindowWire::deserialize(deserializer)?;
        Self::new(ActionWindowDraft {
            id: wire.id,
            clock: wire.clock,
            opens: wire.opens,
            closes: wire.closes,
            generation: wire.generation,
            mode: wire.mode,
            capacity: wire.capacity,
            restart: wire.restart,
            timeout: wire.timeout,
            max_catch_up: wire.max_catch_up,
            resolution_secret_version: wire.resolution_secret_version,
        })
        .map_err(serde::de::Error::custom)
    }
}

/// Construction inputs for an [`ActionWindow`].
#[derive(Clone, Debug)]
pub struct ActionWindowDraft {
    pub id: ActionWindowId,
    pub clock: ClockRef,
    pub opens: ClockStamp,
    pub closes: ClockStamp,
    pub generation: u64,
    pub mode: WindowMode,
    pub capacity: u32,
    pub restart: WindowRestartPolicy,
    pub timeout: WindowTimeoutPolicy,
    pub max_catch_up: u32,
    /// Root-secret version pinned when the window opens, when its eventual
    /// resolution uses authoritative randomness. `None` is valid for a
    /// wholly deterministic window.
    pub resolution_secret_version: Option<u32>,
}

impl ActionWindow {
    pub fn new(draft: ActionWindowDraft) -> Result<Self, ActionWindowError> {
        if draft.generation == 0 {
            return Err(ActionWindowError::ZeroGeneration);
        }
        if draft.capacity == 0 || draft.capacity > MAX_WINDOW_CAPACITY {
            return Err(ActionWindowError::InvalidCapacity {
                actual: draft.capacity,
                max: MAX_WINDOW_CAPACITY,
            });
        }
        if draft.max_catch_up == 0 || draft.max_catch_up > MAX_WINDOW_CATCH_UP {
            return Err(ActionWindowError::InvalidCatchUp {
                actual: draft.max_catch_up,
                max: MAX_WINDOW_CATCH_UP,
            });
        }
        if draft.resolution_secret_version == Some(0) {
            return Err(ActionWindowError::ZeroResolutionSecretVersion);
        }
        if draft.clock != draft.opens.clock || draft.clock != draft.closes.clock {
            return Err(ActionWindowError::ClockIdentityMismatch);
        }
        if draft.opens.checked_cmp(&draft.closes)? != Ordering::Less {
            return Err(ActionWindowError::EmptyRange);
        }
        if draft.mode == WindowMode::Immediate && draft.capacity != 1 {
            return Err(ActionWindowError::ImmediateCapacity);
        }
        Ok(Self {
            id: draft.id,
            clock: draft.clock,
            opens: draft.opens,
            closes: draft.closes,
            generation: draft.generation,
            mode: draft.mode,
            capacity: draft.capacity,
            restart: draft.restart,
            timeout: draft.timeout,
            max_catch_up: draft.max_catch_up,
            resolution_secret_version: draft.resolution_secret_version,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ActionWindowId {
        self.id
    }

    #[must_use]
    pub const fn clock(&self) -> &ClockRef {
        &self.clock
    }

    #[must_use]
    pub const fn opens(&self) -> &ClockStamp {
        &self.opens
    }

    #[must_use]
    pub const fn closes(&self) -> &ClockStamp {
        &self.closes
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn mode(&self) -> WindowMode {
        self.mode
    }

    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    #[must_use]
    pub const fn restart_policy(&self) -> WindowRestartPolicy {
        self.restart
    }

    #[must_use]
    pub const fn timeout_policy(&self) -> WindowTimeoutPolicy {
        self.timeout
    }

    #[must_use]
    pub const fn max_catch_up(&self) -> u32 {
        self.max_catch_up
    }

    /// Exact root-secret version reserved for this window's resolution.
    #[must_use]
    pub const fn resolution_secret_version(&self) -> Option<u32> {
        self.resolution_secret_version
    }

    /// Half-open eligibility check: `opens <= now < closes` on one exact
    /// clock generation.
    pub fn contains(&self, now: &ClockStamp) -> Result<bool, ClockMismatch> {
        Ok(self.opens.checked_cmp(now)? != Ordering::Greater
            && now.checked_cmp(&self.closes)? == Ordering::Less)
    }

    /// Bound provider-requested catch-up work by this window's declared cap.
    #[must_use]
    pub const fn bounded_catch_up(&self, requested: u32) -> u32 {
        if requested < self.max_catch_up {
            requested
        } else {
            self.max_catch_up
        }
    }
}

/// Ruleset-provided primary deterministic order. The engine appends stable
/// actor, subject, and request-hash tie breakers.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WindowOrderKey(Vec<i64>);

impl<'de> Deserialize<'de> for WindowOrderKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<i64>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl WindowOrderKey {
    pub fn new(parts: Vec<i64>) -> Result<Self, ActionWindowError> {
        if parts.len() > MAX_WINDOW_ORDER_KEY_PARTS {
            return Err(ActionWindowError::OrderKeyTooLong {
                actual: parts.len(),
                max: MAX_WINDOW_ORDER_KEY_PARTS,
            });
        }
        Ok(Self(parts))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[i64] {
        &self.0
    }
}

/// One actor/true-subject slot. Submitting again replaces this slot rather
/// than consuming additional capacity.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowIntentSlot {
    pub actor: crate::obj::ObjId,
    pub subject: SubjectKey,
}

/// Durable, guard-free intent accepted into a simultaneous window.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowIntent {
    window: ActionWindowId,
    window_generation: u64,
    truth_subject: SubjectKey,
    truth_targets: Vec<SubjectKey>,
    action: crate::mechanics::DefinitionRef,
    invocation: ActionInvocation,
    order: WindowOrderKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowIntentWire {
    window: ActionWindowId,
    window_generation: u64,
    truth_subject: SubjectKey,
    truth_targets: Vec<SubjectKey>,
    action: crate::mechanics::DefinitionRef,
    invocation: ActionInvocation,
    order: WindowOrderKey,
}

impl<'de> Deserialize<'de> for WindowIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WindowIntentWire::deserialize(deserializer)?;
        Self::new(
            wire.window,
            wire.window_generation,
            wire.truth_subject,
            wire.truth_targets,
            wire.action,
            wire.invocation,
            wire.order,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl WindowIntent {
    pub fn new(
        window: ActionWindowId,
        window_generation: u64,
        truth_subject: SubjectKey,
        truth_targets: Vec<SubjectKey>,
        action: crate::mechanics::DefinitionRef,
        invocation: ActionInvocation,
        order: WindowOrderKey,
    ) -> Result<Self, ActionWindowError> {
        if window_generation == 0 {
            return Err(ActionWindowError::ZeroGeneration);
        }
        if truth_targets.len() != invocation.targets().len() {
            return Err(ActionWindowError::TruthTargetCountMismatch {
                projected: invocation.targets().len(),
                truth: truth_targets.len(),
            });
        }
        Ok(Self {
            window,
            window_generation,
            truth_subject,
            truth_targets,
            action,
            invocation,
            order,
        })
    }

    #[must_use]
    pub const fn window(&self) -> ActionWindowId {
        self.window
    }

    #[must_use]
    pub const fn window_generation(&self) -> u64 {
        self.window_generation
    }

    #[must_use]
    pub const fn truth_subject(&self) -> &SubjectKey {
        &self.truth_subject
    }

    /// Ordered authoritative targets captured when the intent was admitted.
    /// These remain server-private even though the durable invocation retains
    /// only the caller-visible [`crate::action::PerceivedRef`] values.
    #[must_use]
    pub fn truth_targets(&self) -> &[SubjectKey] {
        &self.truth_targets
    }

    /// Exact action definition admitted into this window slot.
    #[must_use]
    pub const fn action(&self) -> &crate::mechanics::DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn invocation(&self) -> &ActionInvocation {
        &self.invocation
    }

    #[must_use]
    pub const fn order(&self) -> &WindowOrderKey {
        &self.order
    }

    #[must_use]
    pub fn slot(&self) -> WindowIntentSlot {
        WindowIntentSlot {
            actor: self.invocation.actor(),
            subject: self.truth_subject.clone(),
        }
    }
}

/// Durable lifecycle phase. Closed and consumed records retain the exact
/// frozen intent set for restart and audit.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionWindowPhase {
    Open,
    Closed,
    Consumed { commit: CommitToken },
    Cancelled,
}

/// Stable result of one idempotent lifecycle mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowMutationResult {
    Accepted { slot: WindowIntentSlot },
    Replaced { slot: WindowIntentSlot },
    IntentCancelled { slot: WindowIntentSlot },
    Closed,
    Consumed { commit: CommitToken },
    WindowCancelled,
}

/// Retained idempotency receipt. `binding` covers the complete operation
/// payload; reusing an operation ID with another payload is an integrity
/// conflict rather than a second mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowMutationReceipt {
    operation: RequestUid,
    binding: CanonicalHash,
    result: WindowMutationResult,
    revision: u64,
    accepted_at: ClockStamp,
}

impl WindowMutationReceipt {
    #[must_use]
    pub const fn operation(&self) -> RequestUid {
        self.operation
    }

    #[must_use]
    pub const fn binding(&self) -> CanonicalHash {
        self.binding
    }

    #[must_use]
    pub const fn result(&self) -> &WindowMutationResult {
        &self.result
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn accepted_at(&self) -> &ClockStamp {
        &self.accepted_at
    }
}

/// Canonical state persisted as one authoritative post-image.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionWindowRecord {
    schema_version: u16,
    window: ActionWindow,
    phase: ActionWindowPhase,
    revision: u64,
    intents: Vec<WindowIntent>,
    receipts: Vec<WindowMutationReceipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionWindowRecordWire {
    schema_version: u16,
    window: ActionWindow,
    phase: ActionWindowPhase,
    revision: u64,
    intents: Vec<WindowIntent>,
    receipts: Vec<WindowMutationReceipt>,
}

impl<'de> Deserialize<'de> for ActionWindowRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ActionWindowRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.schema_version,
            wire.window,
            wire.phase,
            wire.revision,
            wire.intents,
            wire.receipts,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ActionWindowRecord {
    pub const SCHEMA_VERSION: u16 = 3;

    /// Start an empty open record at revision one.
    #[must_use]
    pub fn open(window: ActionWindow) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            window,
            phase: ActionWindowPhase::Open,
            revision: 1,
            intents: Vec::new(),
            receipts: Vec::new(),
        }
    }

    fn from_parts(
        schema_version: u16,
        window: ActionWindow,
        phase: ActionWindowPhase,
        revision: u64,
        intents: Vec<WindowIntent>,
        receipts: Vec<WindowMutationReceipt>,
    ) -> Result<Self, ActionWindowError> {
        if schema_version != Self::SCHEMA_VERSION {
            return Err(ActionWindowError::UnsupportedSchema {
                actual: schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if intents.len() > usize::try_from(window.capacity).unwrap_or(usize::MAX) {
            return Err(ActionWindowError::CapacityReached {
                capacity: window.capacity,
            });
        }
        if receipts.len() > MAX_WINDOW_OPERATION_RECEIPTS {
            return Err(ActionWindowError::ReceiptLimit);
        }
        let expected_revision = u64::try_from(receipts.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(ActionWindowError::RevisionExhausted)?;
        if revision != expected_revision {
            return Err(ActionWindowError::InvalidStoredRevision {
                actual: revision,
                expected: expected_revision,
            });
        }
        let mut slots = BTreeSet::new();
        let mut previous_slot = None;
        for intent in &intents {
            if intent.window != window.id || intent.window_generation != window.generation {
                return Err(ActionWindowError::IntentWindowMismatch);
            }
            let slot = intent.slot();
            if !slots.insert(slot.clone()) {
                return Err(ActionWindowError::DuplicateSlot);
            }
            if previous_slot
                .as_ref()
                .is_some_and(|previous| previous >= &slot)
            {
                return Err(ActionWindowError::NonCanonicalIntentOrder);
            }
            previous_slot = Some(slot);
        }
        let operations: BTreeSet<_> = receipts.iter().map(|receipt| receipt.operation).collect();
        if operations.len() != receipts.len() {
            return Err(ActionWindowError::DuplicateOperation);
        }
        for (index, receipt) in receipts.iter().enumerate() {
            let expected = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(2))
                .ok_or(ActionWindowError::RevisionExhausted)?;
            if receipt.revision != expected {
                return Err(ActionWindowError::InvalidReceiptRevision {
                    actual: receipt.revision,
                    expected,
                });
            }
            if receipt.accepted_at.clock != window.clock {
                return Err(ActionWindowError::ReceiptClockMismatch);
            }
        }
        validate_phase_history(&window, phase, &receipts)?;
        Ok(Self {
            schema_version,
            window,
            phase,
            revision,
            intents,
            receipts,
        })
    }

    /// Re-run every durable structural and lifecycle invariant.
    pub fn validate(&self) -> Result<(), ActionWindowError> {
        Self::from_parts(
            self.schema_version,
            self.window.clone(),
            self.phase,
            self.revision,
            self.intents.clone(),
            self.receipts.clone(),
        )
        .map(|_| ())
    }

    #[must_use]
    pub const fn window(&self) -> &ActionWindow {
        &self.window
    }

    #[must_use]
    pub const fn phase(&self) -> ActionWindowPhase {
        self.phase
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn intents(&self) -> &[WindowIntent] {
        &self.intents
    }

    #[must_use]
    pub fn receipts(&self) -> &[WindowMutationReceipt] {
        &self.receipts
    }

    /// Decide the next coordinator action without mutating the window.
    ///
    /// `Resume` means "apply ordinary timeout policy after restart", not
    /// "reopen an elapsed collection interval".  The two explicit elapsed
    /// policies override that normal close/cancel choice. A durably closed
    /// window always resolves without consulting a newer clock generation;
    /// its close receipt already proves that the old deadline was crossed.
    pub fn drive_decision(
        &self,
        now: &ClockStamp,
        cause: WindowDriveCause,
    ) -> Result<WindowDriveDecision, ClockMismatch> {
        match self.phase {
            ActionWindowPhase::Closed => return Ok(WindowDriveDecision::Resolve),
            ActionWindowPhase::Consumed { .. } | ActionWindowPhase::Cancelled => {
                return Ok(WindowDriveDecision::Terminal(self.phase));
            }
            ActionWindowPhase::Open => {}
        }
        if now.checked_cmp(&self.window.closes)? == Ordering::Less {
            return Ok(WindowDriveDecision::Waiting {
                closes: self.window.closes.clone(),
            });
        }
        if cause == WindowDriveCause::Restart {
            match self.window.restart {
                WindowRestartPolicy::CloseIfElapsed => return Ok(WindowDriveDecision::Close),
                WindowRestartPolicy::CancelIfElapsed => return Ok(WindowDriveDecision::Cancel),
                WindowRestartPolicy::Resume => {}
            }
        }
        Ok(match self.window.timeout {
            WindowTimeoutPolicy::Close => WindowDriveDecision::Close,
            WindowTimeoutPolicy::Cancel => WindowDriveDecision::Cancel,
        })
    }

    /// Resolve the exact frozen set in ruleset order followed by stable
    /// engine tie breakers. Only a durably closed record exposes this set.
    pub fn resolution_order(&self) -> Result<Vec<&WindowIntent>, ActionWindowError> {
        if self.phase != ActionWindowPhase::Closed {
            return Err(ActionWindowError::NotClosed);
        }
        let mut ordered: Vec<_> = self.intents.iter().collect();
        ordered.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.invocation.actor().cmp(&right.invocation.actor()))
                .then_with(|| left.truth_subject.cmp(&right.truth_subject))
                .then_with(|| {
                    left.invocation
                        .request_hash()
                        .cmp(&right.invocation.request_hash())
                })
        });
        Ok(ordered)
    }

    /// Idempotently accept or replace one actor/subject slot.
    pub fn checked_submit(
        &self,
        expected_revision: u64,
        now: &ClockStamp,
        intent: WindowIntent,
    ) -> Result<WindowTransition, ActionWindowError> {
        if intent.window != self.window.id || intent.window_generation != self.window.generation {
            return Err(ActionWindowError::IntentWindowMismatch);
        }
        let operation = intent.invocation.request();
        let binding = operation_binding("submit", &json!({ "intent": intent }))?;
        if let Some(replay) = self.replay(operation, binding)? {
            return Ok(replay);
        }
        self.require_open(expected_revision)?;
        if self.window.mode != WindowMode::CollectSimultaneous {
            return Err(ActionWindowError::NotCollecting);
        }
        if !self.window.contains(now)? {
            return Err(ActionWindowError::OutsideWindow);
        }
        self.require_input_receipt_capacity()?;

        let slot = intent.slot();
        let mut intents = self.intents.clone();
        let result = match intents.binary_search_by(|candidate| candidate.slot().cmp(&slot)) {
            Ok(index) => {
                intents[index] = intent;
                WindowMutationResult::Replaced { slot }
            }
            Err(index) => {
                if intents.len() >= usize::try_from(self.window.capacity).unwrap_or(usize::MAX) {
                    return Err(ActionWindowError::CapacityReached {
                        capacity: self.window.capacity,
                    });
                }
                intents.insert(index, intent);
                WindowMutationResult::Accepted { slot }
            }
        };
        self.transition(operation, binding, result, self.phase, intents, now.clone())
    }

    /// Idempotently cancel one accepted actor/subject slot.
    pub fn checked_cancel_intent(
        &self,
        expected_revision: u64,
        operation: RequestUid,
        now: &ClockStamp,
        slot: WindowIntentSlot,
    ) -> Result<WindowTransition, ActionWindowError> {
        let binding = operation_binding("cancel_intent", &json!({ "slot": slot }))?;
        if let Some(replay) = self.replay(operation, binding)? {
            return Ok(replay);
        }
        self.require_open(expected_revision)?;
        if self.window.mode != WindowMode::CollectSimultaneous {
            return Err(ActionWindowError::NotCollecting);
        }
        if !self.window.contains(now)? {
            return Err(ActionWindowError::OutsideWindow);
        }
        self.require_input_receipt_capacity()?;
        let mut intents = self.intents.clone();
        let index = intents
            .binary_search_by(|candidate| candidate.slot().cmp(&slot))
            .map_err(|_| ActionWindowError::IntentMissing)?;
        intents.remove(index);
        self.transition(
            operation,
            binding,
            WindowMutationResult::IntentCancelled { slot },
            self.phase,
            intents,
            now.clone(),
        )
    }

    /// Idempotently freeze the exact due intent set. The close deadline is
    /// inclusive: the collection interval itself remains half-open.
    pub fn checked_close_due(
        &self,
        expected_revision: u64,
        operation: RequestUid,
        now: &ClockStamp,
    ) -> Result<WindowTransition, ActionWindowError> {
        let binding = operation_binding("close", &json!({ "window": self.window.id }))?;
        if let Some(replay) = self.replay(operation, binding)? {
            return Ok(replay);
        }
        self.require_open(expected_revision)?;
        if now.checked_cmp(&self.window.closes)? == Ordering::Less {
            return Err(ActionWindowError::NotDue);
        }
        self.transition(
            operation,
            binding,
            WindowMutationResult::Closed,
            ActionWindowPhase::Closed,
            self.intents.clone(),
            now.clone(),
        )
    }

    /// Idempotently cancel a due window while retaining its submitted set for
    /// audit. This is the explicit timeout/restart cancellation path.
    pub fn checked_cancel_due(
        &self,
        expected_revision: u64,
        operation: RequestUid,
        now: &ClockStamp,
    ) -> Result<WindowTransition, ActionWindowError> {
        let binding = operation_binding("cancel_window", &json!({ "window": self.window.id }))?;
        if let Some(replay) = self.replay(operation, binding)? {
            return Ok(replay);
        }
        self.require_open(expected_revision)?;
        if now.checked_cmp(&self.window.closes)? == Ordering::Less {
            return Err(ActionWindowError::NotDue);
        }
        self.transition(
            operation,
            binding,
            WindowMutationResult::WindowCancelled,
            ActionWindowPhase::Cancelled,
            self.intents.clone(),
            now.clone(),
        )
    }

    /// Mark a durably closed set consumed by the same authoritative bundle
    /// that commits its outcomes.
    pub fn checked_consume(
        &self,
        expected_revision: u64,
        operation: RequestUid,
        commit: CommitToken,
        now: &ClockStamp,
    ) -> Result<WindowTransition, ActionWindowError> {
        let binding = operation_binding("consume", &json!({ "commit": commit }))?;
        if let Some(replay) = self.replay(operation, binding)? {
            return Ok(replay);
        }
        self.require_revision(expected_revision)?;
        if self.phase != ActionWindowPhase::Closed {
            return Err(ActionWindowError::NotClosed);
        }
        if now.checked_cmp(&self.window.closes)? == Ordering::Less {
            return Err(ActionWindowError::InvalidReceiptTime);
        }
        self.transition(
            operation,
            binding,
            WindowMutationResult::Consumed { commit },
            ActionWindowPhase::Consumed { commit },
            self.intents.clone(),
            now.clone(),
        )
    }

    fn replay(
        &self,
        operation: RequestUid,
        binding: CanonicalHash,
    ) -> Result<Option<WindowTransition>, ActionWindowError> {
        let Some(receipt) = self
            .receipts
            .iter()
            .find(|receipt| receipt.operation == operation)
        else {
            return Ok(None);
        };
        if receipt.binding != binding {
            return Err(ActionWindowError::OperationConflict { operation });
        }
        Ok(Some(WindowTransition {
            record: self.clone(),
            receipt: receipt.clone(),
            replayed: true,
        }))
    }

    fn require_open(&self, expected_revision: u64) -> Result<(), ActionWindowError> {
        self.require_revision(expected_revision)?;
        if self.phase != ActionWindowPhase::Open {
            return Err(ActionWindowError::NotOpen { phase: self.phase });
        }
        Ok(())
    }

    fn require_revision(&self, expected_revision: u64) -> Result<(), ActionWindowError> {
        if self.revision != expected_revision {
            return Err(ActionWindowError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn require_input_receipt_capacity(&self) -> Result<(), ActionWindowError> {
        if self.receipts.len() >= MAX_WINDOW_INPUT_OPERATIONS {
            return Err(ActionWindowError::InputOperationLimit);
        }
        Ok(())
    }

    fn transition(
        &self,
        operation: RequestUid,
        binding: CanonicalHash,
        result: WindowMutationResult,
        phase: ActionWindowPhase,
        intents: Vec<WindowIntent>,
        accepted_at: ClockStamp,
    ) -> Result<WindowTransition, ActionWindowError> {
        if self.receipts.len() >= MAX_WINDOW_OPERATION_RECEIPTS {
            return Err(ActionWindowError::ReceiptLimit);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(ActionWindowError::RevisionExhausted)?;
        let receipt = WindowMutationReceipt {
            operation,
            binding,
            result,
            revision,
            accepted_at,
        };
        let mut receipts = self.receipts.clone();
        receipts.push(receipt.clone());
        Ok(WindowTransition {
            record: Self {
                schema_version: Self::SCHEMA_VERSION,
                window: self.window.clone(),
                phase,
                revision,
                intents,
                receipts,
            },
            receipt,
            replayed: false,
        })
    }
}

fn validate_phase_history(
    window: &ActionWindow,
    stored_phase: ActionWindowPhase,
    receipts: &[WindowMutationReceipt],
) -> Result<(), ActionWindowError> {
    let mut phase = ActionWindowPhase::Open;
    for receipt in receipts {
        let collection_mutation = matches!(
            receipt.result,
            WindowMutationResult::Accepted { .. }
                | WindowMutationResult::Replaced { .. }
                | WindowMutationResult::IntentCancelled { .. }
        );
        let valid_time = if collection_mutation {
            window.contains(&receipt.accepted_at)?
        } else {
            receipt.accepted_at.checked_cmp(&window.closes)? != Ordering::Less
        };
        if !valid_time {
            return Err(ActionWindowError::InvalidReceiptTime);
        }
        phase = match (&phase, &receipt.result) {
            (
                ActionWindowPhase::Open,
                WindowMutationResult::Accepted { .. }
                | WindowMutationResult::Replaced { .. }
                | WindowMutationResult::IntentCancelled { .. },
            ) => ActionWindowPhase::Open,
            (ActionWindowPhase::Open, WindowMutationResult::Closed) => ActionWindowPhase::Closed,
            (ActionWindowPhase::Open, WindowMutationResult::WindowCancelled) => {
                ActionWindowPhase::Cancelled
            }
            (ActionWindowPhase::Closed, WindowMutationResult::Consumed { commit }) => {
                ActionWindowPhase::Consumed { commit: *commit }
            }
            _ => return Err(ActionWindowError::InvalidPhaseHistory),
        };
    }
    if phase != stored_phase {
        return Err(ActionWindowError::InvalidPhaseHistory);
    }
    Ok(())
}

fn operation_binding(
    kind: &str,
    payload: &serde_json::Value,
) -> Result<CanonicalHash, ActionWindowError> {
    CanonicalHash::for_canonical_value(
        "action_window_operation.v1",
        1,
        &json!({ "kind": kind, "payload": payload }),
        crate::action::MAX_ACTION_PARAMETER_BYTES + 64 * 1024,
    )
    .map_err(ActionWindowError::Hash)
}

/// Pure transition result. The caller persists `record` as the exact
/// post-image before acknowledging `receipt`.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowTransition {
    record: ActionWindowRecord,
    receipt: WindowMutationReceipt,
    replayed: bool,
}

impl WindowTransition {
    #[must_use]
    pub const fn record(&self) -> &ActionWindowRecord {
        &self.record
    }

    #[must_use]
    pub const fn receipt(&self) -> &WindowMutationReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }

    #[must_use]
    pub fn into_record(self) -> ActionWindowRecord {
        self.record
    }
}

/// Invalid window definition, stored record, or lifecycle transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionWindowError {
    #[error("action-window generation zero is reserved")]
    ZeroGeneration,
    #[error("action-window capacity {actual} must be in 1..={max}")]
    InvalidCapacity { actual: u32, max: u32 },
    #[error("immediate action windows have capacity one")]
    ImmediateCapacity,
    #[error("action-window catch-up {actual} must be in 1..={max}")]
    InvalidCatchUp { actual: u32, max: u32 },
    #[error("resolution secret version zero is reserved")]
    ZeroResolutionSecretVersion,
    #[error("action-window boundary clock identities do not match")]
    ClockIdentityMismatch,
    #[error("action-window close must be strictly after open")]
    EmptyRange,
    #[error("action-window stamps are incomparable: {0}")]
    Clock(#[from] ClockMismatch),
    #[error("action-window order key has {actual} parts; maximum is {max}")]
    OrderKeyTooLong { actual: usize, max: usize },
    #[error("unsupported action-window record schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("action-window capacity {capacity} is full")]
    CapacityReached { capacity: u32 },
    #[error("action-window intent names another window or generation")]
    IntentWindowMismatch,
    #[error(
        "action-window intent has {projected} projected targets but {truth} authoritative targets"
    )]
    TruthTargetCountMismatch { projected: usize, truth: usize },
    #[error("action-window record contains a duplicate actor/subject slot")]
    DuplicateSlot,
    #[error("action-window intents are not in canonical slot order")]
    NonCanonicalIntentOrder,
    #[error("action-window record contains duplicate operation identities")]
    DuplicateOperation,
    #[error("action-window stored revision {actual} should be {expected}")]
    InvalidStoredRevision { actual: u64, expected: u64 },
    #[error("action-window receipt revision {actual} should be {expected}")]
    InvalidReceiptRevision { actual: u64, expected: u64 },
    #[error("action-window receipt acceptance stamp names another clock")]
    ReceiptClockMismatch,
    #[error("action-window receipt acceptance stamp is outside its lifecycle interval")]
    InvalidReceiptTime,
    #[error("action-window receipt lifecycle does not produce the stored phase")]
    InvalidPhaseHistory,
    #[error("action-window revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("action window is not open: {phase:?}")]
    NotOpen { phase: ActionWindowPhase },
    #[error("action window is not collecting simultaneous intents")]
    NotCollecting,
    #[error("action arrived outside the half-open window")]
    OutsideWindow,
    #[error("action-window intent slot is not present")]
    IntentMissing,
    #[error("action window has not reached its close deadline")]
    NotDue,
    #[error("action window is not durably closed")]
    NotClosed,
    #[error("action-window input operation history is full")]
    InputOperationLimit,
    #[error("action-window operation receipt history is full")]
    ReceiptLimit,
    #[error("action-window operation {operation} was reused with different input")]
    OperationConflict { operation: RequestUid },
    #[error("action-window revision is exhausted")]
    RevisionExhausted,
    #[error("cannot hash action-window operation: {0}")]
    Hash(#[from] CanonicalHashError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{
        ActionInvocationDraft, ActionOfferId, ContactId, IdempotencyKey, IdempotencyScope,
        PerceivedRef, PrincipalId, ScopedIdempotencyKey,
    };
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };

    fn action_definition() -> crate::mechanics::DefinitionRef {
        let mut definitions = DefinitionRegistryBuilder::new();
        definitions
            .register(
                OwnerId::new("toy").unwrap(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("window_action").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap()
    }

    fn turn(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: 9,
                generation: 3,
            },
            at,
        }
    }

    fn window(capacity: u32) -> ActionWindow {
        window_with_policy(
            capacity,
            WindowRestartPolicy::CloseIfElapsed,
            WindowTimeoutPolicy::Close,
        )
    }

    fn window_with_policy(
        capacity: u32,
        restart: WindowRestartPolicy,
        timeout: WindowTimeoutPolicy,
    ) -> ActionWindow {
        let owner = OwnerId::new("toy").unwrap();
        let local = CanonicalId::new("round_7").unwrap();
        ActionWindow::new(ActionWindowDraft {
            id: ActionWindowId::derive(&owner, &local).unwrap(),
            clock: turn(0).clock,
            opens: turn(10),
            closes: turn(20),
            generation: 7,
            mode: WindowMode::CollectSimultaneous,
            capacity,
            restart,
            timeout,
            max_catch_up: 3,
            resolution_secret_version: Some(1),
        })
        .unwrap()
    }

    fn invocation(actor: u32, request_byte: u8, parameter: i64) -> ActionInvocation {
        let request = RequestUid::from_bytes([request_byte; 16]);
        let principal = PrincipalId::Object {
            id: crate::obj::ObjId(actor),
        };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: crate::obj::ObjId(actor),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([0x44; 32])),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({ "choice": parameter }),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("toy").unwrap(),
                    namespace: CanonicalId::new("window_submit").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }

    fn intent(window: &ActionWindow, actor: u32, request_byte: u8, order: i64) -> WindowIntent {
        WindowIntent::new(
            window.id(),
            window.generation(),
            SubjectKey::Object {
                id: crate::obj::ObjId(actor),
            },
            Vec::new(),
            action_definition(),
            invocation(actor, request_byte, order),
            WindowOrderKey::new(vec![order]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn boundaries_are_half_open_and_generation_checked() {
        let window = window(2);
        assert!(!window.contains(&turn(9)).unwrap());
        assert!(window.contains(&turn(10)).unwrap());
        assert!(window.contains(&turn(19)).unwrap());
        assert!(!window.contains(&turn(20)).unwrap());
        assert!(
            window
                .contains(&ClockStamp {
                    clock: ClockRef::SceneTurn {
                        scene: 9,
                        generation: 4,
                    },
                    at: 10,
                })
                .is_err()
        );
        assert_eq!(window.bounded_catch_up(99), 3);
    }

    #[test]
    fn intent_retains_private_truth_targets_and_rejects_cardinality_drift() {
        let definition = window(2);
        let request = RequestUid::from_bytes([0x31; 16]);
        let principal = PrincipalId::Object {
            id: crate::obj::ObjId(7),
        };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: crate::obj::ObjId(7),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([0x32; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![PerceivedRef::Contact {
                provider: OwnerId::new("sensors").unwrap(),
                id: ContactId::from_bytes([0x33; 16]),
                generation: 4,
            }],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("toy").unwrap(),
                    namespace: CanonicalId::new("window_submit").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap();
        let truth_target = SubjectKey::Object {
            id: crate::obj::ObjId(99),
        };
        let intent = WindowIntent::new(
            definition.id(),
            definition.generation(),
            SubjectKey::Object {
                id: crate::obj::ObjId(7),
            },
            vec![truth_target.clone()],
            action_definition(),
            invocation.clone(),
            WindowOrderKey::new(vec![1]).unwrap(),
        )
        .unwrap();
        assert_eq!(intent.truth_targets(), &[truth_target]);
        assert_eq!(
            serde_json::from_value::<WindowIntent>(serde_json::to_value(&intent).unwrap()).unwrap(),
            intent
        );
        assert_eq!(
            WindowIntent::new(
                definition.id(),
                definition.generation(),
                SubjectKey::Object {
                    id: crate::obj::ObjId(7),
                },
                Vec::new(),
                action_definition(),
                invocation,
                WindowOrderKey::new(vec![1]).unwrap(),
            )
            .unwrap_err(),
            ActionWindowError::TruthTargetCountMismatch {
                projected: 1,
                truth: 0,
            }
        );
    }

    #[test]
    fn drive_policy_is_explicit_at_live_and_restart_boundaries() {
        let resume_cancel = ActionWindowRecord::open(window_with_policy(
            2,
            WindowRestartPolicy::Resume,
            WindowTimeoutPolicy::Cancel,
        ));
        assert_eq!(
            resume_cancel
                .drive_decision(&turn(19), WindowDriveCause::Restart)
                .unwrap(),
            WindowDriveDecision::Waiting { closes: turn(20) }
        );
        assert_eq!(
            resume_cancel
                .drive_decision(&turn(20), WindowDriveCause::Live)
                .unwrap(),
            WindowDriveDecision::Cancel
        );
        assert_eq!(
            resume_cancel
                .drive_decision(&turn(20), WindowDriveCause::Restart)
                .unwrap(),
            WindowDriveDecision::Cancel
        );

        let force_close = ActionWindowRecord::open(window_with_policy(
            2,
            WindowRestartPolicy::CloseIfElapsed,
            WindowTimeoutPolicy::Cancel,
        ));
        assert_eq!(
            force_close
                .drive_decision(&turn(20), WindowDriveCause::Restart)
                .unwrap(),
            WindowDriveDecision::Close
        );

        let force_cancel = ActionWindowRecord::open(window_with_policy(
            2,
            WindowRestartPolicy::CancelIfElapsed,
            WindowTimeoutPolicy::Close,
        ));
        assert_eq!(
            force_cancel
                .drive_decision(&turn(20), WindowDriveCause::Restart)
                .unwrap(),
            WindowDriveDecision::Cancel
        );
    }

    #[test]
    fn durably_closed_window_resolves_without_re_reading_an_old_clock() {
        let record = ActionWindowRecord::open(window(2));
        let closed = record
            .checked_close_due(1, RequestUid::from_bytes([9; 16]), &turn(20))
            .unwrap()
            .into_record();

        assert_eq!(
            closed
                .drive_decision(
                    &ClockStamp {
                        clock: ClockRef::Utc,
                        at: 999,
                    },
                    WindowDriveCause::Restart,
                )
                .unwrap(),
            WindowDriveDecision::Resolve
        );
    }

    #[test]
    fn submit_replace_cancel_are_idempotent_and_conflict_checked() {
        let definition = window(2);
        let record = ActionWindowRecord::open(definition.clone());
        let first = record
            .checked_submit(record.revision(), &turn(10), intent(&definition, 2, 1, 8))
            .unwrap();
        assert_eq!(
            first.receipt().result(),
            &WindowMutationResult::Accepted {
                slot: WindowIntentSlot {
                    actor: crate::obj::ObjId(2),
                    subject: SubjectKey::Object {
                        id: crate::obj::ObjId(2)
                    },
                },
            }
        );
        let first_record = first.into_record();
        let replay = first_record
            .checked_submit(1, &turn(10), intent(&definition, 2, 1, 8))
            .unwrap();
        assert!(replay.replayed());
        assert_eq!(replay.record(), &first_record);
        assert!(matches!(
            first_record.checked_submit(1, &turn(10), intent(&definition, 2, 1, 9)),
            Err(ActionWindowError::OperationConflict { .. })
        ));

        let replacement = first_record
            .checked_submit(
                first_record.revision(),
                &turn(11),
                intent(&definition, 2, 2, 6),
            )
            .unwrap()
            .into_record();
        assert_eq!(replacement.intents().len(), 1);
        assert_eq!(replacement.intents()[0].order().as_slice(), &[6]);

        let slot = replacement.intents()[0].slot();
        let cancelled = replacement
            .checked_cancel_intent(
                replacement.revision(),
                RequestUid::from_bytes([3; 16]),
                &turn(12),
                slot,
            )
            .unwrap()
            .into_record();
        assert!(cancelled.intents().is_empty());
    }

    #[test]
    fn closed_set_survives_roundtrip_and_has_stable_resolution_order() {
        let definition = window(3);
        let mut record = ActionWindowRecord::open(definition.clone());
        for candidate in [
            intent(&definition, 5, 5, 10),
            intent(&definition, 3, 3, 10),
            intent(&definition, 4, 4, 1),
        ] {
            record = record
                .checked_submit(record.revision(), &turn(12), candidate)
                .unwrap()
                .into_record();
        }
        record = record
            .checked_close_due(
                record.revision(),
                RequestUid::from_bytes([0x31; 16]),
                &turn(20),
            )
            .unwrap()
            .into_record();
        let close_replay = record
            .checked_close_due(1, RequestUid::from_bytes([0x31; 16]), &turn(25))
            .unwrap();
        assert!(close_replay.replayed());
        assert_eq!(close_replay.receipt().accepted_at(), &turn(20));
        let restored: ActionWindowRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(restored, record);
        let actors: Vec<_> = restored
            .resolution_order()
            .unwrap()
            .iter()
            .map(|intent| intent.invocation().actor().0)
            .collect();
        assert_eq!(actors, vec![4, 3, 5]);

        let consumed = restored
            .checked_consume(
                restored.revision(),
                RequestUid::from_bytes([0x32; 16]),
                CommitToken::from_bytes([0x77; 16]),
                &turn(21),
            )
            .unwrap()
            .into_record();
        assert_eq!(
            consumed.phase(),
            ActionWindowPhase::Consumed {
                commit: CommitToken::from_bytes([0x77; 16])
            }
        );
    }

    #[test]
    fn stored_record_rejects_revision_and_order_tampering() {
        let definition = window(2);
        let mut record = ActionWindowRecord::open(definition.clone());
        record = record
            .checked_submit(record.revision(), &turn(12), intent(&definition, 2, 1, 1))
            .unwrap()
            .into_record();
        record = record
            .checked_submit(record.revision(), &turn(12), intent(&definition, 1, 2, 2))
            .unwrap()
            .into_record();
        let mut value = serde_json::to_value(&record).unwrap();
        value["revision"] = json!(99);
        assert!(serde_json::from_value::<ActionWindowRecord>(value).is_err());

        let mut value = serde_json::to_value(&record).unwrap();
        value["intents"].as_array_mut().unwrap().reverse();
        assert!(serde_json::from_value::<ActionWindowRecord>(value).is_err());

        assert!(
            serde_json::from_value::<WindowOrderKey>(json!(vec![
                0;
                MAX_WINDOW_ORDER_KEY_PARTS + 1
            ]))
            .is_err()
        );
    }

    #[test]
    fn state_family_codec_has_canonical_key_and_strict_round_trip() {
        let definition = window(2);
        let record = ActionWindowRecord::open(definition.clone());
        let codec = ActionWindowStateCodec;
        assert_eq!(
            codec.encode_key(&definition.id()).unwrap(),
            definition.id().canonical_hash().to_hex()
        );
        let descriptor = action_window_state_family_descriptor(
            StateFamilyId::new("windows").expect("static family id"),
        )
        .unwrap();
        assert_eq!(
            descriptor.schema_version(),
            ActionWindowRecord::SCHEMA_VERSION
        );

        let encoded = codec.serialize_value(&record).unwrap();
        let decoded = codec.decode_value(&encoded).unwrap();
        codec.validate_value(&decoded).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.window().resolution_secret_version(), Some(1));

        let mut tampered = encoded.clone();
        tampered["revision"] = json!(99);
        assert!(codec.decode_value(&tampered).is_err());

        let mut zero_secret = encoded;
        zero_secret["window"]["resolution_secret_version"] = json!(0);
        assert!(codec.decode_value(&zero_secret).is_err());
    }
}
