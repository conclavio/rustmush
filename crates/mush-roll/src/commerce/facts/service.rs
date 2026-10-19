//! One bounded automatic intent. Unknown is retained; only actual installed
//! history or a consumed definitive disposition can release an attempt.
use super::super::{LicenceId, TermGeneration, families::CommerceFamilies, rating::RatingFamilies};
use super::*;
use mush_core::action::{
    ActionError, ActionInvocation, AuthorityGuard, AuthorityGuardContext, AuthorityGuardKind,
    AuthorityGuardValidator, IdempotencyKey, IdempotencyKeyText, PerceivedRef,
};
use mush_core::mechanics::CanonicalId;
use mush_core::submission_observation::SubmissionObservation;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, ContinuationHandle, Ctx,
    DefinitionDigest, DefinitionRef, GOD, TickCadence,
};
use std::sync::Mutex;
use std::time::Duration;
const COMMAND: &str = "@mindfacts";
const PAGE: usize = 16;
#[derive(Clone, Default)]
pub(crate) struct Runtime(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    owner_after: Option<String>,
    paid_after: Option<String>,
    paid_next: bool,
    pending: Option<Pending>,
}
struct Pending {
    intent: Intent,
    observation: Option<SubmissionObservation>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct Intent {
    pub(super) request: protocol::Request,
    pub(super) accepted: records::AcceptedTerm,
    pub(super) observed: mush_core::ClockStamp,
    pub(super) action: DefinitionRef,
    pub(super) key: IdempotencyKeyText,
}
impl Intent {
    fn new(
        request: protocol::Request,
        accepted: records::AcceptedTerm,
        world: &World,
        action: &DefinitionRef,
    ) -> Result<Self, PublicationError> {
        let observed = world.world_ic_now();
        accepted.expired(&observed)?;
        let digest=DefinitionDigest::for_canonical_value("roll.minds_lapse_intent_key.v1",&serde_json::json!({"request":request,"term":accepted,"clock":observed.clock,"action":action})).map_err(invalid)?;
        Ok(Self {
            request,
            accepted,
            observed,
            action: action.clone(),
            key: IdempotencyKeyText::parse_canonical(digest.to_hex()).map_err(invalid)?,
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
        self.accepted.expired(&now)
    }
    pub(super) fn guard(&self) -> Result<AuthorityGuard, PublicationError> {
        let hash = DefinitionDigest::for_canonical_value(
            "roll.minds_lapse_intent_capability.v1",
            &serde_json::to_value(self).map_err(invalid)?,
        )
        .map_err(invalid)?;
        Ok(AuthorityGuard::Capability {
            provider: OwnerId::new(crate::NS).map_err(invalid)?,
            actor: GOD,
            capability: CanonicalId::new(hash.to_hex()).map_err(invalid)?,
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
        let request = protocol::Request::parse(inv, action)?;
        let state = self.0.lock().map_err(invalid)?;
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
        let state = self.0.lock().map_err(invalid)?;
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
    action: DefinitionRef,
    continuation: ContinuationHandle,
}
fn unavailable() -> CmdError {
    CmdError::msg("Minds fact processing is unavailable.")
}
pub(super) fn live(world: &World) -> bool {
    world.get(GOD).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
    })
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
fn installed(ctx: &Ctx<'_>, service: &Service, intent: &Intent) -> Result<bool, PublicationError> {
    service.ready.check(ctx.w)?;
    let Some(row) = ctx
        .action_state(&service.f.lapse_occurrences, &intent.request.key())
        .map_err(invalid)?
    else {
        return Ok(false);
    };
    row.matches(&intent.accepted)?;
    let proof = history::resolve(
        ctx,
        &service.f,
        &service.ratings,
        intent.request.licence,
        intent.request.generation,
    )?;
    proof.require_historical_lapse(row)?;
    ctx.w
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .check_required_root(&row.root()?)
        .map_err(invalid)?;
    Ok(true)
}
/// Only the registered Roll timer enters discovery. The public continuation
/// command can drive an already retained intent, but cannot create one.
fn tick(ctx: &mut Ctx<'_>, service: &Service) -> Result<(), PublicationError> {
    service.ready.check(ctx.w)?;
    if !live(ctx.w) {
        return Err(PublicationError::Unavailable);
    }
    let mut state = service.runtime.0.lock().map_err(invalid)?;
    if let Some(pending) = state.pending.as_mut() {
        // A complete real readback is required before consuming any disposition.
        if installed(ctx, service, &pending.intent)? {
            state.pending = None;
        } else if pending
            .observation
            .as_mut()
            .is_some_and(|o| o.take_terminal().is_some())
        {
            pending.observation = None;
            if pending.intent.fresh(ctx.w).is_err() {
                state.pending = None;
            }
        } else if pending.observation.is_none() && pending.intent.fresh(ctx.w).is_err() {
            // No submitted attempt exists, and the exact durable read above
            // proved absence. Unknown always retains Some observation.
            state.pending = None;
        }
    }
    if state.pending.is_none() {
        let paid = state.paid_next;
        let after = if paid {
            state.paid_after.as_deref()
        } else {
            state.owner_after.as_deref()
        };
        let (rows, next): (Vec<(LicenceId, TermGeneration)>, Option<String>) = if paid {
            let page = ctx
                .scan_action_state(&service.f.paid_terms, after, PAGE)
                .map_err(invalid)?;
            let mut rows = Vec::with_capacity(page.entries().len());
            for item in page.entries() {
                let row = item.value();
                row.validate()?;
                rows.push((row.licence(), row.generation()));
            }
            (rows, page.next_after().map(ToString::to_string))
        } else {
            let page = ctx
                .scan_action_state(&service.f.terms, after, PAGE)
                .map_err(invalid)?;
            let mut rows = Vec::with_capacity(page.entries().len());
            for item in page.entries() {
                let row = item.value();
                row.validate()?;
                rows.push((row.licence, row.generation));
            }
            (rows, page.next_after().map(ToString::to_string))
        };
        for (licence, generation) in rows {
            let request = protocol::Request {
                licence,
                generation,
            };
            if ctx
                .action_state(&service.f.lapse_occurrences, &request.key())
                .map_err(invalid)?
                .is_some()
            {
                continue;
            }
            let Ok(proof) =
                history::resolve(ctx, &service.f, &service.ratings, licence, generation)
            else {
                continue;
            };
            if proof.require_lapsed(&ctx.w.world_ic_now()).is_err() {
                continue;
            }
            let intent = Intent::new(request, proof.accepted, ctx.w, &service.action)?;
            state.pending = Some(Pending {
                intent,
                observation: None,
            });
            break;
        }
        if paid {
            state.paid_after = next;
        } else {
            state.owner_after = next;
        }
        state.paid_next = !paid;
    }
    let schedule = state
        .pending
        .as_ref()
        .is_some_and(|p| p.observation.is_none());
    drop(state);
    if schedule && !ctx.authoritative_pending(GOD) {
        ctx.schedule_continuation(&service.continuation, GOD, "", 0)
            .map_err(invalid)?;
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
        // The exact private offer supplies the fixed subject/generation. The
        // ordinary public selector must not duplicate its fixed parameters.
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
    action: DefinitionRef,
) -> Result<(), ModError> {
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: COMMAND.into(),
            switches: vec!["continue".into()],
            shape: ArgShape::Raw,
            help: "Internal bounded minds-fact processing.".into(),
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
        action,
        continuation,
    };
    reg.state(service.clone());
    reg.authority_guard_validator(AuthorityGuardKind::Capability, Guard { runtime, ready })?;
    reg.tick(
        "personal_mind_term_lapses",
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            move |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx, &service);
            },
        ),
    )
}
