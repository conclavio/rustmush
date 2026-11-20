//! Registered terminal custody only. A retained private intent authorizes the
//! service actor; a public action name or selector never creates that authority.
use super::{PendingCharterTransferConfig, Readiness, bad, history, protocol, records::*};
use crate::commerce::{
    families::CommerceFamilies,
    read::CommerceRead,
    records::{PublicationError, Record},
};
use mush_core::action::{
    ActionError, ActionInvocation, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
    IdempotencyKeyText, PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};
use mush_core::{
    ArgShape, ClockStamp, CmdError, CommandCompatibility, CommandSpec, CommitToken,
    ContinuationHandle, Ctx, DefinitionDigest, DefinitionRef, ModError, ObjId, Registrar,
    TickCadence, World,
};
use mush_economy::managed_charter_transfer::CharterPendingDecisionKindV1 as Reason;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const COMMAND: &str = "@charterpending";
const PAGE: usize = 16;

pub(super) fn live(world: &World, actor: ObjId) -> bool {
    world.get(actor).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
    })
}

// Read-only candidate selection mirrors the closed native personal recipient
// shape. It grants nothing: the real E4 terminal preparation repeats its own
// WorldAuth title-receipt and account predicates before custody.
fn receivers(
    world: &World,
    c: &mush_economy::managed_charter_transfer::CharterPendingConsentV1,
) -> Result<bool, PublicationError> {
    let e = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if e.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let s = e.snapshot();
    let Some(payer) = s.account(c.payer) else {
        return Ok(false);
    };
    let Some(seller) = s.account(c.terms.seller_account) else {
        return Ok(false);
    };
    Ok(live(world, c.terms.buyer)
        && payer.owner == mush_org::PrincipalId::Object(c.terms.buyer)
        && payer.scope.is_none()
        && payer.status == mush_economy::AccountStatus::Active
        && payer.instrument == c.terms.instrument
        && seller.owner == mush_org::PrincipalId::Object(c.terms.seller)
        && seller.scope.is_none()
        && seller.status == mush_economy::AccountStatus::Active
        && seller.instrument == c.terms.instrument
        && s.instrument(&c.terms.instrument)
            .is_some_and(|i| i.status == mush_economy::InstrumentStatus::Active))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Intent {
    pub(super) funding: CommitToken,
    pub(super) pending: PendingRef,
    pub(super) actor: ObjId,
    pub(super) action: DefinitionRef,
    pub(super) reason: Reason,
    pub(super) claims: Option<ClaimObservation>,
    pub(super) observed: ClockStamp,
    pub(super) observed_utc: i64,
    pub(super) key: IdempotencyKeyText,
}
impl Intent {
    fn new(
        row: &PendingRecord,
        actor: ObjId,
        reason: Reason,
        claims: Option<ClaimObservation>,
        ctx: &impl CommerceRead,
        f: &CommerceFamilies,
    ) -> Result<Self, PublicationError> {
        let world = ctx.world();
        let action = if reason.settles() {
            protocol::settle_action()
        } else {
            protocol::refund_action()
        }
        .map_err(bad)?;
        let pending = row.reference()?;
        let observed = world.world_ic_now();
        let observed_utc = world.now();
        let digest = DefinitionDigest::for_canonical_value(
            "roll.charter_pending.intent_key.v1",
            &serde_json::json!({"funding":row.operation,"pending":pending,"actor":actor,
                "action":action,"reason":reason,"claims":claims,"clock":observed.clock}),
        )
        .map_err(bad)?;
        let out = Self {
            funding: row.operation,
            pending,
            actor,
            action,
            reason,
            claims,
            observed,
            observed_utc,
            key: IdempotencyKeyText::parse_canonical(digest.to_hex()).map_err(bad)?,
        };
        out.fresh_read(ctx, f)?;
        Ok(out)
    }
    pub(super) fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"funding":self.funding})
    }
    pub(super) fn guard(&self) -> Result<AuthorityGuard, PublicationError> {
        let digest = DefinitionDigest::for_canonical_value(
            "roll.charter_pending.intent_guard.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)?;
        Ok(AuthorityGuard::Capability {
            provider: OwnerId::new(crate::NS).map_err(bad)?,
            actor: self.actor,
            capability: CanonicalId::new(digest.to_hex()).map_err(bad)?,
            generation: 1,
        })
    }
    /// Typed row/status CAS is retained separately by the actual provider. This
    /// repeats the mutable native owner, claim inventory and physical lifecycle.
    pub(super) fn fresh(&self, world: &World) -> Result<(), PublicationError> {
        self.fresh_parties(world).map(|_| ())
    }
    pub(super) fn fresh_read(
        &self,
        ctx: &impl CommerceRead,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        let (seller, buyer) = self.fresh_parties(ctx.world())?;
        if crate::commerce::registry_inspection::principal::any_party_barred_read(
            ctx,
            f,
            &[seller, buyer],
        )? {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn fresh_guard(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        let (seller, buyer) = self.fresh_parties(ctx.world())?;
        if crate::commerce::registry_inspection::principal::any_party_barred_guard(
            ctx,
            f,
            &[seller, buyer],
        )? {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn fresh_engine(
        &self,
        engine: &mush_core::Engine,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        let (seller, buyer) = self.fresh_parties(&engine.world)?;
        if crate::commerce::registry_inspection::principal::any_party_barred_engine(
            engine,
            f,
            &[seller, buyer],
        )? {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
    fn fresh_parties(&self, world: &World) -> Result<(ObjId, ObjId), PublicationError> {
        let now = world.world_ic_now();
        if !live(world, self.actor)
            || now.clock != self.observed.clock
            || now.at < self.observed.at
            || world.now() < self.observed_utc
        {
            return Err(PublicationError::Unavailable);
        }
        let economy = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let native = economy.pending_charter_history(self.funding).map_err(bad)?;
        if native.terminal().is_some() {
            return Err(PublicationError::Unavailable);
        }
        let consent = native.consent();
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        if let Some(expected) = &self.claims {
            let charter = crate::commerce::CharterId::new(consent.terms.charter.0).map_err(bad)?;
            if &roll.charter_pending_claims(charter).map_err(bad)? != expected {
                return Err(PublicationError::Unavailable);
            }
        }
        match &self.reason {
            Reason::ClockChanged if now.clock != consent.accepted_at_ic.clock => (),
            Reason::RecipientUnavailable
                if now.clock == consent.accepted_at_ic.clock
                    && now.at >= consent.accepted_at_ic.at
                    && !receivers(world, consent)? => {}
            Reason::LastClaimClosed { next_transition }
                if now.clock == consent.accepted_at_ic.clock
                    && now.at >= consent.accepted_at_ic.at
                    && receivers(world, consent)?
                    && self.claims.as_ref().is_some_and(|c| {
                        c.open_owned == 0 && c.next_transition == *next_transition
                    }) => {}
            Reason::DeadlineReached {
                next_transition,
                open_owned,
            } if receivers(world, consent)?
                && consent.deadline_reached(&now).map_err(bad)?
                && self.claims.as_ref().is_some_and(|c| {
                    c.next_transition == *next_transition && c.open_owned == *open_owned
                }) => {}
            Reason::RegistryRevoked {
                licence,
                generation,
                term_digest,
                ..
            } if *licence == consent.terms.owner_term.licence
                && *generation == consent.terms.owner_term.generation
                && *term_digest == consent.terms.owner_term.digest => {}
            _ => return Err(PublicationError::Unavailable),
        }
        Ok((consent.terms.seller, consent.terms.buyer))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Runtime(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    after: Option<String>,
    pending: Option<Attempt>,
}
struct Attempt {
    intent: Intent,
    observation: Option<SubmissionObservation>,
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
        let intent = self.projected().ok_or(PublicationError::Unavailable)?;
        if &intent.action != action
            || inv.actor() != intent.actor
            || inv.principal() != &(PrincipalId::Object { id: intent.actor })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.parameters() != &intent.parameters()
            || inv.idempotency().scope().owner != action.key().owner
            || !inv.keyed_by(&intent.key)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(intent)
    }
}

/// Prove one candidate from real retained owner history. No source/parent/term
/// eligibility is consulted to drain accepted escrow.
fn discover(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    row: &PendingRecord,
    actor: ObjId,
) -> Result<Option<Intent>, PublicationError> {
    row.validate()?;
    if !matches!(row.state, PendingState::Funded {}) {
        return Ok(None);
    }
    let head = ctx
        .get(&f.charter_pending_heads, &row.charter.key())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    head.matches(row)?;
    let offer = ctx
        .get(&f.charter_pending_offers, &row.offer.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    row.matches_offer(offer)?;
    // Skip barred parties cleanly without wedging `state.pending` so unbarred
    // peer escrows continue settling until the bar expires or compound revoke runs.
    if crate::commerce::registry_inspection::principal::any_party_barred_read(
        ctx,
        f,
        &[offer.seller, offer.buyer],
    )? {
        return Ok(None);
    }
    let world = ctx.world();
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy
        .pending_charter_history(row.operation)
        .map_err(bad)?
        != &row.economic
    {
        return Err(PublicationError::Mismatch);
    }
    let roll = world
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    for root in [
        history::offer_root(offer)?,
        history::funding_root(row, offer)?,
    ] {
        roll.check_required_root(&root).map_err(bad)?;
    }
    use crate::commerce::registry_revocation::records::{AcceptedTerm, RevokedTerm};
    if let Some(status) = ctx
        .get(
            &f.revoked_terms,
            &RevokedTerm::key_for(row.owner_term.licence, row.owner_term.generation),
        )
        .map_err(bad)?
    {
        let op = ctx
            .get(&f.registry_revocations, &status.operation.to_hex())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        status.matches(op)?;
        let AcceptedTerm::Owner { term } = &op.term else {
            return Err(PublicationError::Mismatch);
        };
        row.owner_term.matches(term)?;
        if op.charter != row.charter {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&op.root()?).map_err(bad)?;
        return Intent::new(
            row,
            actor,
            Reason::RegistryRevoked {
                sanction: op.operation,
                licence: term.licence.get(),
                generation: term.generation.get(),
                term_digest: row.owner_term.digest,
            },
            None,
            ctx,
            f,
        )
        .map(Some);
    }
    let now = world.world_ic_now();
    if now.clock != row.accepted_at_ic.clock {
        return Intent::new(row, actor, Reason::ClockChanged, None, ctx, f).map(Some);
    }
    if now.at < row.accepted_at_ic.at {
        return Err(PublicationError::Unavailable);
    }
    if !receivers(world, row.economic.consent())? {
        return Intent::new(row, actor, Reason::RecipientUnavailable, None, ctx, f).map(Some);
    }
    let claims = roll.charter_pending_claims(row.charter).map_err(bad)?;
    if claims.next_transition < row.claim_observation.next_transition {
        return Err(PublicationError::Mismatch);
    }
    let reason = if claims.open_owned == 0 {
        Reason::LastClaimClosed {
            next_transition: claims.next_transition,
        }
    } else if row.economic.consent().deadline_reached(&now).map_err(bad)? {
        Reason::DeadlineReached {
            next_transition: claims.next_transition,
            open_owned: claims.open_owned,
        }
    } else {
        return Ok(None);
    };
    Intent::new(row, actor, reason, Some(claims), ctx, f).map(Some)
}

#[derive(Clone)]
struct Service {
    runtime: Runtime,
    ready: Readiness,
    f: CommerceFamilies,
    config: PendingCharterTransferConfig,
    continuation: ContinuationHandle,
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
        f: service.f.clone(),
    }
    .is_current(ctx, guard)
}
struct Guard {
    runtime: Runtime,
    ready: Readiness,
    f: CommerceFamilies,
}
impl AuthorityGuardValidator for Guard {
    fn is_current(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        if self.ready.check().is_err() {
            return Ok(false);
        }
        Ok(self.runtime.projected().is_some_and(|i| {
            i.guard().ok().as_ref() == Some(guard) && i.fresh_guard(ctx, &self.f).is_ok()
        }))
    }
}
fn installed(
    ctx: &impl CommerceRead,
    service: &Service,
    intent: &Intent,
) -> Result<bool, PublicationError> {
    let row = ctx
        .get(&service.f.charter_pending, &intent.funding.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    let offer = ctx
        .get(&service.f.charter_pending_offers, &row.offer.to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    row.matches_offer(offer)?;
    let native = ctx
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if native
        .pending_charter_history(intent.funding)
        .map_err(bad)?
        != &row.economic
    {
        return Err(PublicationError::Mismatch);
    }
    if matches!(row.state, PendingState::Funded {}) {
        if row.reference()? != intent.pending {
            return Err(PublicationError::Mismatch);
        }
        return Ok(false);
    }
    // A different authentic terminal (e.g. concurrent Registry refund) also
    // completes custody; it must have its exact paired root, not our guessed ID.
    let roll = ctx
        .world()
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.check_required_root(&history::terminal_root(row, offer)?)
        .map_err(bad)?;
    Ok(true)
}
fn should_clear_pending(disposition: SubmissionDisposition, still_fresh: bool) -> bool {
    disposition == SubmissionDisposition::NoIntent || !still_fresh
}

fn tick(ctx: &mut Ctx<'_>, service: &Service) -> Result<(), PublicationError> {
    service.ready.check()?;
    if !live(ctx.w, service.config.service_actor) {
        return Err(PublicationError::Unavailable);
    }
    let mut state = service.runtime.0.lock().map_err(bad)?;
    if let Some(p) = state.pending.as_mut() {
        if installed(ctx, service, &p.intent)? {
            state.pending = None;
        } else if let Some(disposition) = p
            .observation
            .as_mut()
            .and_then(SubmissionObservation::take_terminal)
        {
            p.observation = None;
            if should_clear_pending(disposition, p.intent.fresh_read(ctx, &service.f).is_ok()) {
                state.pending = None;
            }
        } else if p.observation.is_none() && p.intent.fresh_read(ctx, &service.f).is_err() {
            state.pending = None;
        }
    }
    if state.pending.is_none() {
        let page = ctx
            .scan_action_state(
                &service.f.charter_pending_heads,
                state.after.as_deref(),
                PAGE,
            )
            .map_err(bad)?;
        let candidates = page
            .entries()
            .iter()
            .map(|e| (e.encoded_key().to_string(), e.value().clone()))
            .collect::<Vec<_>>();
        let next = page.next_after().map(ToString::to_string);
        // At most one full native history/proof per tick. Move after every
        // examined head so a funded but not-yet-due charter cannot starve peers.
        state.after = next;
        for (key, head) in candidates {
            head.validate()?;
            if !matches!(head.state, PendingState::Funded {}) {
                continue;
            }
            state.after = Some(key);
            let row = ctx
                .action_state(&service.f.charter_pending, &head.current.operation.to_hex())
                .map_err(bad)?
                .ok_or(PublicationError::Missing)?;
            head.matches(row)?;
            if let Some(intent) = discover(ctx, &service.f, row, service.config.service_actor)? {
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
    if schedule && !ctx.authoritative_pending(service.config.service_actor) {
        ctx.schedule_continuation(&service.continuation, service.config.service_actor, "", 0)
            .map_err(bad)?;
    }
    Ok(())
}
fn unavailable() -> CmdError {
    CmdError::msg("Charter settlement processing is unavailable.")
}
fn drive(ctx: &mut Ctx<'_>, service: &Service) -> Result<(), CmdError> {
    let actor = service.config.service_actor;
    if ctx.dispatch_actor() != Some(actor) || ctx.actor != actor || !ctx.args.raw().is_empty() {
        return Err(unavailable());
    }
    service.ready.check().map_err(|_| unavailable())?;
    let mut state = service.runtime.0.lock().map_err(|_| unavailable())?;
    let p = state.pending.as_mut().ok_or_else(unavailable)?;
    if p.observation.is_some() || ctx.authoritative_pending(actor) {
        return Ok(());
    }
    if p.intent.fresh_read(ctx, &service.f).is_err() {
        state.pending = None;
        return Err(unavailable());
    }
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
    reg: &mut Registrar<'_>,
    runtime: Runtime,
    ready: Readiness,
    f: CommerceFamilies,
    config: PendingCharterTransferConfig,
) -> Result<(), ModError> {
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: COMMAND.into(),
            switches: vec!["continue".into()],
            shape: ArgShape::Raw,
            help: "Internal bounded charter escrow processing.".into(),
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
        config,
        continuation,
    };
    reg.state(service.clone());
    reg.tick(
        "personal_charter_pending",
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            move |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx, &service);
            },
        ),
    )
}

#[cfg(test)]
mod high20_tests {
    use super::*;
    use crate::commerce::registry_inspection::principal::{
        RevokedPrincipal, any_party_barred_engine, any_party_barred_read,
    };
    use mush_core::submission_observation::{
        SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
    };
    use mush_core::{AuthoritativeSubmit, GameModule};
    use std::sync::atomic::{AtomicBool, Ordering};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    struct ProbeReader(Arc<AtomicBool>);
    impl SubmissionDispositionReader for ProbeReader {
        fn no_intent(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }
    struct ProbeRuntime(Arc<AtomicBool>);
    impl SubmissionRuntime for ProbeRuntime {
        fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
            SubmissionRuntimeState::new(Arc::new(ProbeReader(self.0.clone())), Box::new(()))
        }
    }

    struct ProbeMod {
        families: Arc<Mutex<Option<CommerceFamilies>>>,
        read_barred: Arc<Mutex<Vec<bool>>>,
        obs_slot: Arc<Mutex<Option<SubmissionObservation>>>,
    }
    impl GameModule for ProbeMod {
        fn name(&self) -> &'static str {
            crate::NS
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let f = CommerceFamilies::register(reg)?;
            *self.families.lock().map_err(|_| ModError::msg("lock"))? = Some(f.clone());
            let out = self.read_barred.clone();
            let obs_slot = self.obs_slot.clone();
            let f_cmd = f;
            reg.classify_commands(CommandCompatibility::Authoritative);
            reg.command(
                CommandSpec {
                    name: "@probe_cp_high20".into(),
                    shape: ArgShape::Raw,
                    help: "test probe".into(),
                    ..Default::default()
                },
                Arc::new(move |ctx: &mut Ctx<'_>| {
                    let barred = any_party_barred_read(ctx, &f_cmd, &[ObjId(10), ObjId(11)])
                        .map_err(|_| unavailable())?;
                    let peer_barred = any_party_barred_read(ctx, &f_cmd, &[ObjId(12), ObjId(13)])
                        .map_err(|_| unavailable())?;
                    out.lock()
                        .map_err(|_| unavailable())?
                        .extend([barred, peer_barred]);
                    if ctx.args.raw() == "submit" {
                        let action = protocol::settle_action().map_err(|_| unavailable())?;
                        let key = IdempotencyKeyText::parse_canonical("high20_cp_probe")
                            .map_err(|_| unavailable())?;
                        let obs = ctx.submit_authoritative_observed(
                            action,
                            PerceivedRef::SelfActor,
                            vec![],
                            serde_json::json!({}),
                            key,
                        )?;
                        *obs_slot.lock().map_err(|_| unavailable())? = Some(obs);
                    }
                    Ok(())
                }),
            )?;
            Ok(())
        }
    }

    #[test]
    fn high20_charter_pending_skips_barred_party_and_clears_pending_on_no_intent() -> TestResult {
        let families = Arc::new(Mutex::new(None));
        let read_barred = Arc::new(Mutex::new(Vec::new()));
        let obs_slot = Arc::new(Mutex::new(None));
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(ProbeMod {
            families: families.clone(),
            read_barred: read_barred.clone(),
            obs_slot: obs_slot.clone(),
        })]);
        let no_intent_flag = Arc::new(AtomicBool::new(true));
        tw.engine_mut()
            .set_submission_runtime(Arc::new(ProbeRuntime(no_intent_flag)))?;
        let f = families
            .lock()
            .map_err(|_| "lock")?
            .clone()
            .ok_or("families")?;
        tw.world_mut().set_game_clock(mush_core::GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        });
        tw.world_mut().set_now(500);

        let seller = ObjId(10);
        let buyer = ObjId(11);
        let peer_seller = ObjId(12);
        let peer_buyer = ObjId(13);
        let bar = RevokedPrincipal {
            format: 1,
            principal: mush_org::PrincipalId::Object(seller),
            from_ic: 100,
            until_ic: 1_000,
            inspection: CommitToken::from_bytes([1; 16]),
            hull: ObjId(20),
            seq: 1,
            operation: CommitToken::from_bytes([2; 16]),
            operator: ObjId(1),
        };
        {
            let mut txn = tw.engine_mut().action_transaction(&f.revoked_principals)?;
            txn.insert(&f.revoked_principals, &bar.key(), bar)?;
            let (_, install) = txn.prepare()?.into_parts();
            tw.engine_mut().install_prepared_action(install)?;
        }

        // 1. Both engine-level and Ctx/CommerceRead-level bar checks detect barred seller
        //    at now=500 while leaving peer escrows unbarred.
        check!(any_party_barred_engine(tw.engine(), &f, &[seller, buyer])?);
        check!(!any_party_barred_engine(
            tw.engine(),
            &f,
            &[peer_seller, peer_buyer]
        )?);
        let _ = tw.run(mush_core::GOD, "@probe_cp_high20 submit");
        check_eq!(
            *read_barred.lock().map_err(|_| "lock")?,
            vec![true, false],
            "CommerceRead must detect barred seller while peer parties stay unbarred"
        );

        // 2. Drive the observed submission into `SubmissionDisposition::NoIntent` and
        //    verify `should_clear_pending` clears `state.pending` unconditionally.
        let driven = tw
            .engine_mut()
            .take_observed_authoritative_submit()
            .ok_or("driven submission")?;
        let mut obs = obs_slot
            .lock()
            .map_err(|_| "lock")?
            .take()
            .ok_or("observation")?;
        let disposition = obs.take_terminal().ok_or("NoIntent disposition")?;
        check_eq!(disposition, SubmissionDisposition::NoIntent);
        check!(should_clear_pending(disposition, true));
        check!(should_clear_pending(disposition, false));
        drop(driven);

        // 3. Undriven attempt (`NotDispatched`) retains a still-fresh intent for retry,
        //    and clears an intent that became barred/stale.
        check!(!should_clear_pending(
            SubmissionDisposition::NotDispatched,
            true
        ));
        check!(should_clear_pending(
            SubmissionDisposition::NotDispatched,
            false
        ));

        // 4. Once the bar expires (now=1000 == until_ic), both engine and CommerceRead
        //    checks clear cleanly so the row can resume normal settlement.
        tw.world_mut().set_now(1_000);
        check!(!any_party_barred_engine(tw.engine(), &f, &[seller, buyer])?);
        read_barred.lock().map_err(|_| "lock")?.clear();
        let _ = tw.run(mush_core::GOD, "@probe_cp_high20");
        check_eq!(*read_barred.lock().map_err(|_| "lock")?, vec![false, false]);
        Ok(())
    }
}
