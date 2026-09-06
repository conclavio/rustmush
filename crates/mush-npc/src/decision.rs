//! **The `npc` decision source** (H2a; `docs/design/11-h-implementation-plan.md`
//! §3.2) — the first production composition of the K7 delayed-decision
//! kernel, copying the harness module's registration by symbol under the
//! module's own owner.
//!
//! What it registers: the four kernel families (`npc_pending_decisions`,
//! `npc_decision_prompts`, `npc_decision_deadlines`,
//! `npc_decision_terminal_receipts`, on the kernel's own descriptors and
//! codecs) plus one owner family, `npc_turn_receipts` — one bounded record per
//! NPC, the durable "last decision"; three action definitions — `npc_decide`
//! (the capture action), `npc_lease` (the lease action the kernel records each
//! provider lease with) and `npc_idle` (the no-op every NPC always holds, the
//! deadline fallback); one action provider per definition; the decision
//! capture sink the driving loop arms; and the decision source itself with the
//! idle fallback.
//!
//! `npc_decide` and `npc_idle` are projected **only** for an `ObjKind::Npc`
//! actor that carries a valid agent profile, so a player never receives either
//! offer and cannot submit one; and every provider refuses an invocation whose
//! principal is not the acting object itself.
//!
//! No verb is added: a decision is requested through
//! [`NpcHostState::request_decision`](crate::runtime::NpcHostState::request_decision)
//! and issued by the agent tick — one per tick step, because a step that
//! captures twice captures nothing (the kernel's fan-out rule).

use mush_core::ActionPresentation;
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    CanonicalId, DecisionCaptureFamilies, DecisionCaptureSink, DecisionFallbackAdapter,
    DecisionFallbackSelection, DecisionPrompt, DecisionPromptStateCodec,
    DecisionTerminalReceiptStateCodec, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRef, DefinitionValidationContext, DurableScheduleStateCodec, Engine,
    FallbackPolicyRef, IdempotencyKeyText, LegalActionQuery, ModError, ObjId, ObjKind,
    PendingDecisionStateCodec, PrincipalId, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, Registrar, RevisionSet, RulesValue, StateFamilyDescriptor,
    StateFamilyHandle, StateFamilyId, decision_prompt_state_family_descriptor,
    decision_terminal_receipt_state_family_descriptor, durable_schedule_state_family_descriptor,
    pending_decision_state_family_descriptor,
};
use serde::{Deserialize, Serialize};

/// How long one provider lease on an NPC decision lasts, seconds (11-h §7;
/// well under the kernel's `MAX_DECISION_LEASE_DURATION`).
pub const NPC_DECISION_LEASE_SECS: i64 = 60;
/// The wake adapter identity the source registers (the durable outbox routes
/// the capture's wake through it).
pub const NPC_WAKE_ADAPTER: &str = "npc_wake";
/// The one owner-local controller key every NPC decision is filed under.
pub const NPC_CONTROLLER_KEY: &str = "npc";
/// The public-safe context schema an NPC decision carries.
pub const NPC_CONTEXT_SCHEMA: &str = "npc.turn.v1";
/// The `npc_idle` offer's verb — what the fallback looks for in a prompt.
pub const IDLE_VERB: &str = "idle";
/// The `npc_decide` offer's verb.
pub const DECIDE_VERB: &str = "decide";
/// How many decision requests may wait for the tick at once.
pub const MAX_DECISION_REQUESTS: usize = 64;

const DECIDE_ID: &str = "npc_decide";
const LEASE_ID: &str = "npc_lease";
const IDLE_ID: &str = "npc_idle";
const RECEIPTS_FAMILY: &str = "npc_turn_receipts";
const FALLBACK_IDENTITY: &str = "npc_idle_fallback.v1";
const MAX_REFUSAL_BYTES: usize = 200;

/// The composed handles the module publishes as state, for the tick (the
/// fallback definition), the indicator and the tests.
#[derive(Clone, Debug)]
pub struct NpcDecisionHandles {
    /// The four kernel families.
    pub families: DecisionCaptureFamilies,
    /// The capture sink the driving loop arms.
    pub sink: DecisionCaptureSink,
    /// `npc_decide`.
    pub decide: DefinitionRef,
    /// `npc_lease`.
    pub lease: DefinitionRef,
    /// `npc_idle`.
    pub idle: DefinitionRef,
    /// `npc_turn_receipts`.
    pub receipts: StateFamilyHandle<CanonicalId, TurnReceipt>,
}

