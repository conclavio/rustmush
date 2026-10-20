//! A genuine withheld departure is distinct from permission to start or resume.
//! No decoder, public constructor or ordinary Open acknowledgement creates it.
use super::*;
use crate::minds::ic_sortie::IcSorties;
use crate::minds::ic_sortie::participant::HostTransition;
use crate::minds::ic_sortie::records::Record;
use serde::{Deserialize, Serialize};

/// Retained data produced by the private capability. It cannot reconstruct one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::minds::ic_sortie) struct WithheldCapture {
    format: u16,
    transition: DefinitionDigest,
    departure: DefinitionDigest,
    zone: DefinitionRef,
}
impl WithheldCapture {
    pub(in crate::minds::ic_sortie) fn validate(
        &self,
        unit: &mush_tactics::TacticalUnit,
        actor: ObjId,
    ) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.transition.as_bytes() == &[0; 32]
            || self.zone.digest().as_bytes() == &[0; 32]
            || self.departure != departure_digest(unit, actor, &self.zone, self.transition)?
        {
            return Err(bad("retained withheld identity"));
        }
        Ok(())
    }
}

/// Actual region transition metadata retained with the owner-generated Close.
/// Decoding this record is not a new host transition or a payable loss proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::minds::ic_sortie) struct ClosedCoverageCapture {
    format: u16,
    departure: DefinitionDigest,
    region_transition: DefinitionDigest,
    zone: DefinitionRef,
}
impl ClosedCoverageCapture {
    pub(in crate::minds::ic_sortie) fn validate(
        &self,
        open: &crate::minds::ic_sortie::records::Opening,
    ) -> Result<(), ParticipantError> {
        let withheld = open
            .withheld
            .as_ref()
            .ok_or_else(|| bad("ordinary opening"))?;
        withheld.validate(&open.unit, open.dispatcher)?;
        if self.format != 1
            || self.departure != withheld.departure
            || self.region_transition.as_bytes() == &[0; 32]
            || self.region_transition == withheld.transition
            || self.zone.digest().as_bytes() == &[0; 32]
        {
            return Err(bad("retained covered close"));
        }
        Ok(())
    }
}

/// Constructed only alongside the real later host transition. An existing
/// CloseEvent, zone reference or acknowledged Open cannot manufacture it.
pub(in crate::minds::ic_sortie) struct CoverageCloseEvidence {
    permit: Arc<WithheldDeparture>,
    zone: DefinitionRef,
    region_transition: DefinitionDigest,
    _transition: HostTransition,
}
impl CoverageCloseEvidence {
    pub(in crate::minds::ic_sortie) fn from_transition(
        permit: Arc<WithheldDeparture>,
        zone: DefinitionRef,
        region_transition: DefinitionDigest,
        transition: HostTransition,
    ) -> Result<Self, ParticipantError> {
        if zone.digest().as_bytes() == &[0; 32]
            || region_transition.as_bytes() == &[0; 32]
            || region_transition == permit.transition
        {
            return Err(bad("covered host transition"));
        }
        Ok(Self {
            permit,
            zone,
            region_transition,
            _transition: transition,
        })
    }
    pub(in crate::minds::ic_sortie) fn capture(
        &self,
        open: &crate::minds::ic_sortie::records::Opening,
    ) -> Result<ClosedCoverageCapture, ParticipantError> {
        self.permit.matches_capture(open.withheld.as_ref())?;
        if !matches!(&*self.permit.state.lock().map_err(bad)?,WithheldState::Acknowledged{open:actual,..}|WithheldState::Released{open:actual,..}
            if *actual==open.operation)
        {
            return Err(bad("unacknowledged departure close"));
        }
        let value = ClosedCoverageCapture {
            format: 1,
            departure: self.permit.identity()?,
            region_transition: self.region_transition,
            zone: self.zone.clone(),
        };
        value.validate(open)?;
        Ok(value)
    }
}
fn departure_digest(
    unit: &mush_tactics::TacticalUnit,
    actor: ObjId,
    zone: &DefinitionRef,
    transition: DefinitionDigest,
) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "tacsim.withheld_departure.v1",
        &serde_json::json!({"format":1,"transition":transition,
            "actor":actor,"unit":unit,"zone":zone}),
        crate::minds::ic_sortie::protocol::MAX_RECORD_BYTES,
    )
    .map_err(bad)
}

