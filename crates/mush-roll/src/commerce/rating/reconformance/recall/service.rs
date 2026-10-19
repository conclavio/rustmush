//! One bounded automatic intent. Unknown is retained; only actual installed
//! history or a consumed definitive disposition can release an attempt.
use super::history::RejectionProof;
use super::*;
use crate::commerce::{FiledRevisionId, rating::RatingCompatibility};
use mush_core::action::{
    ActionError, ActionInvocation, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    IdempotencyKey, IdempotencyKeyText, PerceivedRef,
};
use mush_core::mechanics::CanonicalId;
use mush_core::submission_observation::SubmissionObservation;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, ContinuationHandle, Ctx,
    DefinitionDigest, DefinitionRef, GOD, TickCadence,
};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
const COMMAND: &str = "@registry-recall";
const PAGE: usize = 16;
#[derive(Clone, Default)]
pub(crate) struct Runtime(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    after: Option<String>,
    candidates: VecDeque<FiledRevisionId>,
    pending: Option<Pending>,
}
struct Pending {
    intent: Intent,
    observation: Option<SubmissionObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Intent {
    pub(super) request: protocol::Request,
    pub(super) proof: RejectionProof,
    pub(super) observed: mush_core::ClockStamp,
    pub(super) action: DefinitionRef,
    pub(super) key: IdempotencyKeyText,
}
impl Intent {
    fn new(
        request: protocol::Request,
        proof: RejectionProof,
        world: &World,
        action: &DefinitionRef,
    ) -> Result<Self, PublicationError> {
        let observed = world.world_ic_now();
        if proof.status.recalled_by.is_some()
            || (observed.clock == proof.measured_at.clock && observed.at < proof.measured_at.at)
        {
            return Err(PublicationError::Unavailable);
        }
        let digest=DefinitionDigest::for_canonical_value("roll.registry.measured_recall.intent_key.v1",&serde_json::json!({"request":request,"proof":proof,"clock":observed.clock,"action":action})).map_err(bad)?;
        Ok(Self {
            request,
            proof,
            observed,
            action: action.clone(),
            key: IdempotencyKeyText::parse_canonical(digest.to_hex()).map_err(bad)?,
        })
    }
    pub(super) fn parameters(&self) -> serde_json::Value {
        serde_json::json!(self.request)
    }
    pub(super) fn fresh(&self, world: &World) -> Result<(), PublicationError> {
        let now = world.world_ic_now();
        if now.clock != self.observed.clock || now.at < self.observed.at {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn guard(&self) -> Result<AuthorityGuard, PublicationError> {
        let hash = DefinitionDigest::for_canonical_value(
            "roll.registry.measured_recall.intent_capability.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)?;
        Ok(AuthorityGuard::Capability {
            provider: OwnerId::new(crate::NS).map_err(bad)?,
            actor: GOD,
            capability: CanonicalId::new(hash.to_hex()).map_err(bad)?,
            generation: 1,
        })
    }
}
impl Runtime {
    pub(super) fn projected(&self) -> Option<Intent> {
        self.0
            .lock()
            .ok()?
            .pending
            .as_ref()
            .map(|p| p.intent.clone())
    }

    pub(super) fn intent(
        &self,
        inv: &ActionInvocation,
        action: &DefinitionRef,
    ) -> Result<Intent, PublicationError> {
        let request = protocol::Request::parse(inv)?;
        let state = self.0.lock().map_err(bad)?;
        let intent = &state
            .pending
            .as_ref()
            .ok_or(PublicationError::Unavailable)?
            .intent;
        if intent.action != *action
            || intent.request != request
            || inv.parameters() != &intent.parameters()
            || inv.idempotency().key() != &IdempotencyKey::Caller(intent.key.clone())
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(intent.clone())
    }
    pub(super) fn require(&self, intent: &Intent) -> Result<(), PublicationError> {
        let state = self.0.lock().map_err(bad)?;
        if state.pending.as_ref().is_none_or(|p| &p.intent != intent) {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Service {
    runtime: Runtime,
    ready: Readiness,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
    action: DefinitionRef,
    continuation: ContinuationHandle,
}
fn unavailable() -> CmdError {
    CmdError::msg("Registry recall processing is unavailable.")
}
pub(super) fn live(world: &World) -> bool {
    world.get(GOD).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
    })
}
// The single Roll Capability dispatcher reads this actual registered service.
// No caller can supply a runtime or turn a public selector into an intent.
pub(crate) fn guard_current(
    ctx: &AuthorityGuardContext<'_>,
    guard: &AuthorityGuard,
) -> Result<bool, ActionError> {
    if ctx
        .world()
        .module_state_owner(std::any::TypeId::of::<Service>())
        .is_none_or(|owner| owner.as_str() != crate::NS)
    {
        return Ok(false);
    }
    let Some(service) = ctx.world().module_state::<Service>() else {
        return Ok(false);
    };
    Guard {
        runtime: service.runtime.clone(),
        ready: service.ready.clone(),
    }
    .is_current(ctx, guard)
}
struct Guard {
    runtime: Runtime,
    ready: Readiness,
}
impl AuthorityGuardValidator for Guard {
    fn is_current(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        if self.ready.check(ctx.world()).is_err() || !live(ctx.world()) {
            return Ok(false);
        }
        let Ok(state) = self.runtime.0.lock() else {
            return Ok(false);
        };
        Ok(state.pending.as_ref().is_some_and(|p| {
            p.intent.guard().ok().as_ref() == Some(guard) && p.intent.fresh(ctx.world()).is_ok()
        }))
    }
}
fn installed(ctx: &Ctx<'_>, s: &Service, intent: &Intent) -> Result<bool, PublicationError> {
    s.ready.check(ctx.w)?;
    let status = ctx
        .action_state(&s.f.status, &intent.request.filed.key())
        .map_err(bad)?
        .ok_or(PublicationError::Mismatch)?;
    let Some(cause) = status.recalled_by else {
        return Ok(false);
    };
    let Some(row) = ctx
        .action_state(&s.f.measured_recalls, &cause.to_hex())
        .map_err(bad)?
    else {
        return Ok(false);
    };
    history::verify(ctx, &s.f, &s.ratings, row)?;
    if row.filed != intent.request.filed
        || row.rejection != intent.proof.rejection
        || row.action != intent.action
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(true)
}
fn valid(ctx: &Ctx<'_>, s: &Service, intent: &Intent) -> bool {
    intent.fresh(ctx.w).is_ok()
        && history::resolve(ctx, &s.f, &s.ratings, intent.request.filed, &s.expected)
            .is_ok_and(|p| p == intent.proof && p.status.recalled_by.is_none())
}
// Only this registered timer discovers work. One full measured proof per discovery
// tick; metadata pagination retains at most sixteen candidates, never all results.
fn tick(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), PublicationError> {
    s.ready.check(ctx.w)?;
    if !live(ctx.w) {
        return Err(PublicationError::Unavailable);
    }
    let mut state = s.runtime.0.lock().map_err(bad)?;
    if let Some(p) = state.pending.as_mut() {
        if installed(ctx, s, &p.intent)? {
            state.pending = None;
        } else if p
            .observation
            .as_mut()
            .is_some_and(|o| o.take_terminal().is_some())
        {
            p.observation = None;
            if !valid(ctx, s, &p.intent) {
                state.pending = None;
            }
        } else if p.observation.is_none() && !valid(ctx, s, &p.intent) {
            state.pending = None;
        }
        // Unknown retains its exact intent and is never interpreted as absence.
    }
    if state.pending.is_none() {
        if state.candidates.is_empty() {
            let page = ctx
                .scan_action_state(&s.f.reconformance_heads, state.after.as_deref(), PAGE)
                .map_err(bad)?;
            let mut rows = VecDeque::new();
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                if row.key() != entry.encoded_key() {
                    return Err(PublicationError::Mismatch);
                }
                if row.compatibility == s.expected.digest() {
                    rows.push_back(row.filed);
                }
            }
            state.after = page.next_after().map(ToString::to_string);
            state.candidates = rows;
        }
        if let Some(filed) = state.candidates.pop_front() {
            if let Ok(proof) = history::resolve(ctx, &s.f, &s.ratings, filed, &s.expected) {
                if proof.status.recalled_by.is_none() {
                    let intent = Intent::new(protocol::Request { filed }, proof, ctx.w, &s.action)?;
                    state.pending = Some(Pending {
                        intent,
                        observation: None,
                    });
                }
            }
        }
    }
    let schedule = state
        .pending
        .as_ref()
        .is_some_and(|p| p.observation.is_none());
    drop(state);
    if schedule && !ctx.authoritative_pending(GOD) {
        ctx.schedule_continuation(&s.continuation, GOD, "", 0)
            .map_err(bad)?;
    }
    Ok(())
}
fn drive(ctx: &mut Ctx<'_>, service: &Service) -> Result<(), CmdError> {
    if ctx.dispatch_actor() != Some(GOD) || ctx.actor != GOD || !ctx.args.raw().is_empty() {
        return Err(unavailable());
    }
    service.ready.check(ctx.w).map_err(|_| unavailable())?;
    let mut state = service.runtime.0.lock().map_err(|_| unavailable())?;
    let Some(pending) = state.pending.as_mut() else {
        return Err(unavailable());
    };
    if pending.observation.is_some() || ctx.authoritative_pending(GOD) {
        return Ok(());
    }
    pending.intent.fresh(ctx.w).map_err(|_| unavailable())?;
    pending.observation = Some(ctx.submit_authoritative_observed(
        pending.intent.action.clone(),
        PerceivedRef::SelfActor,
        vec![],
        // The private offer fixes the exact filed revision; the public selector is empty.
        serde_json::json!({}),
        pending.intent.key.clone(),
    )?);
    Ok(())
}
pub(super) fn register(
    reg: &mut Registrar<'_>,
    runtime: Runtime,
    ready: Readiness,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    expected: RatingCompatibility,
    action: DefinitionRef,
) -> Result<(), ModError> {
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: COMMAND.into(),
            switches: vec!["continue".into()],
            shape: ArgShape::Raw,
            help: "Internal bounded measured Registry recall.".into(),
            ..Default::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| {
            let service = ctx.state::<Service>().cloned().ok_or_else(unavailable)?;
            drive(ctx, &service)
        }),
    )?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    let service = Service {
        runtime: runtime.clone(),
        ready: ready.clone(),
        f,
        ratings,
        expected,
        action,
        continuation,
    };
    reg.state(service.clone());
    reg.tick(
        "measured_registry_recalls",
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            move |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx, &service);
            },
        ),
    )
}
