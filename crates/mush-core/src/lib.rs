//! # mush-core
//!
//! The synchronous heart of rustmush: the in-RAM [`World`], the object
//! model (objects, attributes, flags, locks, zones), and the IC game clock.
//!
//! Design ground rules (see `docs/PLAN.md`):
//!
//! - **No I/O, no async, no locks.** `World` is plain data plus methods; at
//!   runtime a single task owns it outright, and in tests it is driven
//!   directly. Nothing in this crate's public API mentions tokio.
//! - **All mutation goes through `World` methods**, which maintain every
//!   inverse index (contents/exits, player names, zone membership), stamp
//!   `modified`, and record dirty ids for the persistence layer.
//! - **Ids are never recycled** ([`obj::ObjId`] is monotonic; destroyed
//!   objects leave tombstones), so a stale dbref can never alias a new
//!   object.
//!
//! On top of the foundation sit the command layer:
//!
//! - [`perm`] — centralized permission checks (`controls`, `can_examine`,
//!   power/role gating); handlers never inline flag math.
//! - [`output`] — [`Audience`]/[`Outbox`] plus view builders emitting
//!   `mush-proto` types **directly** (treaty ruling 1: no parallel event
//!   enum in core).
//! - [`matchit`] — the single classic-precedence name resolver
//!   (`me`/`here`/`#dbref`/`*player`/inventory/location/exits/prefix/
//!   `2nd sword`).
//! - [`cmd`] — [`CommandSpec`]/[`Registry`]/[`dispatch`], implementing the
//!   treaty ruling 5 pipeline (prefix chars → alias stage → HOME → exits →
//!   command table → Huh).
//! - [`commands`] — the core MUSH command set, registered via
//!   [`register_core_commands`].
//! - [`module`] — the merged module API (treaty ruling 4): [`GameModule`]
//!   and [`Registrar`] (commands, alias stages, marker flags, attr
//!   namespaces, ticks, hooks, observers, state slots, panel feeds, query
//!   handlers, job queues, script-api reservations, migrations), module
//!   persistence ops ([`ModuleOp`]), scenes ([`SceneId`], the tactics seam),
//!   and the input-context stack.
//!
//! And the async seam (still fully sync in here — the tokio loop lives in
//! `mushd`):
//!
//! - [`engine`] — [`Engine`]/[`Inbound`]/[`Effects`]/[`WorldCommand`]: the
//!   step function the world task drives, connection bookkeeping, and the
//!   typed re-entry path for async completions (treaty ruling 3).
//! - [`sched`] — the pure min-heap [`Scheduler`] behind `@wait` and module
//!   ticks.

pub mod action;
pub mod action_state;
pub mod authoritative;
pub mod attr;
pub mod behavior;
pub mod clock;
pub mod cmd;
pub mod commands;
pub mod engine;
pub mod flags;
pub mod ledger;
pub mod lock;
pub mod matchit;
pub mod mechanics;
pub mod module;
pub mod obj;
pub mod output;
pub mod perm;
pub mod sched;
pub mod world;