enum WithheldState {
    New,
    Released {
        open: CommitToken,
        binding: CommitToken,
    },
    Opening {
        origin: Arc<Mutex<Inbox>>,
        request: DefinitionDigest,
    },
    Acknowledged {
        origin: Arc<Mutex<Inbox>>,
        request: DefinitionDigest,
        open: CommitToken,
    },
}

/// The actual host has withheld this exact physical departure. It has no public
/// constructor: the absent region producer cannot be replaced by a StartPermit,
/// an OOC sortie, a serialized receipt or an arbitrary caller's zone selection.
pub struct WithheldDeparture {
    start: Arc<StartPermit>,
    zone: DefinitionRef,
    transition: DefinitionDigest,
    state: Mutex<WithheldState>,
    _withheld: HostTransition,
}
impl WithheldDeparture {
    pub(in crate::minds::ic_sortie) fn from_transition(
        start: Arc<StartPermit>,
        zone: DefinitionRef,
        transition: DefinitionDigest,
        withheld: HostTransition,
    ) -> Result<Self, ParticipantError> {
        if start.actor.0 == 0
            || start.unit.object() == start.actor
            || zone.digest().as_bytes() == &[0; 32]
            || transition.as_bytes() == &[0; 32]
        {
            return Err(bad("withheld transition"));
        }
        Ok(Self {
            start,
            zone,
            transition,
            state: Mutex::new(WithheldState::New),
            _withheld: withheld,
        })
    }
    fn identity(&self) -> Result<DefinitionDigest, ParticipantError> {
        departure_digest(
            &self.start.unit,
            self.start.actor,
            &self.zone,
            self.transition,
        )
    }
    pub(in crate::minds::ic_sortie) fn capture(
        &self,
        request: DefinitionDigest,
    ) -> Result<WithheldCapture, ParticipantError> {
        if !matches!(&*self.state.lock().map_err(bad)?,WithheldState::Opening{request:actual,..}
            if *actual==request)
        {
            return Err(bad("withheld opening request"));
        }
        Ok(WithheldCapture {
            format: 1,
            transition: self.transition,
            departure: self.identity()?,
            zone: self.zone.clone(),
        })
    }
    fn matches_capture(&self, capture: Option<&WithheldCapture>) -> Result<(), ParticipantError> {
        let actual = capture.ok_or_else(|| bad("ordinary Open is not a withheld departure"))?;
        actual.validate(&self.start.unit, self.start.actor)?;
        if actual.transition != self.transition
            || actual.zone != self.zone
            || actual.departure != self.identity()?
        {
            return Err(bad("withheld capture differs"));
        }
        Ok(())
    }
}

/// The same private capability and real IC ticket remain together until actual
/// owner readback. This selector is not permission to release the departure.
pub struct WithheldOpen {
    ticket: IcHostTicket,
    permit: Arc<WithheldDeparture>,
}
impl WithheldOpen {
    pub fn ticket(&self) -> &IcHostTicket {
        &self.ticket
    }
}

/// Definitively unadmitted custody. Restoration uses the exact original
/// capability, request and host; no replacement departure can be minted.
pub struct WithheldUnadmitted {
    original: IcUnadmittedOpen,
    permit: Arc<WithheldDeparture>,
}
impl WithheldUnadmitted {
    pub fn ticket(&self) -> &IcHostTicket {
        self.original.ticket()
    }
}

/// Actual acknowledged Open plus genuine withheld departure. No release method
/// is exposed here: the underwriting host must additionally prove the installed
/// paired policy/native/retention union before it can release physical custody.
pub struct AcknowledgedDeparture {
    host: IcHost,
    permit: Arc<WithheldDeparture>,
    record: Arc<Record>,
    digest: DefinitionDigest,
}

