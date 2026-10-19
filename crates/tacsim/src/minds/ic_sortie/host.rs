//! Bounded retention of genuine host capabilities. A wire ticket is only a
//! lookup; it cannot mint a start, a terminal event, or a successful receipt.
use super::participant::{CloseEvent, StartPermit};
use super::records::bad;
use mush_core::participant::ParticipantError;
use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_core::{Ctx, IdempotencyKeyText, PerceivedRef};
use mush_roll::commerce::{LicenceId, ic_claims::IcWriter};
use smol_str::SmolStr;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(super) mod coverage;

const MAX_PENDING: usize = 1024;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct HostStart {
    pub(super) request: DefinitionDigest,
    pub(super) permit: Arc<StartPermit>,
    pub(super) withheld: Option<Arc<coverage::WithheldDeparture>>,
}
#[derive(Clone)]
pub struct HostClose {
    pub(super) request: DefinitionDigest,
    pub(super) permit: Arc<CloseEvent>,
}
#[derive(Clone)]
pub(super) enum Intent {
    Open {
        start: HostStart,
        licence: LicenceId,
        profiles: Vec<SmolStr>,
    },
    Close(HostClose),
}
impl Intent {
    pub(super) fn actor(&self) -> ObjId {
        match self {
            Self::Open { start, .. } => start.permit.actor,
            Self::Close(c) => c.permit.actor,
        }
    }
    pub(super) fn closing(&self) -> bool {
        matches!(self, Self::Close(_))
    }
    pub(super) fn request(&self) -> DefinitionDigest {
        match self {
            Self::Open { start, .. } => start.request,
            Self::Close(c) => c.request,
        }
    }
}
enum Attempt {
    Unsubmitted,
    Observed(SubmissionObservation),
    Retryable,
}
impl Attempt {
    fn poll(&mut self) {
        if let Self::Observed(observation) = self {
            match observation.take_terminal() {
                Some(SubmissionDisposition::NotDispatched | SubmissionDisposition::NoIntent) => {
                    *self = Self::Retryable;
                }
                None => {}
            }
        }
    }
    fn status(&self) -> IcSubmissionState {
        match self {
            Self::Unsubmitted => IcSubmissionState::Unsubmitted,
            Self::Observed(_) => IcSubmissionState::AwaitingAcknowledgement,
            Self::Retryable => IcSubmissionState::Retryable,
        }
    }
}
struct Pending {
    intent: Intent,
    bytes: usize,
    attempt: Attempt,
}
/// A volatile attempt status, never proof that an IC claim opened or closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IcSubmissionState {
    Unsubmitted,
    AwaitingAcknowledgement,
    Retryable,
}
#[derive(Default)]
struct Inbox {
    rows: BTreeMap<DefinitionDigest, Pending>,
    bytes: usize,
}