/// One NPC's durable "last decision": what the last executed decision action
/// was. Bounded to one record per NPC, keyed by the NPC's id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurnReceipt {
    /// The NPC.
    pub npc: ObjId,
    /// The action's outcome word (`"idle"` for the no-op).
    pub outcome: String,
}

/// The key `npc`'s turn receipt lives under.
///
/// # Errors
/// Never in practice: the id renders as lowercase digits.
pub fn receipt_key(npc: ObjId) -> Result<CanonicalId, ModError> {
    CanonicalId::new(format!("npc_{}", npc.0)).map_err(|error| ModError::msg(error.to_string()))
}

/// A capture the agent tick will issue for an NPC (H2a's ingress; H2b's scene
/// observer queues one per turn). Everything in it is the module's selector —
/// never an offer, a witness or a revision (D-H-6).
#[derive(Clone, Debug, PartialEq)]
pub struct NpcDecisionRequest {
    /// The NPC the decision is for.
    pub npc: ObjId,
    /// The cause kind (`npc.scene_turn`, `npc.poke`, …).
    pub cause_kind: CanonicalId,
    /// The cause instance: identical instances coalesce into one decision.
    pub instance: IdempotencyKeyText,
    /// A replaceable key: a newer decision with the same key displaces the
    /// older pending one instead of adding a second.
    pub replaceable_key: Option<IdempotencyKeyText>,
    /// The public-safe context value (under `npc.turn.v1`).
    pub context: serde_json::Value,
    /// Seconds until the idle fallback fires.
    pub deadline_secs: i64,
    /// The chosen action's idempotency discriminator; unique per decision.
    pub key: IdempotencyKeyText,
}