/// Readback of the actual permanent host event. It carries no current source or
/// licence eligibility; an accepted policy supplies its own interval and zone.
pub(crate) struct CommittedCoverageClose {
    host: IcHost,
    record: Arc<Record>,
    departure: DefinitionDigest,
}
impl CommittedCoverageClose {
    fn close(&self) -> Result<&crate::minds::ic_sortie::records::Closing, ParticipantError> {
        self.record.closed.as_ref().ok_or_else(|| bad("close"))
    }
    pub(crate) fn open(&self) -> CommitToken {
        self.record.open.operation
    }
    pub(crate) fn operation(&self) -> Result<CommitToken, ParticipantError> {
        Ok(self.close()?.operation)
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        Ok(self.close()?.event_digest)
    }
    pub(crate) fn transition(&self) -> Result<u64, ParticipantError> {
        Ok(self.close()?.transition)
    }
    pub(crate) fn at_ic(&self) -> Result<&mush_core::ClockStamp, ParticipantError> {
        Ok(&self.close()?.at_ic)
    }
    pub(crate) fn zone(&self) -> Result<&DefinitionRef, ParticipantError> {
        Ok(&self
            .close()?
            .coverage
            .as_ref()
            .ok_or_else(|| bad("close zone"))?
            .zone)
    }
    pub(crate) fn destroyed(&self) -> Result<bool, ParticipantError> {
        Ok(matches!(
            self.close()?.kind,
            crate::minds::ic_sortie::records::EventKind::Destroyed {}
        ))
    }
    pub(crate) fn kind(
        &self,
    ) -> Result<mush_roll::commerce::ic_claims::IcCloseKind, ParticipantError> {
        Ok(self.close()?.kind.data())
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        let state = audited_state(world, &self.host.writer)?;
        let actual = state
            .rows
            .get(&self.open())
            .ok_or_else(|| bad("retained close"))?;
        let close = actual
            .closed
            .as_ref()
            .ok_or_else(|| bad("retained close"))?;
        let capture = close
            .coverage
            .as_ref()
            .ok_or_else(|| bad("missing covered close"))?;
        capture.validate(&actual.open)?;
        if **actual != *self.record || capture.departure != self.departure {
            return Err(bad("retained covered close changed"));
        }
        Ok(())
    }
}
/// One-shot release of the actual withheld departure after complete paired
/// policy acknowledgement. It is not serializable or reconstructible from a
/// receipt. The absent region driver must still consume this at its own seam.
pub struct CoveredDeparture {
    permit: Arc<WithheldDeparture>,
    open: CommitToken,
    binding: CommitToken,
}
impl CoveredDeparture {
    pub fn open(&self) -> CommitToken {
        self.open
    }
    pub fn binding(&self) -> CommitToken {
        self.binding
    }
    pub(crate) fn exact(&self) -> Result<(), ParticipantError> {
        if !matches!(&*self.permit.state.lock().map_err(bad)?,WithheldState::Released{open,binding}
            if *open==self.open && *binding==self.binding)
        {
            return Err(bad("released departure differs"));
        }
        Ok(())
    }
}
impl AcknowledgedDeparture {
    pub(crate) fn release_after_binding(
        &self,
        world: &World,
        binding: CommitToken,
    ) -> Result<CoveredDeparture, ParticipantError> {
        self.recheck(world)?;
        let mut state = self.permit.state.lock().map_err(bad)?;
        if !matches!(&*state,WithheldState::Acknowledged{open,..}if *open==self.open()) {
            return Err(bad("departure already consumed"));
        }
        *state = WithheldState::Released {
            open: self.open(),
            binding,
        };
        Ok(CoveredDeparture {
            permit: Arc::clone(&self.permit),
            open: self.open(),
            binding,
        })
    }
    pub(crate) fn open(&self) -> CommitToken {
        self.record.open.operation
    }
    pub(crate) fn actor(&self) -> ObjId {
        self.record.open.dispatcher
    }
    pub(crate) fn unit(&self) -> ObjId {
        self.record.open.unit.object()
    }
    pub(crate) fn unit_revision(&self) -> u64 {
        self.record.open.unit.revision()
    }
    pub(crate) fn unit_schema(&self) -> &DefinitionRef {
        self.record.open.unit.schema()
    }
    pub(crate) fn zone(&self) -> &DefinitionRef {
        &self.permit.zone
    }
    pub(crate) fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub(crate) fn seal_digest(&self) -> DefinitionDigest {
        self.record.open.execution.digest()
    }
    pub(crate) fn admission_digest(&self) -> DefinitionDigest {
        self.record.open.admission_digest
    }
    pub(crate) fn seal(&self) -> &crate::arena::MindSeal {
        &self.record.open.execution.seal
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        let state = audited_state(world, &self.host.writer)?;
        let actual = state.rows.get(&self.open()).ok_or_else(|| bad("open"))?;
        self.permit.matches_capture(actual.open.withheld.as_ref())?;
        let withheld = self.permit.state.lock().map_err(bad)?;
        if !matches!(&*withheld, WithheldState::Acknowledged {origin,request,open}
            if Arc::ptr_eq(origin,&self.host.inbox)
                && *request==self.record.open.request && *open==self.open())
            || actual.open != self.record.open
            || actual.closed.is_some()
            || state.active.get(&self.unit()) != Some(&self.open())
            || self.permit.start.unit != actual.open.unit
            || self.permit.start.actor != actual.open.dispatcher
            || self.digest != self.permit.identity()?
        {
            return Err(bad("withheld departure changed"));
        }
        Ok(())
    }
}