/// No actor override, event decoder or success setter is exposed. The absent
/// region host must produce an actual StartPermit/CloseEvent before enqueueing.
#[derive(Clone)]
pub struct IcHost {
    inbox: Arc<Mutex<Inbox>>,
    writer: IcWriter,
    actions: [DefinitionRef; 2],
}
/// Source-free selector to use through the ordinary signed self-action route.
/// Its data conveys no authority when the actual private capability is absent.
pub struct IcHostTicket {
    request: DefinitionDigest,
    actor: ObjId,
    action: DefinitionRef,
}
impl IcHostTicket {
    pub fn actor(&self) -> ObjId {
        self.actor
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"ticket":self.request})
    }
}
/// The original Open capability returned after definitive non-dispatch and
/// actual audited absence. This is neither a receipt nor a fresh start permit.
/// It is not cloneable/serializable and can be restored only once to its host.
pub struct IcUnadmittedOpen {
    origin: Arc<Mutex<Inbox>>,
    ticket: IcHostTicket,
    permit: Arc<StartPermit>,
    pending: Option<Pending>,
}
impl IcUnadmittedOpen {
    pub fn ticket(&self) -> &IcHostTicket {
        &self.ticket
    }
    pub fn permit(&self) -> &Arc<StartPermit> {
        &self.permit
    }
}
/// Actual installed-owner readback, not a capability accepted by any producer.
pub struct IcHostCompletion {
    open: CommitToken,
    closed: Option<CommitToken>,
}
impl IcHostCompletion {
    pub fn open(&self) -> CommitToken {
        self.open
    }
    pub fn closed(&self) -> Option<CommitToken> {
        self.closed
    }
}
/// Source-private executable material from an actually acknowledged active
/// claim. It is not a StartPermit, a close event, or a physical owner write.
/// Installing on a runtime rechecks the same active claim, never live rights.
pub struct IcRetainedExecution {
    writer: IcWriter,
    record: Arc<super::records::Record>,
}
impl IcRetainedExecution {
    pub fn open(&self) -> CommitToken {
        self.record.open.operation
    }
    pub fn seal(&self) -> &crate::arena::MindSeal {
        &self.record.open.execution.seal
    }
    pub fn install(
        &self,
        world: &World,
        arena: &mut crate::arena::Arena,
    ) -> Result<(), ParticipantError> {
        let state = audited_state(world, &self.writer)?;
        let actual = state
            .rows
            .get(&self.open())
            .ok_or_else(|| bad("retained open"))?;
        if actual.closed.is_some()
            || actual.open != self.record.open
            || state.active.get(&self.seal().unit) != Some(&self.open())
        {
            return Err(bad("closed or crossed retained execution"));
        }
        self.record.open.execution.install(arena)
    }
}
fn audited_state<'a>(
    world: &'a World,
    writer: &IcWriter,
) -> Result<&'a super::IcSorties, ParticipantError> {
    let expected = writer.require_audited(world).map_err(bad)?;
    if world
        .module_state_owner(std::any::TypeId::of::<super::IcSorties>())
        .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Err(ParticipantError::Authority);
    }
    let state = world
        .module_state::<super::IcSorties>()
        .ok_or_else(|| bad("owner"))?;
    state.ready()?;
    if state.next_transition != expected {
        return Err(bad("paired head"));
    }
    Ok(state)
}
impl IcHost {
    /// Restart uses the immutable bytes retained by the actual four-owner Open.
    /// Expiry, issuer lapse and changed current holders do not rewrite a sortie.
    /// Actual Close still ends its active authority and prevents reinstallation.
    pub fn resume(
        &self,
        world: &World,
        open: CommitToken,
    ) -> Result<IcRetainedExecution, ParticipantError> {
        let state = audited_state(world, &self.writer)?;
        let record = state.rows.get(&open).ok_or_else(|| bad("retained open"))?;
        if record.closed.is_some()
            || state.active.get(&record.open.execution.seal.unit) != Some(&open)
        {
            return Err(bad("inactive retained execution"));
        }
        record.open.execution.validate()?;
        Ok(IcRetainedExecution {
            writer: self.writer.clone(),
            record: Arc::clone(record),
        })
    }
    pub(super) fn new(writer: IcWriter, actions: [DefinitionRef; 2]) -> Self {
        Self {
            inbox: Arc::new(Mutex::new(Inbox::default())),
            writer,
            actions,
        }
    }
    fn insert(&self, intent: Intent, bytes: usize) -> Result<IcHostTicket, ParticipantError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let total = inbox.bytes.checked_add(bytes).ok_or_else(|| bad("bytes"))?;
        if bytes > super::protocol::MAX_RECORD_BYTES
            || total > MAX_BYTES
            || inbox.rows.len() >= MAX_PENDING
            || inbox.rows.contains_key(&intent.request())
            || inbox.rows.values().any(|p| {
                p.intent.actor() == intent.actor() && p.intent.closing() == intent.closing()
            })
        {
            return Err(bad("host pending capacity or duplicate"));
        }
        let ticket = IcHostTicket {
            request: intent.request(),
            actor: intent.actor(),
            action: self
                .actions
                .get(usize::from(intent.closing()))
                .ok_or_else(|| bad("action"))?
                .clone(),
        };
        inbox.rows.insert(
            intent.request(),
            Pending {
                intent,
                bytes,
                attempt: Attempt::Unsubmitted,
            },
        );
        inbox.bytes = total;
        Ok(ticket)
    }
    pub fn enqueue_start(
        &self,
        permit: &Arc<StartPermit>,
        licence: LicenceId,
        profiles: Vec<SmolStr>,
    ) -> Result<IcHostTicket, ParticipantError> {
        if profiles.len() > 8 || profiles.iter().any(|p| p.is_empty() || p.len() > 64) {
            return Err(bad("profiles"));
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(
            &serde_json::json!({"unit":permit.unit,"licence":licence,"profiles":profiles}),
            super::protocol::MAX_RECORD_BYTES,
        )
        .map_err(bad)?
        .len();
        let request = DefinitionDigest::from_bytes(rand::random());
        self.insert(
            Intent::Open {
                start: HostStart {
                    request,
                    permit: Arc::clone(permit),
                    withheld: None,
                },
                licence,
                profiles,
            },
            bytes,
        )
    }
    pub fn enqueue_close(
        &self,
        permit: &Arc<CloseEvent>,
    ) -> Result<IcHostTicket, ParticipantError> {
        let bytes = permit.retained_bytes()?;
        let request = DefinitionDigest::from_bytes(rand::random());
        self.insert(
            Intent::Close(HostClose {
                request,
                permit: Arc::clone(permit),
            }),
            bytes,
        )
    }
    /// Dispatch this exact retained ticket through the normal signed selector.
    /// Core stamps the real ingress actor; changing Ctx.actor cannot select an
    /// alternative principal. A missing reply keeps the same opaque observation.
    /// Only a consumed definitive disposition permits another identical attempt.
    /// The host must call this from real actor dispatch, never a fabricated Ctx.
    pub fn submit(
        &self,
        ctx: &mut Ctx<'_>,
        ticket: &IcHostTicket,
    ) -> Result<IcSubmissionState, ParticipantError> {
        if ctx.actor != ticket.actor || !self.ready(ctx.w) {
            return Err(ParticipantError::Authority);
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let pending = inbox
            .rows
            .get_mut(&ticket.request)
            .ok_or_else(|| bad("pending"))?;
        self.check_ticket(pending, ticket)?;
        pending.attempt.poll();
        if matches!(pending.attempt, Attempt::Observed(_))
            || ctx.authoritative_pending(ticket.actor)
            || ctx.authoritative_commit_pending()
            || ctx.authoritative_queue_full()
        {
            return Ok(pending.attempt.status());
        }
        let key = IdempotencyKeyText::parse_canonical(format!("ic.{}", ticket.request.to_hex()))
            .map_err(bad)?;
        let observed = ctx
            .submit_authoritative_observed(
                ticket.action.clone(),
                PerceivedRef::SelfActor,
                vec![],
                ticket.parameters(),
                key,
            )
            .map_err(bad)?;
        pending.attempt = Attempt::Observed(observed);
        Ok(pending.attempt.status())
    }
    /// No timeout cleanup. A definitive disposition permits retry of the same
    /// ticket, or explicit return of an unadmitted Open after audited absence.
    /// Missing responses and every Close keep their original capabilities.
    pub fn submission_state(
        &self,
        ticket: &IcHostTicket,
    ) -> Result<IcSubmissionState, ParticipantError> {
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let pending = inbox
            .rows
            .get_mut(&ticket.request)
            .ok_or_else(|| bad("pending"))?;
        self.check_ticket(pending, ticket)?;
        pending.attempt.poll();
        Ok(pending.attempt.status())
    }
    /// Return a definitively unadmitted Open to its actual host. A consumed
    /// NoIntent/NotDispatched observation alone is insufficient: complete owner
    /// readback must also contain no use of this exact request. There is no
    /// await, callback or mutable World access between that check and removal.
    /// Installed/readback disagreement therefore retains the pending capability.
    pub fn discard_unadmitted_open(
        &self,
        world: &World,
        ticket: &IcHostTicket,
    ) -> Result<IcUnadmittedOpen, ParticipantError> {
        let state = audited_state(world, &self.writer)?;
        if state.request_used(ticket.request) {
            return Err(bad("request has actual installed history"));
        }
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let pending = inbox
            .rows
            .get_mut(&ticket.request)
            .ok_or_else(|| bad("pending"))?;
        self.check_ticket(pending, ticket)?;
        pending.attempt.poll();
        if !matches!(pending.attempt, Attempt::Retryable) {
            return Err(bad("no definitive non-dispatch observation"));
        }
        let Intent::Open { start, .. } = &pending.intent else {
            return Err(bad("Close capability must remain retained"));
        };
        let permit = Arc::clone(&start.permit);
        let bytes = pending.bytes;
        let remaining = inbox.bytes.checked_sub(bytes).ok_or_else(|| bad("bytes"))?;
        let pending = inbox
            .rows
            .remove(&ticket.request)
            .ok_or_else(|| bad("pending"))?;
        inbox.bytes = remaining;
        Ok(IcUnadmittedOpen {
            origin: Arc::clone(&self.inbox),
            ticket: IcHostTicket {
                request: ticket.request,
                actor: ticket.actor,
                action: ticket.action.clone(),
            },
            permit,
            pending: Some(pending),
        })
    }
    /// Restore that same capability, ticket, parameters and idempotency key.
    /// Capacity or readiness refusal leaves the capsule with its caller. No new
    /// permit or request identity is minted; a consumed capsule cannot reenter.
    pub fn restore_unadmitted_open(
        &self,
        world: &World,
        released: &mut IcUnadmittedOpen,
    ) -> Result<IcHostTicket, ParticipantError> {
        if !Arc::ptr_eq(&self.inbox, &released.origin) {
            return Err(ParticipantError::Authority);
        }
        let state = audited_state(world, &self.writer)?;
        if state.request_used(released.ticket.request) {
            return Err(bad("request has actual installed history"));
        }
        let pending = released
            .pending
            .as_ref()
            .ok_or_else(|| bad("already restored"))?;
        self.check_ticket(pending, &released.ticket)?;
        let ticket = self.insert(pending.intent.clone(), pending.bytes)?;
        released.pending.take();
        Ok(ticket)
    }
    fn check_ticket(
        &self,
        pending: &Pending,
        ticket: &IcHostTicket,
    ) -> Result<(), ParticipantError> {
        if pending.intent.request() != ticket.request
            || pending.intent.actor() != ticket.actor
            || self.actions.get(usize::from(pending.intent.closing())) != Some(&ticket.action)
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(super) fn ready(&self, world: &World) -> bool {
        self.writer
            .require_audited(world)
            .ok()
            .is_some_and(|expected| {
                world
                    .module_state_owner(std::any::TypeId::of::<super::IcSorties>())
                    .is_some_and(|o| o.as_str() == crate::NS)
                    && world
                        .module_state::<super::IcSorties>()
                        .is_some_and(|s| s.loaded && s.next_transition == expected)
            })
    }
    pub(super) fn selected(
        &self,
        actor: ObjId,
        closing: bool,
    ) -> Result<Option<Intent>, ParticipantError> {
        let inbox = self.inbox.lock().map_err(bad)?;
        Ok(inbox
            .rows
            .values()
            .find(|p| p.intent.actor() == actor && p.intent.closing() == closing)
            .map(|p| p.intent.clone()))
    }
    pub(super) fn lookup(
        &self,
        request: DefinitionDigest,
        actor: ObjId,
        closing: bool,
    ) -> Result<Intent, ParticipantError> {
        let inbox = self.inbox.lock().map_err(bad)?;
        let p = inbox
            .rows
            .get(&request)
            .ok_or_else(|| bad("missing actual host capability"))?;
        if p.intent.actor() != actor || p.intent.closing() != closing {
            return Err(ParticipantError::Authority);
        }
        Ok(p.intent.clone())
    }
    /// Missing/Unknown acknowledgement retains the same bounded entry. Only
    /// exact installed four-owner readback releases it; elapsed time never does.
    pub fn acknowledge(
        &self,
        world: &World,
        ticket: &IcHostTicket,
    ) -> Result<Option<IcHostCompletion>, ParticipantError> {
        let state = audited_state(world, &self.writer)?;
        let mut inbox = self.inbox.lock().map_err(bad)?;
        let pending = inbox
            .rows
            .get(&ticket.request)
            .ok_or_else(|| bad("pending"))?;
        self.check_ticket(pending, ticket)?;
        let completion = state.rows.values().find_map(|r| {
            if r.open.request == ticket.request && !pending.intent.closing() {
                Some(IcHostCompletion {
                    open: r.open.operation,
                    closed: None,
                })
            } else {
                r.closed
                    .as_ref()
                    .filter(|c| c.request == ticket.request && pending.intent.closing())
                    .map(|c| IcHostCompletion {
                        open: r.open.operation,
                        closed: Some(c.operation),
                    })
            }
        });
        if completion.is_some() {
            let removed = inbox
                .rows
                .remove(&ticket.request)
                .ok_or_else(|| bad("pending"))?;
            inbox.bytes = inbox
                .bytes
                .checked_sub(removed.bytes)
                .ok_or_else(|| bad("bytes"))?;
        }
        Ok(completion)
    }
}
