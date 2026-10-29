//! Private registered continuation for retained paid-property escrow. No public
//! selector, caller-supplied reason or service actor identity creates an intent.
use super::{history, records::*};
use crate::commerce::{
    RevisionIdentity, families::CommerceFamilies, rating::RatingFamilies, read::CommerceRead,
    records::PublicationError,
};
use mush_core::action::{
    ActionInvocation, AuthorityGuard, IdempotencyKeyText, PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_economy::{
    EconomyStore,
    managed_paid_resale::{PaidResaleAvailability, PaidResaleDecisionKindV1 as Reason},
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}
fn live(w: &World, actor: ObjId) -> bool {
    w.get(actor).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Intent {
    pub(super) acceptance: CommitToken,
    pub(super) before: PaidResaleRef,
    pub(super) revision: RevisionIdentity,
    pub(super) actor: ObjId,
    pub(super) action: DefinitionRef,
    pub(super) reason: Reason,
    pub(super) claims: Option<ResaleClaimObservation>,
    pub(super) observed: ClockStamp,
    pub(super) observed_utc: i64,
    pub(super) key: IdempotencyKeyText,
}
impl Intent {
    fn new(
        row: &PaidResaleRecord,
        revision: RevisionIdentity,
        actor: ObjId,
        reason: Reason,
        claims: Option<ResaleClaimObservation>,
        world: &World,
    ) -> Result<Self, PublicationError> {
        let action = super::protocol::action(if reason.settles() { 3 } else { 4 })?;
        let before = row.reference()?;
        let observed = world.world_ic_now();
        let key = DefinitionDigest::for_canonical_value("roll.paid_resale.intent_key.v1", &serde_json::json!({"acceptance":row.operation(),"before":before,"actor":actor,"action":action,"reason":reason,"claims":claims,"clock":observed.clock})).map_err(bad)?;
        let value = Self {
            acceptance: row.operation(),
            before,
            revision,
            actor,
            action,
            reason,
            claims,
            observed,
            observed_utc: world.now(),
            key: IdempotencyKeyText::parse_canonical(key.to_hex()).map_err(bad)?,
        };
        value.fresh(world)?;
        Ok(value)
    }
    pub(super) fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"acceptance":self.acceptance})
    }
    pub(super) fn guard(&self) -> Result<AuthorityGuard, PublicationError> {
        let d = DefinitionDigest::for_canonical_value(
            "roll.paid_resale.intent_guard.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)?;
        Ok(AuthorityGuard::Capability {
            provider: OwnerId::new(crate::NS).map_err(bad)?,
            actor: self.actor,
            capability: CanonicalId::new(d.to_hex()).map_err(bad)?,
            generation: 1,
        })
    }
    pub(super) fn fresh(&self, w: &World) -> Result<(), PublicationError> {
        let now = w.world_ic_now();
        if !live(w, self.actor)
            || now.clock != self.observed.clock
            || now.at < self.observed.at
            || w.now() < self.observed_utc
        {
            return Err(PublicationError::Unavailable);
        }
        let e = w
            .module_state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let actual = e.paid_resale_history(self.acceptance).map_err(bad)?;
        if actual.terminal().is_some()
            || actual.outcome() != mush_economy::managed_paid_resale::PaidResaleOutcome::Funded
        {
            return Err(PublicationError::Unavailable);
        }
        let c = actual.consent();
        let roll = w
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        if let Some(expected) = &self.claims {
            let id = crate::commerce::LicenceId::new(c.terms.basis.licence).map_err(bad)?;
            if &roll.paid_resale_claims(id).map_err(bad)? != expected {
                return Err(PublicationError::Mismatch);
            }
        }
        let same = now.clock == c.accepted_at_ic.clock && now.at >= c.accepted_at_ic.at;
        match &self.reason {
            Reason::ClockChanged {} if now.clock != c.accepted_at_ic.clock => (),
            Reason::RecipientUnavailable {}
                if same
                    && e.paid_resale_availability(w, self.acceptance)
                        .map_err(bad)?
                        == PaidResaleAvailability::RecipientUnavailable => {}
            Reason::MonetaryBindingChanged {}
                if same
                    && e.paid_resale_availability(w, self.acceptance)
                        .map_err(bad)?
                        == PaidResaleAvailability::MonetaryBindingChanged => {}
            Reason::LastClaimClosed { next_transition }
                if same
                    && e.paid_resale_availability(w, self.acceptance)
                        .map_err(bad)?
                        == PaidResaleAvailability::Current
                    && *next_transition > c.claims.next_transition
                    && self.claims.as_ref().is_some_and(|v| {
                        v.open_paid() == 0 && v.next_transition() == *next_transition
                    }) => {}
            Reason::DeadlineReached {
                next_transition,
                open_paid,
            } if c.deadline_reached(&now).map_err(bad)?
                && e.paid_resale_availability(w, self.acceptance)
                    .map_err(bad)?
                    == PaidResaleAvailability::Current
                && self.claims.as_ref().is_some_and(|v| {
                    v.open_paid() == *open_paid && v.next_transition() == *next_transition
                }) => {}
            Reason::RegistryRevoked {
                licence,
                generation,
                term_digest,
                ..
            } if *licence == c.terms.basis.licence
                && *generation == c.terms.basis.term.generation
                && *term_digest == c.terms.basis.term.digest
                && now.at < c.terms.basis.term.window.grace_end
                && (now.clock != c.accepted_at_ic.clock || same) => {}
            _ => return Err(PublicationError::Unavailable),
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct Runtime(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    after: Option<String>,
    pending: Option<Attempt>,
}
struct Attempt {
    intent: Intent,
    observation: Option<mush_core::submission_observation::SubmissionObservation>,
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
    pub(super) fn require(&self, intent: &Intent) -> Result<(), PublicationError> {
        if self
            .0
            .lock()
            .map_err(bad)?
            .pending
            .as_ref()
            .is_none_or(|p| &p.intent != intent)
        {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn intent(
        &self,
        inv: &ActionInvocation,
        action: &DefinitionRef,
    ) -> Result<Intent, PublicationError> {
        let v = self.projected().ok_or(PublicationError::Unavailable)?;
        if &v.action != action
            || inv.actor() != v.actor
            || inv.principal() != &(PrincipalId::Object { id: v.actor })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.parameters() != &v.parameters()
            || inv.idempotency().scope().owner != action.key().owner
            || !inv.keyed_by(&v.key)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(v)
    }
}

fn discover(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    row: &PaidResaleRecord,
    actor: ObjId,
) -> Result<Option<Intent>, PublicationError> {
    row.validate()?;
    if !matches!(row.state(), PaidResaleState::Funded {}) {
        return Ok(None);
    }
    let head = ctx
        .get(&f.paid_resale_heads, &row.binding().licence().key())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    head.matches(row)?;
    let pointer = ctx
        .get(&f.paid_licences, &row.binding().licence().key())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    let birth = history::birth(ctx, f, ratings, pointer)?;
    history::native(ctx.world(), row)?;
    history::roots(ctx.world(), row, birth.revision)?;
    use crate::commerce::registry_revocation::records::{AcceptedTerm, RevokedTerm};
    let w = ctx.world();
    let now = w.world_ic_now();
    if let Some(status) = ctx
        .get(
            &f.revoked_terms,
            &RevokedTerm::key_for(row.binding().licence(), row.binding().term().generation()),
        )
        .map_err(bad)?
    {
        let op = ctx
            .get(&f.registry_revocations, &status.operation.to_hex())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        status.matches(op)?;
        let AcceptedTerm::Paid { term } = &op.term else {
            return Err(PublicationError::Mismatch);
        };
        if term.as_ref() != row.binding().term() || op.charter != row.binding().charter() {
            return Err(PublicationError::Mismatch);
        }
        w.module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .check_required_root(&op.root()?)
            .map_err(bad)?;
        if now.at < row.economic().consent().terms.basis.term.window.grace_end {
            return Intent::new(
                row,
                birth.revision,
                actor,
                Reason::RegistryRevoked {
                    sanction: op.operation,
                    licence: term.licence().get(),
                    generation: term.generation().get(),
                    term_digest: row.binding().native_basis()?.term.digest,
                },
                None,
                w,
            )
            .map(Some);
        }
    }
    let c = row.economic().consent();
    if now.clock != c.accepted_at_ic.clock {
        return Intent::new(row, birth.revision, actor, Reason::ClockChanged {}, None, w).map(Some);
    }
    if now.at < c.accepted_at_ic.at {
        return Err(PublicationError::Unavailable);
    }
    let e = w
        .module_state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let reason = match e
        .paid_resale_availability(w, row.operation())
        .map_err(bad)?
    {
        PaidResaleAvailability::RecipientUnavailable => Some(Reason::RecipientUnavailable {}),
        PaidResaleAvailability::MonetaryBindingChanged => Some(Reason::MonetaryBindingChanged {}),
        PaidResaleAvailability::Current => None,
    };
    if let Some(reason) = reason {
        return Intent::new(row, birth.revision, actor, reason, None, w).map(Some);
    }
    let claims = w
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .paid_resale_claims(row.binding().licence())
        .map_err(bad)?;
    if claims.next_transition() < c.claims.next_transition
        || claims.open_paid() > c.claims.open_paid
    {
        return Err(PublicationError::Mismatch);
    }
    let reason = if claims.open_paid() == 0 && claims.next_transition() > c.claims.next_transition {
        Reason::LastClaimClosed {
            next_transition: claims.next_transition(),
        }
    } else if c.deadline_reached(&now).map_err(bad)? {
        Reason::DeadlineReached {
            next_transition: claims.next_transition(),
            open_paid: claims.open_paid(),
        }
    } else {
        return Ok(None);
    };
    Intent::new(row, birth.revision, actor, reason, Some(claims), w).map(Some)
}

#[derive(Clone)]
struct Service {
    runtime: Runtime,
    ready: super::Readiness,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    actor: ObjId,
    continuation: mush_core::ContinuationHandle,
}
/// The shared Roll capability dispatcher consults only this actual registered
/// private slot. Knowledge of its public action/actor never substitutes for it.
pub(crate) fn guard_current(
    ctx: &mush_core::action::AuthorityGuardContext<'_>,
    guard: &AuthorityGuard,
) -> Result<bool, mush_core::action::ActionError> {
    if ctx
        .world()
        .module_state_owner(std::any::TypeId::of::<Service>())
        .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Ok(false);
    }
    let Some(s) = ctx.world().module_state::<Service>() else {
        return Ok(false);
    };
    Ok(s.ready.check().is_ok()
        && s.runtime.projected().is_some_and(|i| {
            i.guard().ok().as_ref() == Some(guard) && i.fresh(ctx.world()).is_ok()
        }))
}
fn installed(
    ctx: &impl CommerceRead,
    service: &Service,
    intent: &Intent,
) -> Result<bool, PublicationError> {
    let row = ctx
        .get(&service.families.paid_resales, &intent.acceptance.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    history::native(ctx.world(), row)?;
    history::roots(ctx.world(), row, intent.revision)?;
    if matches!(row.state(), PaidResaleState::Funded {}) {
        if row.reference()? != intent.before {
            return Err(PublicationError::Mismatch);
        }
        Ok(false)
    } else {
        Ok(true)
    }
}
fn tick(ctx: &mut mush_core::Ctx<'_>, service: &Service) -> Result<(), PublicationError> {
    service.ready.check()?;
    if !live(ctx.w, service.actor) {
        return Err(PublicationError::Unavailable);
    }
    let mut state = service.runtime.0.lock().map_err(bad)?;
    if let Some(p) = state.pending.as_mut() {
        if installed(ctx, service, &p.intent)? {
            state.pending = None;
        } else if p
            .observation
            .as_mut()
            .is_some_and(|o| o.take_terminal().is_some())
        {
            // Only the observation's definitive NotDispatched/NoIntent result
            // reaches here. Unknown retains both the original intent and key.
            p.observation = None;
            if p.intent.fresh(ctx.w).is_err() {
                state.pending = None;
            }
        } else if p.observation.is_none() && p.intent.fresh(ctx.w).is_err() {
            state.pending = None;
        }
    }
    if state.pending.is_none() {
        let page = ctx
            .scan_action_state(
                &service.families.paid_resale_heads,
                state.after.as_deref(),
                16,
            )
            .map_err(bad)?;
        let candidates = page
            .entries()
            .iter()
            .map(|e| (e.encoded_key().to_string(), e.value().clone()))
            .collect::<Vec<_>>();
        state.after = page.next_after().map(ToString::to_string);
        // At most one full native/issued-term proof per tick. Advance after
        // every examined funded head, including not-yet-due or unavailable ones.
        for (key, head) in candidates {
            head.validate()?;
            if !matches!(head.state(), PaidResaleState::Funded {}) {
                continue;
            }
            state.after = Some(key);
            let row = ctx
                .action_state(
                    &service.families.paid_resales,
                    &head.current().operation().to_hex(),
                )
                .map_err(bad)?
                .ok_or(PublicationError::Missing)?;
            head.matches(row)?;
            if let Some(intent) =
                discover(ctx, &service.families, &service.ratings, row, service.actor)?
            {
                state.pending = Some(Attempt {
                    intent,
                    observation: None,
                });
            }
            break;
        }
    }
    let schedule = state
        .pending
        .as_ref()
        .is_some_and(|p| p.observation.is_none());
    drop(state);
    if schedule && !ctx.authoritative_pending(service.actor) {
        ctx.schedule_continuation(&service.continuation, service.actor, "", 0)
            .map_err(bad)?;
    }
    Ok(())
}
fn unavailable() -> mush_core::CmdError {
    mush_core::CmdError::msg("Paid licence settlement processing is unavailable.")
}
fn drive(ctx: &mut mush_core::Ctx<'_>, s: &Service) -> Result<(), mush_core::CmdError> {
    if ctx.actor != s.actor || ctx.dispatch_actor() != Some(s.actor) || !ctx.args.raw().is_empty() {
        return Err(unavailable());
    }
    s.ready.check().map_err(|_| unavailable())?;
    let mut state = s.runtime.0.lock().map_err(|_| unavailable())?;
    let p = state.pending.as_mut().ok_or_else(unavailable)?;
    if p.observation.is_some() || ctx.authoritative_pending(s.actor) {
        return Ok(());
    }
    p.intent.fresh(ctx.w).map_err(|_| unavailable())?;
    p.observation = Some(ctx.submit_authoritative_observed(
        p.intent.action.clone(),
        PerceivedRef::SelfActor,
        vec![],
        serde_json::json!({}),
        p.intent.key.clone(),
    )?);
    Ok(())
}
pub(super) fn register(
    reg: &mut mush_core::Registrar<'_>,
    runtime: Runtime,
    ready: super::Readiness,
    families: CommerceFamilies,
    ratings: RatingFamilies,
    actor: ObjId,
) -> Result<(), mush_core::ModError> {
    const COMMAND: &str = "@paidresale";
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        mush_core::CommandSpec {
            name: COMMAND.into(),
            switches: vec!["continue".into()],
            shape: mush_core::ArgShape::Raw,
            help: "Internal bounded paid licence settlement.".into(),
            ..Default::default()
        },
        Arc::new(|ctx: &mut mush_core::Ctx<'_>| {
            let s = ctx.state::<Service>().cloned().ok_or_else(unavailable)?;
            drive(ctx, &s)
        }),
    )?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    let s = Service {
        runtime,
        ready,
        families,
        ratings,
        actor,
        continuation,
    };
    reg.state(s.clone());
    reg.tick(
        "paid_resale_terminals",
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(1)),
        Arc::new(
            move |ctx: &mut mush_core::Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx, &s);
            },
        ),
    )
}