impl IcHost {
    pub(crate) fn committed_coverage_close(
        &self,
        world: &World,
        open: CommitToken,
        departure: DefinitionDigest,
    ) -> Result<CommittedCoverageClose, ParticipantError> {
        let state = audited_state(world, &self.writer)?;
        let record = state.rows.get(&open).ok_or_else(|| bad("retained open"))?;
        let close = record
            .closed
            .as_ref()
            .ok_or_else(|| bad("no committed close"))?;
        let capture = close
            .coverage
            .as_ref()
            .ok_or_else(|| bad("no committed covered close"))?;
        capture.validate(&record.open)?;
        if capture.departure != departure {
            return Err(bad("another withheld departure"));
        }
        Ok(CommittedCoverageClose {
            host: self.clone(),
            record: Arc::clone(record),
            departure,
        })
    }

    /// Rebind the original private retained host capability after a real owner
    /// restart. A caller cannot substitute a data-only decoded Open or permit.
    /// This does not restore a missing region host's own transition checkpoint.
    pub fn recover_withheld(
        &self,
        world: &World,
        permit: &Arc<WithheldDeparture>,
    ) -> Result<AcknowledgedDeparture, ParticipantError> {
        let actual = audited_state(world, &self.writer)?;
        let mut state = permit.state.lock().map_err(bad)?;
        let request = match &*state {
            WithheldState::Opening { request, .. }
            | WithheldState::Acknowledged { request, .. } => *request,
            WithheldState::New => return Err(bad("departure was never submitted")),
            WithheldState::Released { .. } => return Err(bad("departure already released")),
        };
        let record = actual
            .rows
            .values()
            .find(|r| r.open.request == request)
            .ok_or_else(|| bad("no actual acknowledged opening"))?;
        permit.matches_capture(record.open.withheld.as_ref())?;
        if record.closed.is_some()
            || record.open.unit != permit.start.unit
            || record.open.dispatcher != permit.start.actor
            || actual.active.get(&record.open.unit.object()) != Some(&record.open.operation)
            || matches!(&*state,WithheldState::Acknowledged{open,..}if *open!=record.open.operation)
        {
            return Err(bad("changed acknowledged departure"));
        }
        let digest = permit.identity()?;
        *state = WithheldState::Acknowledged {
            origin: Arc::clone(&self.inbox),
            request,
            open: record.open.operation,
        };
        Ok(AcknowledgedDeparture {
            host: self.clone(),
            permit: Arc::clone(permit),
            record: Arc::clone(record),
            digest,
        })
    }