fn canonical(name: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

fn family_id(name: &str) -> Result<StateFamilyId, ModError> {
    StateFamilyId::new(name).map_err(|error| ModError::msg(error.to_string()))
}

fn refuse(detail: impl Into<String>) -> ActionError {
    let mut detail: String = detail.into();
    if detail.len() > MAX_REFUSAL_BYTES {
        let mut cut = MAX_REFUSAL_BYTES;
        while !detail.is_char_boundary(cut) {
            cut -= 1;
        }
        detail.truncate(cut);
    }
    ActionError::new(ActionErrorCode::ProviderRejected, false, detail)
        .expect("a bounded npc decision refusal")
}

/// Register the source. Called from `NpcModule::register`.
pub(crate) fn register(reg: &mut Registrar<'_>, lease_secs: i64) -> Result<(), ModError> {
    let action_kind =
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?;
    reg.definition_validator(
        action_kind.clone(),
        |context: &DefinitionValidationContext<'_>| {
            context
                .definition()
                .payload()
                .is_object()
                .then_some(())
                .ok_or_else(|| "npc action payload must be an object".to_owned())
        },
    )?;
    let define = |reg: &mut Registrar<'_>, id: &str| -> Result<DefinitionRef, ModError> {
        reg.definition(
            DefinitionDescriptor::new(
                action_kind.clone(),
                DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
                1,
                serde_json::json!({}),
            )
            .map_err(|error| ModError::msg(error.to_string()))?
            .as_current(),
        )
    };
    let decide = define(reg, DECIDE_ID)?;
    let lease = define(reg, LEASE_ID)?;
    let idle = define(reg, IDLE_ID)?;

    let pending = reg.state_family(
        pending_decision_state_family_descriptor(family_id("npc_pending_decisions")?)
            .map_err(|error| ModError::msg(error.to_string()))?,
        PendingDecisionStateCodec,
        PendingDecisionStateCodec,
        PendingDecisionStateCodec,
        PendingDecisionStateCodec,
    )?;
    let prompts = reg.state_family(
        decision_prompt_state_family_descriptor(family_id("npc_decision_prompts")?)
            .map_err(|error| ModError::msg(error.to_string()))?,
        DecisionPromptStateCodec,
        DecisionPromptStateCodec,
        DecisionPromptStateCodec,
        DecisionPromptStateCodec,
    )?;
    let deadlines = reg.state_family(
        durable_schedule_state_family_descriptor(family_id("npc_decision_deadlines")?)
            .map_err(|error| ModError::msg(error.to_string()))?,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
    )?;
    let terminal_receipts = reg.state_family(
        decision_terminal_receipt_state_family_descriptor(family_id(
            "npc_decision_terminal_receipts",
        )?)
        .map_err(|error| ModError::msg(error.to_string()))?,
        DecisionTerminalReceiptStateCodec,
        DecisionTerminalReceiptStateCodec,
        DecisionTerminalReceiptStateCodec,
        DecisionTerminalReceiptStateCodec,
    )?;
    let receipts = reg.state_family(
        StateFamilyDescriptor::new(
            family_id(RECEIPTS_FAMILY)?,
            canonical("canonical_id.v1")?,
            canonical("npc_turn_receipt.v1")?,
            1,
            canonical("npc_turn_receipt_decode.v1")?,
            canonical("npc_turn_receipt_validate.v1")?,
            canonical("npc_turn_receipt_serialize.v1")?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &CanonicalId| Ok(key.to_string()),
        |value: &serde_json::Value| {
            serde_json::from_value::<TurnReceipt>(value.clone()).map_err(|error| error.to_string())
        },
        |value: &TurnReceipt| {
            if value.outcome.len() > 64 {
                return Err("turn receipt outcome is too long".to_owned());
            }
            Ok(())
        },
        |value: &TurnReceipt| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let families = DecisionCaptureFamilies::new(pending, prompts, deadlines, terminal_receipts)
        .map_err(|error| ModError::msg(error.to_string()))?;
    let sink = DecisionCaptureSink::new();

    reg.action_provider(
        decide.clone(),
        NpcDecideProvider {
            action: decide.clone(),
            families: families.clone(),
            sink: sink.clone(),
        },
    )?;
    reg.action_provider(
        lease.clone(),
        NpcLeaseProvider {
            action: lease.clone(),
        },
    )?;
    reg.action_provider(
        idle.clone(),
        NpcIdleProvider {
            action: idle.clone(),
            receipts: receipts.clone(),
        },
    )?;
    reg.decision_capture_sink(decide.clone(), sink.clone())?;
    reg.decision_source(
        lease.clone(),
        families.clone(),
        canonical(NPC_WAKE_ADAPTER)?,
        lease_secs,
        NpcIdleFallback {
            identity: canonical(FALLBACK_IDENTITY)?,
        },
    )?;
    reg.state(NpcDecisionHandles {
        families,
        sink,
        decide,
        lease,
        idle,
        receipts,
    });
    Ok(())
}

/// Whether `actor` is an NPC agent this source acts for: an `ObjKind::Npc`
/// carrying a valid profile.
fn is_agent(engine: &Engine, actor: ObjId) -> bool {
    engine
        .world
        .get(actor)
        .is_some_and(|object| object.kind == ObjKind::Npc)
        && crate::module::read_profile(&engine.world, actor).is_some()
}

/// The offer draft for one of the source's projected actions, or `None` for
/// an actor that is not an NPC agent.
fn agent_offer(
    engine: &Engine,
    query: &LegalActionQuery,
    action: &DefinitionRef,
    verb: &str,
    label: &str,
    help: &str,
) -> Result<Option<ProviderOfferDraft>, ActionError> {
    if !is_agent(engine, query.actor()) {
        return Ok(None);
    }
    let definitions = engine
        .registry
        .definitions()
        .ok_or_else(|| refuse("definition registry unavailable"))?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(RevisionSet::default())
        .map_err(|error| refuse(error.to_string()))?;
    guards
        .add_definition(action.clone(), definitions)
        .map_err(|error| refuse(error.to_string()))?;
    Ok(Some(ProviderOfferDraft {
        fixed_parameters: serde_json::json!({}),
        parameters: ProjectedParameterSchema::empty(),
        targets: ProjectedTargetSchema::None,
        required_channels: Vec::new(),
        expires: None,
        presentation: ActionPresentation::new(verb, label, help)
            .map_err(|error| refuse(error.to_string()))?,
        guards: guards.finish().map_err(|error| refuse(error.to_string()))?,
    }))
}

/// Every provider here acts only for the object itself.
fn require_self(invocation: &ActionInvocation) -> Result<(), ActionError> {
    let expected = PrincipalId::Object {
        id: invocation.actor(),
    };
    if invocation.principal() != &expected {
        return Err(refuse(
            "an npc decision action acts only for the npc itself",
        ));
    }
    Ok(())
}

fn outcome(action: &DefinitionRef, value: serde_json::Value) -> Result<RulesValue, ActionError> {
    RulesValue::new(action.clone(), value).map_err(|error| refuse(error.to_string()))
}

/// `npc_decide`: stage the plan the driving loop armed.
struct NpcDecideProvider {
    action: DefinitionRef,
    families: DecisionCaptureFamilies,
    sink: DecisionCaptureSink,
}

impl ActionProvider for NpcDecideProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        require_self(invocation)
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        agent_offer(
            engine,
            query,
            action,
            DECIDE_VERB,
            "Decide",
            "Capture a durable decision for this NPC.",
        )
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        require_self(invocation)?;
        let plan = self
            .sink
            .take()
            .ok_or_else(|| refuse("no decision capture plan is armed"))?;
        if plan.pending().actor != invocation.actor() {
            return Err(refuse("the armed capture plan is for another actor"));
        }
        let wake = plan
            .stage(txn, &self.families)
            .map_err(|error| refuse(error.to_string()))?;
        Ok(AuthoritativeActionOutput::new(outcome(
            &self.action,
            serde_json::json!({ "captured": true }),
        )?)
        .with_durable_outbox(vec![wake]))
    }
}