pub use action::{
    ActionAttemptMetadata, ActionAttemptMetadataError, ActionError, ActionErrorBuildError,
    ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionLifecycle, ActionMetadataError,
    ActionOfferId, ActionProvider, ActionProviderDispatchError, ActionProviderEntry,
    ActionProviderLookupError, ActionProviderPrepareError, ActionProviderRegistry,
    ActionProviderRegistryBuilder, ActionProviderRegistryError, ActionReceiptMetadata,
    ActionReceiptMetadataError, AuthorityGuard, AuthorityGuardKind, AuthorityGuardValidator,
    AuthorityGuardValidatorEntry, AuthorityGuardValidatorLookupError,
    AuthorityGuardValidatorRegistry, AuthorityGuardValidatorRegistryBuilder,
    AuthorityGuardValidatorRegistryError, CanonicalHash, CanonicalHashError, CascadeError,
    CausalMetadataError, CausalTriggerQueue, CommitToken, ContactId, FixedHexIdError,
    GuardRevalidation, GuardRevalidationError, GuardSet, GuardSetBuildError, IdempotencyKey,
    IdempotencyKeyText, IdempotencyKeyTextError, IdempotencyScope, IngressEnvelope, IngressPayload,
    IngressSurfaceVersion, MAX_ACTION_ERROR_BYTES, MAX_ACTION_PARAMETER_BYTES,
    MAX_ACTION_PROVIDERS, MAX_ACTION_PROVIDERS_PER_OWNER, MAX_ACTION_REVISIONS, MAX_ACTION_TARGETS,
    MAX_AUTHORITY_GUARDS, MAX_AUTHORITY_VALIDATORS, MAX_AUTHORITY_VALIDATORS_PER_OWNER,
    MAX_CASCADE_BYTES, MAX_CASCADE_OPERATIONS, MAX_CASCADE_TRIGGERS, MAX_GUARD_SET_BYTES,
    MAX_IDEMPOTENCY_KEY_BYTES, MAX_INGRESS_LINE_BYTES, MAX_INGRESS_PAYLOAD_BYTES,
    MAX_REVISION_VALIDATORS, MAX_REVISION_VALIDATORS_PER_OWNER, MAX_SEMANTIC_EVENTS,
    MAX_TRIGGER_DEPTH, PerceivedRef, PreparedCausalRef, PreparedEventId, PreparedTrigger,
    PreparedTriggerDraft, PrincipalId, ProviderRegistrationOrdinal, RequestUid, RevisionKey,
    RevisionKind, RevisionMismatch, RevisionSet, RevisionSetError, RevisionStamp,
    RevisionValidator, RevisionValidatorEntry, RevisionValidatorLookupError,
    RevisionValidatorRegistry, RevisionValidatorRegistryBuilder, RevisionValidatorRegistryError,
    ScopedIdempotencyKey, ServerGuardSetBuilder, SubjectKey, TriggerEnqueue, TriggerHandlerId,
    TriggerIdentity, TriggerWork,
};
pub use authoritative::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeBundleError, AuthoritativeKey,
    AuthoritativeKeyError, DurableOutboxIntent, MAX_AUTHORITATIVE_BUNDLE_BYTES,
    MAX_AUTHORITATIVE_DEFINITIONS, MAX_AUTHORITATIVE_REPLAY_WRITES,
    MAX_AUTHORITATIVE_WITNESSES, MAX_DURABLE_OUTBOX_INTENTS,
    MAX_DURABLE_OUTBOX_PAYLOAD_BYTES, ReplayWrite,
};
pub use action_state::{
    ACTION_STATE_PERSISTENCE_ROOT, ActionStateBootRecord, ActionStateError, ActionStateStore,
    ActionTxn, ActionTxnLimits, CanonicalStateBytes, MAX_ACTION_BUNDLE_BYTES,
    MAX_ACTION_MUTATION_OPERATIONS, MAX_ACTION_STATE_BOOT_BYTES, MAX_ACTION_STATE_BOOT_RECORDS,
    MAX_ACTION_STATE_RECORD_BYTES, MAX_STATE_FAMILIES, MAX_STATE_FAMILIES_PER_OWNER,
    MAX_STATE_KEY_BYTES, PreparedInstallation, PreparedRecord, PreparedTxn, StateFamilyDescriptor,
    StateFamilyDescriptorError, StateFamilyHandle, StateFamilyId, StateFamilyRegistry,
    StateFamilyRegistryBuilder, StateFamilyRegistryError, StateKey, StateKeyCodec,
    StateValueDecoder, StateValueSerializer, StateValueValidator, is_action_state_persistence_key,
};
pub use attr::{Attr, AttrFlags, AttrMap, AttrValue, UncasedName};
pub use behavior::{
    BINDING_SCHEMA_VERSION, BehaviorBinding, BehaviorCommand, BehaviorCommandHandler, BehaviorId,
    BehaviorIdError, BehaviorRegError, BehaviorResolution, BehaviorSpec, BehaviorTarget, ChannelId,
    CommandReach, DestroyGate, DestroyPolicy, DetachGate, MAX_CARRIED_SCAN,
    resolve_behavior_command,
};
pub use clock::{CivilDate, GameClock, Season, TimeOfDay};
pub use cmd::{
    AliasOutcome, AliasStage, ArgShape, CmdError, CommandFallback, CommandGate, CommandHandler,
    CommandSpec, Ctx, HelpTopic, LookupErr, ParsedArgs, Registry, RegistryError, SceneClass,
    SwitchSet, dispatch,
};
pub use commands::{
    event_subject, ignore_list, is_ignoring, narrative_actor, register_core_commands,
};
pub use engine::{
    ControlMsg, Deferred, DisconnectKind, Effect, Effects, Engine, Inbound, ModuleComposition,
    StagePreparedActionError, StagedAction, StagedActionAdmissionError, StepOutcome, WorldCommand,
    register_engine_commands,
};
pub use flags::{Flags, Powers, Role};
pub use ledger::{
    BreachKind, ContiguityBreach, Ledger, LedgerAppendError, LedgerApplyError, LedgerChunk,
    LedgerEntry, LedgerRestoreError, LedgerRestoreReport, MAX_LEDGER_LOG_ID_BYTES,
    MAX_RESTORE_BREACH_DETAILS, PreparedLedgerAppend, chunk_key, is_log_key, is_sealed_chunk,
    scan_contiguity,
};
pub use lock::{LockExpr, LockParseError, LockType, eval_lock, glob_match, parse_lock};
pub use matchit::{MatchErr, match_object};
pub use mechanics::{
    BridgeDescriptor, BridgeEntry, BridgeKey, BridgeLookupError, BridgeRegistry,
    BridgeRegistryBuilder, BridgeRegistryError, CanonicalEncodingError, CanonicalId,
    CanonicalIdError, DefinitionDescriptor, DefinitionDescriptorError, DefinitionDigest,
    DefinitionDigestError, DefinitionEntry, DefinitionId, DefinitionKey, DefinitionKind,
    DefinitionLocator, DefinitionLookupError, DefinitionMigrationDescriptor, DefinitionMigrationFn,
    DefinitionMigrationRegistry, DefinitionMigrationRegistryBuilder, DefinitionRef,
    DefinitionRegistry, DefinitionRegistryBuilder, DefinitionRegistryError,
    DefinitionRegistrySummary, DefinitionValidationContext, DefinitionValidator, IncidentId,
    IncidentState, InteractionRoute, MAX_BOOT_REPORT_DETAILS, MAX_MECHANICS_OWNERS,
    MAX_RECOVERY_SUBJECTS, MAX_RULES_VALUE_BYTES, MechanicalOwnerFreeze, MechanicalOwnerReadiness,
    MechanicalOwnerReadinessError, MechanicalState, MechanicsLimits, MigrationError, MigrationPlan,
    MigrationPlanSummary, MigrationStep, ModuleBootReport, OwnerId, RecoveryIncident,
    RecoveryIncidentError, RecoveryIncidentSummary, RecoveryReason, RecoveryScope,
    RecoverySeverity, RecoverySubject, RulesValue, RulesValueError, canonical_json_bytes,
    canonical_json_bytes_with_limit,
};
pub use module::{
    AsyncCompletion, ContextOutcome, EventHook, GameModule, HookPoint, InputContext, JobQueueSpec,
    MAX_MODULE_KEY_BYTES, MAX_MODULE_NAMESPACE_BYTES, Migration, ModError, ModuleBootData,
    ModuleCtx, ModuleOp, ModuleWrite, PanelFeedHandle, Perceive, QueryHandler, RecoveryReporter,
    Registrar, Scene, SceneId, StackedContext, TickCadence, TickHandler, WeatherQuery, WorldEvent,
    WorldEventKind, WorldObserver, WxSnapshot, can_perceive, is_concealed, perceives_via,
};
pub use obj::{GOD, LIMBO, Obj, ObjId, ObjKind};
pub use output::{
    Audience, ConnId, DeliveryClass, DeliveryOutcome, Outbox, name_span, obj_ref, room_view,
};
pub use sched::{
    MAX_PENDING_ENTRIES_PER_INSPECTION, MAX_SCHEDULED_FIRES_PER_STEP, PendingEntrySnapshot,
    ScheduledKind, Scheduler, TimerId,
};
pub use world::{CommandSubjectLink, MAX_CONTAINMENT_DEPTH, SessionStat, World, WorldError};
