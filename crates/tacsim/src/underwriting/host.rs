//! Bounded private host intents. Wire selectors cannot create a departure,
//! quote authority, loss, successful submission or policy acknowledgement.
use super::{
    CoverageError, audit, authority,
    families::{Families, Row},
    metadata::Publications,
    policy::{Policy, Terminal},
    protocol::Protocol,
    records::Quote,
};
use crate::minds::ic_sortie::{AcknowledgedDeparture, CommittedCoverageClose, IcHost};
use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};
use mush_core::{
    CommitToken, Ctx, DefinitionDigest, DefinitionRef, IdempotencyKeyText, ObjId, PerceivedRef,
    World,
};
use mush_economy::{AccountId, Amount};
use mush_roll::commerce::coverage::CoverageWriter;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
fn bad(_: impl std::fmt::Display) -> CoverageError {
    CoverageError::Authority
}
#[derive(Clone)]
pub(super) enum Intent {
    Quote {
        departure: Arc<AcknowledgedDeparture>,
        publication: CommitToken,
        payer: AccountId,
        value: Amount,
    },
    Bind {
        departure: Arc<AcknowledgedDeparture>,
        quote: CommitToken,
    },
    Terminal {
        close: Arc<CommittedCoverageClose>,
        policy: CommitToken,
        cancel: bool,
    },
}
enum Attempt {
    New,
    Observed(SubmissionObservation),
    Retryable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageSubmissionState {
    Unsubmitted,
    AwaitingAcknowledgement,
    Retryable,
}
impl Attempt {
    fn poll(&mut self) {
        if let Self::Observed(o) = self {
            if matches!(
                o.take_terminal(),
                Some(SubmissionDisposition::NotDispatched | SubmissionDisposition::NoIntent)
            ) {
                *self = Self::Retryable;
            }
        }
    }
    fn status(&self) -> CoverageSubmissionState {
        match self {
            Self::New => CoverageSubmissionState::Unsubmitted,
            Self::Observed(_) => CoverageSubmissionState::AwaitingAcknowledgement,
            Self::Retryable => CoverageSubmissionState::Retryable,
        }
    }
}
struct Pending {
    actor: ObjId,
    index: usize,
    intent: Intent,
    attempt: Attempt,
}
#[derive(Clone)]
pub(super) struct Installed {
    pub(super) families: Families,
    pub(super) readiness: authority::Readiness,
}
#[derive(Clone)]
pub struct CoverageHost {
    pub(super) protocol: Protocol,
    pub(super) writer: CoverageWriter,
    pub(super) ic: IcHost,
    pub(super) service: ObjId,
    inbox: Arc<Mutex<BTreeMap<DefinitionDigest, Pending>>>,
    pub(super) installed: Arc<OnceLock<Installed>>,
}
pub struct CoverageTicket {
    request: DefinitionDigest,
    actor: ObjId,
    index: usize,
    action: DefinitionRef,
}
impl CoverageTicket {
    pub fn actor(&self) -> ObjId {
        self.actor
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    /// The public parameters to submit: none. The provider's offer carries
    /// this ticket as a fixed parameter, so resubmitting it would collide.
    pub fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
/// A readback result, not policy or host authority. Release consumes a separate
/// private value produced only by the same complete installed-policy audit.
pub struct CoverageCompletion {
    operation: CommitToken,
    kind: &'static str,
    departure: Option<Arc<AcknowledgedDeparture>>,
}
impl CoverageCompletion {
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn kind(&self) -> &str {
        self.kind
    }
}
pub struct CoverageUnadmitted {
    origin: Arc<Mutex<BTreeMap<DefinitionDigest, Pending>>>,
    ticket: CoverageTicket,
    pending: Option<Pending>,
}
impl CoverageUnadmitted {
    pub fn ticket(&self) -> &CoverageTicket {
        &self.ticket
    }
}
impl CoverageHost {
    pub(super) fn new(
        protocol: Protocol,
        writer: CoverageWriter,
        ic: IcHost,
        service: ObjId,
    ) -> Self {
        Self {
            protocol,
            writer,
            ic,
            service,
            inbox: Arc::new(Mutex::new(BTreeMap::new())),
            installed: Arc::new(OnceLock::new()),
        }
    }
    pub(super) fn ready(&self, world: &World, index: usize) -> Result<&Installed, CoverageError> {
        let r = self.installed.get().ok_or(CoverageError::Authority)?;
        r.readiness.check(&self.protocol.action(index)?)?;
        self.writer.require_audited(world).map_err(bad)?;
        if world
            .module_state_owner(std::any::TypeId::of::<Publications>())
            .is_none_or(|o| o.as_str() != crate::NS)
            || world
                .module_state::<Publications>()
                .is_none_or(|p| !p.loaded)
        {
            return Err(CoverageError::Authority);
        }
        Ok(r)
    }
    fn insert(
        &self,
        actor: ObjId,
        index: usize,
        intent: Intent,
    ) -> Result<CoverageTicket, CoverageError> {
        let mut pending = self.inbox.lock().map_err(bad)?;
        if pending.len() >= 128
            || pending
                .values()
                .any(|p| p.actor == actor && p.index == index)
        {
            return Err(CoverageError::Bound);
        }
        let request = DefinitionDigest::from_bytes(rand::random());
        if request.as_bytes() == &[0; 32] || pending.contains_key(&request) {
            return Err(CoverageError::Bound);
        }
        pending.insert(
            request,
            Pending {
                actor,
                index,
                intent,
                attempt: Attempt::New,
            },
        );
        Ok(CoverageTicket {
            request,
            actor,
            index,
            action: self.protocol.action(index)?,
        })
    }
    pub fn quote(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        publication: CommitToken,
        payer: AccountId,
        value: Amount,
    ) -> Result<CoverageTicket, CoverageError> {
        self.ready(world, 1)?;
        departure.recheck(world).map_err(bad)?;
        self.insert(
            departure.actor(),
            1,
            Intent::Quote {
                departure,
                publication,
                payer,
                value,
            },
        )
    }
    pub fn bind(
        &self,
        world: &World,
        departure: Arc<AcknowledgedDeparture>,
        quote: CommitToken,
    ) -> Result<CoverageTicket, CoverageError> {
        self.ready(world, 2)?;
        departure.recheck(world).map_err(bad)?;
        self.insert(departure.actor(), 2, Intent::Bind { departure, quote })
    }
    /// Selects only an already committed genuine covered Close. Expiry alone
    /// cannot make a terminal proof; a missing region event keeps collateral.
    pub fn terminal(
        &self,
        ctx: &Ctx<'_>,
        policy: CommitToken,
        cancel: bool,
    ) -> Result<CoverageTicket, CoverageError> {
        let r = self.ready(ctx.w, 4)?;
        let p = read(ctx, &r.families.policies, policy.to_hex())?;
        if p.history.terminal().is_some() {
            return Err(CoverageError::Authority);
        }
        let c = p.history.consent();
        let close = Arc::new(
            self.ic
                .committed_coverage_close(ctx.w, c.basis.ic_open, c.basis.departure_digest)
                .map_err(bad)?,
        );
        let proof = super::policy::CloseProof::capture(ctx.w, &close)?;
        let pays = proof.covered_loss(c)?;
        if cancel && pays {
            return Err(CoverageError::Authority);
        }
        let actor = if cancel { c.terms.buyer } else { self.service };
        let index = if pays { 3 } else { 4 };
        self.ready(ctx.w, index)?;
        self.insert(
            actor,
            index,
            Intent::Terminal {
                close,
                policy,
                cancel,
            },
        )
    }
    pub(super) fn lookup(
        &self,
        request: DefinitionDigest,
        actor: ObjId,
        index: usize,
    ) -> Result<Intent, CoverageError> {
        let inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox.get(&request).ok_or(CoverageError::Authority)?;
        if p.actor != actor || p.index != index {
            return Err(CoverageError::Authority);
        }
        Ok(p.intent.clone())
    }
    pub(super) fn selected(
        &self,
        actor: ObjId,
        index: usize,
    ) -> Result<Option<DefinitionDigest>, CoverageError> {
        Ok(self
            .inbox
            .lock()
            .map_err(bad)?
            .iter()
            .find_map(|(id, p)| (p.actor == actor && p.index == index).then_some(*id)))
    }
    fn check(&self, p: &Pending, t: &CoverageTicket) -> Result<(), CoverageError> {
        if p.actor != t.actor || p.index != t.index || self.protocol.action(p.index)? != t.action {
            return Err(CoverageError::Authority);
        }
        Ok(())
    }
    /// Rebind only the same actual retained withheld capability to an installed
    /// policy after restart/ACK loss. No second native Bind or new permit occurs.
    pub fn recover_binding(
        &self,
        ctx: &Ctx<'_>,
        departure: Arc<AcknowledgedDeparture>,
        binding: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        let r = self.ready(ctx.w, 2)?;
        audit::all(ctx, &r.families, Some(&self.writer), Some(self.service))?;
        departure.recheck(ctx.w).map_err(bad)?;
        let p = read(ctx, &r.families.policies, binding.to_hex())?;
        let c = p.history.consent();
        if p.history.terminal().is_some()
            || c.basis.ic_open != departure.open()
            || c.basis.departure_digest != departure.digest()
            || c.terms.buyer != departure.actor()
        {
            return Err(CoverageError::Authority);
        }
        Ok(CoverageCompletion {
            operation: binding,
            kind: "bound",
            departure: Some(departure),
        })
    }
    pub fn release_departure(
        &self,
        ctx: &Ctx<'_>,
        completion: &mut CoverageCompletion,
    ) -> Result<crate::minds::ic_sortie::CoveredDeparture, CoverageError> {
        let r = self.ready(ctx.w, 2)?;
        audit::all(ctx, &r.families, Some(&self.writer), Some(self.service))?;
        let departure = completion
            .departure
            .as_ref()
            .ok_or(CoverageError::Authority)?;
        let p = read(ctx, &r.families.policies, completion.operation.to_hex())?;
        let c = p.history.consent();
        let now = ctx.w.world_ic_now();
        if completion.kind != "bound"
            || p.history.terminal().is_some()
            || c.basis.ic_open != departure.open()
            || c.basis.departure_digest != departure.digest()
            || now.clock != c.terms.starts_at.clock
            || now.at < c.terms.starts_at.at
            || now.at >= c.terms.ends_at.at
        {
            return Err(CoverageError::Authority);
        }
        let released = departure
            .release_after_binding(ctx.w, completion.operation)
            .map_err(bad)?;
        completion.departure.take();
        Ok(released)
    }

    pub fn submit(
        &self,
        ctx: &mut Ctx<'_>,
        ticket: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        self.ready(ctx.w, ticket.index)?;
        if ctx.dispatch_actor() != Some(ticket.actor) || ctx.actor != ticket.actor {
            return Err(CoverageError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .get_mut(&ticket.request)
            .ok_or(CoverageError::Authority)?;
        self.check(p, ticket)?;
        p.attempt.poll();
        if matches!(p.attempt, Attempt::Observed(_))
            || ctx.authoritative_pending(ticket.actor)
            || ctx.authoritative_commit_pending()
            || ctx.authoritative_queue_full()
        {
            return Ok(p.attempt.status());
        }
        let key =
            IdempotencyKeyText::parse_canonical(format!("coverage.{}", ticket.request.to_hex()))
                .map_err(bad)?;
        p.attempt = Attempt::Observed(
            ctx.submit_authoritative_observed(
                ticket.action.clone(),
                PerceivedRef::SelfActor,
                vec![],
                ticket.parameters(),
                key,
            )
            .map_err(bad)?,
        );
        Ok(p.attempt.status())
    }
    pub fn submission_state(
        &self,
        t: &CoverageTicket,
    ) -> Result<CoverageSubmissionState, CoverageError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox.get_mut(&t.request).ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        p.attempt.poll();
        Ok(p.attempt.status())
    }
    /// Complete real inverse readback precedes removal; no response bit or
    /// disappearance can turn an attempt into an installed policy.
    pub fn acknowledge(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
        operation: CommitToken,
    ) -> Result<CoverageCompletion, CoverageError> {
        let r = self.ready(ctx.w, t.index)?;
        audit::all(ctx, &r.families, Some(&self.writer), Some(self.service))?;
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox.get(&t.request).ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        let (kind, departure) = match &p.intent {
            Intent::Quote {
                departure,
                publication,
                payer,
                value,
            } => {
                let q: &Quote = read(ctx, &r.families.quotes, operation.to_hex())?;
                if q.request != t.request
                    || q.risk.ic_open != departure.open()
                    || q.risk.departure_digest != departure.digest()
                    || q.publication != *publication
                    || q.terms.payer != *payer
                    || q.terms.insured_value != *value
                {
                    return Err(CoverageError::Authority);
                }
                ("quoted", None)
            }
            Intent::Bind { departure, quote } => {
                let policy: &Policy = read(ctx, &r.families.policies, operation.to_hex())?;
                if policy.request != t.request
                    || policy.quote != *quote
                    || policy.history.consent().basis.departure_digest != departure.digest()
                    || policy.history.consent().basis.ic_open != departure.open()
                {
                    return Err(CoverageError::Authority);
                }
                ("bound", Some(Arc::clone(departure)))
            }
            Intent::Terminal {
                close,
                policy,
                cancel,
            } => {
                let terminal: &Terminal = read(ctx, &r.families.terminals, policy.to_hex())?;
                let actual = read(ctx, &r.families.policies, policy.to_hex())?;
                let reason = &actual
                    .history
                    .terminal()
                    .ok_or(CoverageError::Authority)?
                    .decision()
                    .reason;
                if terminal.operation != operation
                    || terminal.request != t.request
                    || terminal.policy != *policy
                    || terminal.close.close != close.operation().map_err(bad)?
                    || *cancel
                        != matches!(
                            reason,
                            mush_economy::managed_coverage::CoverageDecisionKindV1::Cancelled { .. }
                        )
                {
                    return Err(CoverageError::Authority);
                }
                (if t.index == 3 { "claimed" } else { "released" }, None)
            }
        };
        inbox.remove(&t.request).ok_or(CoverageError::Authority)?;
        Ok(CoverageCompletion {
            operation,
            kind,
            departure,
        })
    }
    pub fn discard_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        t: &CoverageTicket,
    ) -> Result<CoverageUnadmitted, CoverageError> {
        if t.index > 2 {
            return Err(CoverageError::Authority);
        }
        let r = self.ready(ctx.w, t.index)?;
        audit::all(ctx, &r.families, Some(&self.writer), Some(self.service))?;
        if audit::request_used(ctx, &r.families, t.request)? {
            return Err(CoverageError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox.get_mut(&t.request).ok_or(CoverageError::Authority)?;
        self.check(p, t)?;
        p.attempt.poll();
        if !matches!(p.attempt, Attempt::Retryable) {
            return Err(CoverageError::Authority);
        }
        let pending = inbox.remove(&t.request).ok_or(CoverageError::Authority)?;
        Ok(CoverageUnadmitted {
            origin: Arc::clone(&self.inbox),
            ticket: CoverageTicket {
                request: t.request,
                actor: t.actor,
                index: t.index,
                action: t.action.clone(),
            },
            pending: Some(pending),
        })
    }
    pub fn restore_unadmitted(
        &self,
        ctx: &Ctx<'_>,
        released: &mut CoverageUnadmitted,
    ) -> Result<CoverageTicket, CoverageError> {
        if !Arc::ptr_eq(&self.inbox, &released.origin) {
            return Err(CoverageError::Authority);
        }
        let t = &released.ticket;
        let r = self.ready(ctx.w, t.index)?;
        audit::all(ctx, &r.families, Some(&self.writer), Some(self.service))?;
        if audit::request_used(ctx, &r.families, t.request)? {
            return Err(CoverageError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        if inbox.len() >= 128
            || inbox.contains_key(&t.request)
            || inbox
                .values()
                .any(|p| p.actor == t.actor && p.index == t.index)
        {
            return Err(CoverageError::Bound);
        }
        self.check(
            released.pending.as_ref().ok_or(CoverageError::Authority)?,
            t,
        )?;
        let p = released.pending.take().ok_or(CoverageError::Authority)?;
        inbox.insert(t.request, p);
        Ok(CoverageTicket {
            request: t.request,
            actor: t.actor,
            index: t.index,
            action: t.action.clone(),
        })
    }
}
pub(super) fn read<'a, R: Row>(
    ctx: &'a Ctx<'_>,
    f: &mush_core::StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, CoverageError> {
    let row = ctx
        .action_state(f, &key)
        .map_err(bad)?
        .ok_or(CoverageError::Product)?;
    row.validate()?;
    if row.key() != key {
        return Err(CoverageError::Product);
    }
    Ok(row)
}