/// `npc_lease`: the kernel seeds the lease post-image; the provider adds only
/// its outcome.
struct NpcLeaseProvider {
    action: DefinitionRef,
}

impl ActionProvider for NpcLeaseProvider {
    fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }

    fn prepare_authoritative(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        Ok(AuthoritativeActionOutput::new(outcome(
            &self.action,
            serde_json::json!({ "leased": true }),
        )?))
    }
}

/// `npc_idle`: the no-op every NPC holds — one turn-receipt post-image and
/// nothing else.
struct NpcIdleProvider {
    action: DefinitionRef,
    receipts: StateFamilyHandle<CanonicalId, TurnReceipt>,
}

impl ActionProvider for NpcIdleProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        require_self(invocation)
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        agent_offer(
            engine,
            query,
            action,
            IDLE_VERB,
            "Idle",
            "Do nothing this turn.",
        )
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        require_self(invocation)?;
        let npc = invocation.actor();
        let key = receipt_key(npc).map_err(|error| refuse(error.to_string()))?;
        let receipt = TurnReceipt {
            npc,
            outcome: IDLE_VERB.to_owned(),
        };
        let existing = txn
            .get(&self.receipts, &key)
            .map_err(|error| refuse(error.to_string()))?
            .cloned();
        match existing {
            Some(_) => txn
                .replace(&self.receipts, &key, receipt)
                .map_err(|error| refuse(error.to_string()))?,
            None => txn
                .insert(&self.receipts, &key, receipt)
                .map_err(|error| refuse(error.to_string()))?,
        }
        Ok(AuthoritativeActionOutput::new(outcome(
            &self.action,
            serde_json::json!({ "idle": true }),
        )?))
    }
}

/// The deadline fallback: select the `idle` offer, which every NPC prompt
/// carries by construction; a prompt without it is an error, never a guess.
struct NpcIdleFallback {
    identity: CanonicalId,
}

impl DecisionFallbackAdapter for NpcIdleFallback {
    fn identity(&self) -> &CanonicalId {
        &self.identity
    }

    fn select(
        &self,
        _policy: &FallbackPolicyRef,
        prompt: &DecisionPrompt,
    ) -> Result<DecisionFallbackSelection, String> {
        select_idle(prompt)
    }
}

/// The idle fallback's selection: the `idle` offer of `prompt`, or an error
/// for a prompt that carries none (never a guess at another offer).
///
/// # Errors
/// The prompt has no offer whose verb is [`IDLE_VERB`].
pub fn select_idle(prompt: &DecisionPrompt) -> Result<DecisionFallbackSelection, String> {
    let idle = prompt
        .actions
        .iter()
        .find(|action| action.presentation().verb == IDLE_VERB)
        .ok_or_else(|| "the npc prompt carries no idle offer".to_owned())?;
    Ok(DecisionFallbackSelection {
        offer: idle.offer(),
        targets: Vec::new(),
        parameters: serde_json::json!({}),
    })
}