    /// Only the real distinct withheld capability reaches this path. The
    /// ordinary IC ticket/action still drives the unchanged four-owner Open.
    pub fn enqueue_withheld(
        &self,
        permit: &Arc<WithheldDeparture>,
        licence: LicenceId,
        profiles: Vec<SmolStr>,
    ) -> Result<WithheldOpen, ParticipantError> {
        let mut state = permit.state.lock().map_err(bad)?;
        if !matches!(*state, WithheldState::New) {
            return Err(bad("departure already retained"));
        }
        permit.identity()?;
        if profiles.len() > 8 || profiles.iter().any(|p| p.is_empty() || p.len() > 64) {
            return Err(bad("profiles"));
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(
            &serde_json::json!({"unit":permit.start.unit,"licence":licence,
                "profiles":profiles,"zone":permit.zone,"transition":permit.transition}),
            crate::minds::ic_sortie::protocol::MAX_RECORD_BYTES,
        )
        .map_err(bad)?
        .len();
        let request = DefinitionDigest::from_bytes(rand::random());
        let ticket = self.insert(
            Intent::Open {
                start: HostStart {
                    request,
                    permit: Arc::clone(&permit.start),
                    withheld: Some(Arc::clone(permit)),
                },
                licence,
                profiles,
            },
            bytes,
        )?;
        *state = WithheldState::Opening {
            origin: Arc::clone(&self.inbox),
            request: ticket.request,
        };
        Ok(WithheldOpen {
            ticket,
            permit: Arc::clone(permit),
        })
    }

    /// Only complete actual four-owner readback creates this proof. An ordinary
    /// success response and even an ordinary IcHostCompletion are insufficient.
    pub fn acknowledge_withheld(
        &self,
        world: &World,
        pending: &WithheldOpen,
    ) -> Result<Option<AcknowledgedDeparture>, ParticipantError> {
        let actual = audited_state(world, &self.writer)?;
        let mut state = pending.permit.state.lock().map_err(bad)?;
        if !matches!(&*state, WithheldState::Opening {origin,request}
            if Arc::ptr_eq(origin,&self.inbox) && *request==pending.ticket.request)
        {
            return Err(bad("withheld ticket changed or already acknowledged"));
        }
        let Some(record) = actual
            .rows
            .values()
            .find(|r| r.open.request == pending.ticket.request)
        else {
            return Ok(None);
        };
        pending
            .permit
            .matches_capture(record.open.withheld.as_ref())?;
        if record.closed.is_some()
            || record.open.unit != pending.permit.start.unit
            || record.open.dispatcher != pending.permit.start.actor
            || pending.ticket.actor != pending.permit.start.actor
            || pending.ticket.action != self.actions[0]
            || actual.active.get(&record.open.unit.object()) != Some(&record.open.operation)
        {
            return Err(bad("withheld opening differs"));
        }
        let digest = pending.permit.identity()?;
        // A caller may have consumed ordinary ACK readback first. The private
        // retained request and exact actual row still bind this original permit.
        let mut inbox = self.inbox.lock().map_err(bad)?;
        if let Some(row) = inbox.rows.get(&pending.ticket.request) {
            self.check_ticket(row, &pending.ticket)?;
            if !matches!(&row.intent,Intent::Open{start,..}
                if Arc::ptr_eq(&start.permit,&pending.permit.start))
            {
                return Err(bad("withheld permit affinity"));
            }
            let bytes = inbox
                .bytes
                .checked_sub(row.bytes)
                .ok_or_else(|| bad("bytes"))?;
            inbox
                .rows
                .remove(&pending.ticket.request)
                .ok_or_else(|| bad("pending"))?;
            inbox.bytes = bytes;
        }
        *state = WithheldState::Acknowledged {
            origin: Arc::clone(&self.inbox),
            request: pending.ticket.request,
            open: record.open.operation,
        };
        Ok(Some(AcknowledgedDeparture {
            host: self.clone(),
            permit: Arc::clone(&pending.permit),
            record: Arc::clone(record),
            digest,
        }))
    }

    pub fn discard_unadmitted_withheld(
        &self,
        world: &World,
        pending: &WithheldOpen,
    ) -> Result<WithheldUnadmitted, ParticipantError> {
        let state = pending.permit.state.lock().map_err(bad)?;
        if !matches!(&*state,WithheldState::Opening{origin,request}
            if Arc::ptr_eq(origin,&self.inbox) && *request==pending.ticket.request)
        {
            return Err(bad("withheld ticket"));
        }
        {
            let inbox = self.inbox.lock().map_err(bad)?;
            let row = inbox
                .rows
                .get(&pending.ticket.request)
                .ok_or_else(|| bad("pending"))?;
            self.check_ticket(row, &pending.ticket)?;
            if !matches!(&row.intent,Intent::Open{start,..}
                if Arc::ptr_eq(&start.permit,&pending.permit.start)
                    && start.withheld.as_ref().is_some_and(|p|Arc::ptr_eq(p,&pending.permit)))
            {
                return Err(bad("withheld original permit"));
            }
        }
        let original = self.discard_unadmitted_open(world, &pending.ticket)?;
        Ok(WithheldUnadmitted {
            original,
            permit: Arc::clone(&pending.permit),
        })
    }

    pub fn restore_unadmitted_withheld(
        &self,
        world: &World,
        released: &mut WithheldUnadmitted,
    ) -> Result<WithheldOpen, ParticipantError> {
        let state = released.permit.state.lock().map_err(bad)?;
        if !matches!(&*state,WithheldState::Opening{origin,request}
            if Arc::ptr_eq(origin,&self.inbox) && *request==released.original.ticket.request)
            || !Arc::ptr_eq(released.original.permit(), &released.permit.start)
        {
            return Err(bad("withheld original permit"));
        }
        let ticket = self.restore_unadmitted_open(world, &mut released.original)?;
        Ok(WithheldOpen {
            ticket,
            permit: Arc::clone(&released.permit),
        })
    }
}

/// Boot-only comparison against actual loaded IC owner data. This returns no
/// capability and never substitutes for the independent four-owner final audit.
pub(crate) fn audit_history(
    world: &World,
    basis: &mush_economy::managed_coverage::CoverageBasisV1,
    buyer: ObjId,
    close: Option<(
        CommitToken,
        DefinitionDigest,
        u64,
        &mush_core::ClockStamp,
        &DefinitionRef,
        mush_roll::commerce::ic_claims::IcCloseKind,
    )>,
) -> Result<(), ParticipantError> {
    if world
        .module_state_owner(std::any::TypeId::of::<IcSorties>())
        .is_none_or(|owner| owner.as_str() != crate::NS)
    {
        return Err(bad("IC owner"));
    }
    let state = world
        .module_state::<IcSorties>()
        .ok_or_else(|| bad("IC inventory"))?;
    if !state.loaded {
        return Err(bad("IC inventory not loaded"));
    }
    let row = state
        .rows
        .get(&basis.ic_open)
        .ok_or_else(|| bad("IC opening"))?;
    row.validate()?;
    let withheld = row
        .open
        .withheld
        .as_ref()
        .ok_or_else(|| bad("ordinary opening"))?;
    withheld.validate(&row.open.unit, row.open.dispatcher)?;
    if row.open.dispatcher != buyer
        || row.open.unit.object() != basis.hull
        || row.open.unit.revision() != basis.hull_revision
        || row.open.unit.schema() != &basis.hull_schema
        || row.open.execution.digest() != basis.seal_digest
        || row.open.admission_digest != basis.admission_digest
        || withheld.departure != basis.departure_digest
        || withheld.zone != basis.zone
    {
        return Err(bad("coverage opening affinity"));
    }
    if let Some((operation, digest, transition, at, zone, kind)) = close {
        let actual = row.closed.as_ref().ok_or_else(|| bad("IC close"))?;
        let capture = actual
            .coverage
            .as_ref()
            .ok_or_else(|| bad("ordinary close"))?;
        capture.validate(&row.open)?;
        if actual.operation != operation
            || actual.event_digest != digest
            || actual.transition != transition
            || &actual.at_ic != at
            || &capture.zone != zone
            || actual.kind.data() != kind
        {
            return Err(bad("coverage close affinity"));
        }
    }
    Ok(())
}

/// Historical comparison only, after the actual native opening inventory has
/// loaded. No decoded selection can mint a permit or establish current rights.
pub(crate) fn audit_qualified_history(
    world: &World,
    basis: &mush_economy::managed_coverage::CoverageBasisV1,
    buyer: ObjId,
    selected: &mush_roll::commerce::coverage::CoverageConductRef,
) -> Result<(), ParticipantError> {
    audit_history(world, basis, buyer, None)?;
    let state = world
        .module_state::<IcSorties>()
        .ok_or_else(|| bad("IC inventory"))?;
    let row = state
        .rows
        .get(&basis.ic_open)
        .ok_or_else(|| bad("IC opening"))?;
    let seal = &row.open.execution.seal;
    let certificate = seal
        .origin
        .certificate()
        .ok_or_else(|| bad("qualified certificate"))?;
    let identity = selected.revision();
    let revision = identity.revision();
    let rating = selected.rating();
    if !seal.governed
        || seal.engine != crate::arena::mind::SealEngine::Scripted
        || seal.identity() != seal.requested
        || seal.cards.selected.len() != 1
        || seal.cards.selected.name(0) != Some(selected.profile())
        || certificate.revision_character != revision.key.character
        || certificate.revision_mind != revision.key.mind
        || certificate.revision_number != revision.key.number
        || certificate.revision_digest != revision.digest
        || certificate.source_digest != identity.source()
        || certificate.rating_job != rating.job()
        || certificate.rating_id != rating.id().digest()
        || certificate.rating_schema != rating.schema()
        || certificate.rating_digest != rating.receipt_digest()
    {
        return Err(bad("qualified captured executable"));
    }
    Ok(())
}
