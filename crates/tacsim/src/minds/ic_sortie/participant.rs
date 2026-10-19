//! The actual Tacsim seal owner. No public data constructor mints the host
//! capabilities below. The absent region producer remains an explicit boundary.
use super::execution::Execution;
use super::records::{CapturedProtocol, Closing, EventKind, HEAD, Head, Opening, Record, bad};
use super::state::IcSorties;
use mush_core::action::PrincipalId;
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::mechanics::DefinitionRegistry;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, OwnerId, World};
use mush_roll::commerce::ic_claims::{
    IcAdmissionFacts, IcClaimConfig, IcCloseKind, IcSealOutput, IcSealParticipant,
};
use mush_sim::ic_exclusion::IcEventRef;
use mush_tactics::{TacticalUnit, UnitMutationPlan};
use smol_str::SmolStr;
use std::sync::{Arc, OnceLock};

/// Deliberately private, non-Serde and non-Clone. There is presently no producer
/// for a genuine IC region transition, so this module exports no constructor.
/// A later host must supply its actual transition before any permit is minted.
pub(super) struct HostTransition {
    _private: (),
}
pub struct StartPermit {
    pub(super) unit: TacticalUnit,
    pub(super) actor: ObjId,
    _transition: HostTransition,
}
pub struct CloseEvent {
    pub(super) actor: ObjId,
    pub(super) open: CommitToken,
    unit: TacticalUnit,
    seal: DefinitionDigest,
    sequence: u64,
    kind: EventKind,
    at_ic: ClockStamp,
    pub(super) conditions: UnitMutationPlan,
    coverage: Option<super::host::coverage::CoverageCloseEvidence>,
    _transition: HostTransition,
}
impl CloseEvent {
    pub(super) fn retained_bytes(&self) -> Result<usize, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::json!({
                "unit":self.unit,"conditions":self.conditions,"open":self.open,
                "seal":self.seal,"sequence":self.sequence,"kind":self.kind,"at_ic":self.at_ic,
            }),
            super::protocol::MAX_RECORD_BYTES,
        )
        .map(|v| v.len())
        .map_err(bad)
    }
}
// Neither a player token, recorded receipt, nor a current Interior can create
// either capability. Keeping these types unmintable is preferable to claiming
// the OOC lifecycle is an IC host. Tests of preparation must state that scope.

pub struct IcSortieRequest {
    operation: Operation,
}
enum Operation {
    Open {
        start: super::host::HostStart,
        facts: IcAdmissionFacts,
        profiles: Vec<SmolStr>,
    },
    Close(super::host::HostClose),
}

#[derive(Clone)]
pub(super) struct Config {
    pub(super) claims: IcClaimConfig,
    pub(super) writer: mush_roll::commerce::ic_claims::IcWriter,
    pub(super) arena: crate::config::ArenaConfig,
    pub(super) minds: crate::config::MindsConfig,
    pub(super) definitions: Arc<OnceLock<DefinitionRegistry>>,
}
impl Config {
    fn check(&self, ctx: &OwnerPrepareContext<'_>, close: bool) -> Result<(), ParticipantError> {
        let expected = self.writer.require_audited(ctx.world()).map_err(bad)?;
        let state = ctx
            .world()
            .module_state::<IcSorties>()
            .ok_or_else(|| bad("seal owner"))?;
        if !state.loaded || state.next_transition != expected {
            return Err(bad("actual four-owner audit or head differs"));
        }
        let definitions = self
            .definitions
            .get()
            .ok_or_else(|| bad("definitions unavailable"))?;
        let actions = self.claims.actions().map_err(bad)?;
        if actions.get(usize::from(close)) != Some(ctx.action())
            || ctx.invocation().principal()
                != &(PrincipalId::Object {
                    id: ctx.invocation().actor(),
                })
        {
            return Err(ParticipantError::Authority);
        }
        for r in self
            .claims
            .participants()
            .iter()
            .chain([self.claims.manifest(), ctx.action()])
        {
            definitions.get_exact(r).map_err(bad)?;
        }
        Ok(())
    }
}
pub struct IcSortieParticipant {
    pub(super) config: Config,
}
pub struct IcSortieOutput {
    action: DefinitionRef,
    commit: CommitToken,
    row: Arc<Record>,
    event: Option<IcEventRef>,
    conditions: Option<UnitMutationPlan>,
}
impl IcSealOutput for IcSortieOutput {
    fn action(&self) -> &DefinitionRef {
        &self.action
    }
    fn commit(&self) -> CommitToken {
        self.commit
    }
    fn open(&self) -> CommitToken {
        self.row.open.operation
    }
    fn unit(&self) -> ObjId {
        self.row.open.execution.seal.unit
    }
    fn sortie(&self) -> u64 {
        self.row.open.execution.seal.sortie.0
    }
    fn transition(&self) -> u64 {
        self.row
            .closed
            .as_ref()
            .map_or(self.sortie(), |c| c.transition)
    }
    fn admission_digest(&self) -> DefinitionDigest {
        self.row.open.admission_digest
    }
    fn seal_digest(&self) -> DefinitionDigest {
        self.row.open.execution.digest()
    }
    fn at_ic(&self) -> &ClockStamp {
        self.row
            .closed
            .as_ref()
            .map_or(&self.row.open.at_ic, |c| &c.at_ic)
    }
    fn close_event(&self) -> Option<&IcEventRef> {
        self.event.as_ref()
    }
    fn close_kind(&self) -> Option<IcCloseKind> {
        self.row.closed.as_ref().map(|c| c.kind.data())
    }
    fn conditions(&self) -> Option<&UnitMutationPlan> {
        self.conditions.as_ref()
    }
    fn closing_unit_digest(&self) -> Option<DefinitionDigest> {
        self.row.closed.as_ref().map(|c| c.unit_digest)
    }
}
impl IcSealParticipant for IcSortieParticipant {
    type Start = super::host::HostStart;
    type Close = super::host::HostClose;
    fn opening_unit(start: &Self::Start) -> ObjId {
        start.permit.unit.object()
    }
    fn opening(
        start: Self::Start,
        facts: IcAdmissionFacts,
        profiles: Vec<SmolStr>,
    ) -> IcSortieRequest {
        IcSortieRequest {
            operation: Operation::Open {
                start,
                facts,
                profiles,
            },
        }
    }
    fn closing(event: Self::Close) -> IcSortieRequest {
        IcSortieRequest {
            operation: Operation::Close(event),
        }
    }
    fn closing_open(event: &Self::Close) -> CommitToken {
        event.permit.open
    }
    fn audit_claims(
        world: &World,
        claims: &[mush_roll::commerce::ic_claims::IcClaimHistory],
        next_transition: u64,
    ) -> Result<(), mush_core::ModError> {
        super::audit::claims(world, claims, next_transition)
    }
}

pub struct IcSortiePatch {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    close: bool,
    before_generation: u64,
    unit: TacticalUnit,
    post: IcSorties,
}
pub struct BoundIcSortie<'a> {
    state: &'a mut IcSorties,
    post: IcSorties,
}
impl BoundParticipant for BoundIcSortie<'_> {
    fn apply(self) {
        *self.state = self.post;
    }
}
fn current_unit(world: &World, expected: &TacticalUnit) -> Result<(), ParticipantError> {
    if world
        .module_state_owner(std::any::TypeId::of::<mush_tactics::UnitStore>())
        .is_none_or(|owner| owner.as_str() != "tactics")
        || mush_tactics::module::unit_of(world, expected.object()) != Some(expected)
    {
        return Err(bad("unit changed"));
    }
    Ok(())
}
impl OwnerParticipant for IcSortieParticipant {
    type State = IcSorties;
    type Request = IcSortieRequest;
    type Output = IcSortieOutput;
    type Patch = IcSortiePatch;
    type Bound<'a> = BoundIcSortie<'a>;
    fn prepare(
        &self,
        state: &IcSorties,
        ctx: &OwnerPrepareContext<'_>,
        request: IcSortieRequest,
    ) -> Result<OwnerPreparation<IcSortieOutput, IcSortiePatch>, ParticipantError> {
        state.ready()?;
        let (row, conditions, unit) = match request.operation {
            Operation::Open {
                start,
                facts,
                profiles,
            } => {
                self.config.check(ctx, false)?;
                let request = start.request;
                let withheld = start
                    .withheld
                    .as_ref()
                    .map(|w| w.capture(request))
                    .transpose()?;
                let start = &start.permit;
                if start.actor != ctx.invocation().actor()
                    || start.actor != facts.dispatcher()
                    || start.unit.object() != facts.unit()
                {
                    return Err(ParticipantError::Authority);
                }
                current_unit(ctx.world(), &start.unit)?;
                let execution = Execution::capture(
                    ctx.world(),
                    &facts,
                    state.next_transition,
                    &profiles,
                    &self.config.arena,
                    &self.config.minds,
                )?;
                let row = Record {
                    format: 1,
                    open: Opening {
                        request,
                        operation: ctx.commit_token(),
                        action: ctx.action().clone(),
                        protocol: CapturedProtocol::capture(&self.config.claims),
                        dispatcher: start.actor,
                        withheld,
                        admission_digest: facts.binding_digest().map_err(bad)?,
                        at_ic: ctx.world().world_ic_now(),
                        unit: start.unit.clone(),
                        execution,
                    },
                    closed: None,
                };
                (row, None, start.unit.clone())
            }
            Operation::Close(event) => {
                let request = event.request;
                let event = &event.permit;
                if event.actor != ctx.invocation().actor() {
                    return Err(ParticipantError::Authority);
                }
                self.config.check(ctx, true)?;
                let before = state
                    .rows
                    .get(&event.open)
                    .ok_or_else(|| bad("open missing"))?;
                if before.closed.is_some()
                    || before.open.unit != event.unit
                    || before.open.execution.digest() != event.seal
                    || event.sequence == 0
                {
                    return Err(bad("host event identity"));
                }
                current_unit(ctx.world(), &event.unit)?;
                let digest = DefinitionDigest::for_canonical_value_with_limit(
                    "tacsim.ic_conditions.v1",
                    &serde_json::to_value(&event.conditions).map_err(bad)?,
                    super::protocol::MAX_RECORD_BYTES,
                )
                .map_err(bad)?;
                let physical = ctx
                    .world()
                    .module_state::<mush_tactics::UnitStore>()
                    .ok_or_else(|| bad("physical owner"))?
                    .ic_custody_inventory()?;
                let unit_digest = physical
                    .iter()
                    .find(|v| v.open() == event.open)
                    .ok_or_else(|| bad("physical opening"))?
                    .project_close_unit_digest(
                        &before.open.unit,
                        &event.conditions,
                        ctx.world().now(),
                    )?;
                let mut close = Closing {
                    request,
                    operation: ctx.commit_token(),
                    action: ctx.action().clone(),
                    protocol: CapturedProtocol::capture(&self.config.claims),
                    transition: state.next_transition,
                    sequence: event.sequence,
                    event_digest: DefinitionDigest::from_bytes([0; 32]),
                    kind: event.kind.clone(),
                    at_ic: event.at_ic.clone(),
                    conditions_digest: digest,
                    unit_digest,
                    coverage: event
                        .coverage
                        .as_ref()
                        .map(|c| c.capture(&before.open))
                        .transpose()?,
                };
                close.event_digest = close.event_digest(&before.open)?;
                let mut row = (**before).clone();
                row.closed = Some(close);
                (row, Some(event.conditions.clone()), event.unit.clone())
            }
        };
        row.validate()?;
        let close = row.closed.is_some();
        let post = if close {
            state.closing(row.clone())?
        } else {
            state.opening(row.clone())?
        };
        let head = Head {
            format: 1,
            next_transition: post.next_transition,
        };
        head.validate()?;
        let event = row.closed.as_ref().map(|c| IcEventRef {
            sequence: c.sequence,
            digest: c.event_digest,
        });
        let writes = vec![
            ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(crate::NS).map_err(bad)?,
                key: AuthoritativeKey::parse_canonical(row.key()).map_err(bad)?,
                value: serde_json::to_value(&row).map_err(bad)?,
            },
            ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(crate::NS).map_err(bad)?,
                key: AuthoritativeKey::parse_canonical(HEAD).map_err(bad)?,
                value: serde_json::to_value(head).map_err(bad)?,
            },
        ];
        Ok(OwnerPreparation::new(
            IcSortieOutput {
                action: ctx.action().clone(),
                commit: ctx.commit_token(),
                row: Arc::new(row),
                event,
                conditions,
            },
            IcSortiePatch {
                action: ctx.action().clone(),
                commit: ctx.commit_token(),
                actor: ctx.invocation().actor(),
                close,
                before_generation: state.generation,
                unit,
                post,
            },
            writes,
        ))
    }
    fn recheck(
        &self,
        state: &IcSorties,
        ctx: &OwnerPrepareContext<'_>,
        patch: &IcSortiePatch,
    ) -> Result<(), ParticipantError> {
        state.ready()?;
        self.config.check(ctx, patch.close)?;
        if state.generation != patch.before_generation
            || ctx.action() != &patch.action
            || ctx.commit_token() != patch.commit
            || ctx.invocation().actor() != patch.actor
        {
            return Err(bad("stale preparation"));
        }
        current_unit(ctx.world(), &patch.unit)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut IcSorties,
        patch: IcSortiePatch,
    ) -> Result<BoundIcSortie<'a>, ParticipantError> {
        state.ready()?;
        if state.generation != patch.before_generation {
            return Err(bad("stale bind"));
        }
        Ok(BoundIcSortie {
            state,
            post: patch.post,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != crate::NS {
            return false;
        }
        if key.as_str() == HEAD {
            let Ok(head) = serde_json::from_value::<Head>(value.clone()) else {
                return false;
            };
            return head.validate().is_ok()
                && serde_json::to_value(head).ok().as_ref() == Some(value);
        }
        let Ok(row) = Record::decode(value) else {
            return false;
        };
        key.as_str() == row.key()
            && self
                .config
                .claims
                .actions()
                .ok()
                .and_then(|a| a.get(usize::from(row.closed.is_some())).cloned())
                == Some(
                    row.closed
                        .as_ref()
                        .map_or(&row.open.action, |c| &c.action)
                        .clone(),
                )
    }
}

/// Fixture-only host boundary. This does not establish a genuine region event;
/// every owner, certificate, seal, physical image and readback after it is real.
#[cfg(test)]
pub(crate) fn fixture_start(
    world: &World,
    actor: ObjId,
    unit: ObjId,
) -> Result<Arc<StartPermit>, ParticipantError> {
    Ok(Arc::new(StartPermit {
        unit: mush_tactics::module::unit_of(world, unit)
            .ok_or_else(|| bad("fixture unit"))?
            .clone(),
        actor,
        _transition: HostTransition { _private: () },
    }))
}
#[cfg(test)]
pub(crate) fn fixture_close(
    world: &World,
    actor: ObjId,
    open: CommitToken,
    base: ObjId,
) -> Result<Arc<CloseEvent>, ParticipantError> {
    let state = world
        .module_state::<IcSorties>()
        .ok_or_else(|| bad("fixture owner"))?;
    let record = state
        .rows
        .get(&open)
        .ok_or_else(|| bad("fixture actual open"))?;
    if record.closed.is_some() {
        return Err(bad("fixture closed"));
    }
    Ok(Arc::new(CloseEvent {
        actor,
        open,
        unit: record.open.unit.clone(),
        seal: record.open.execution.digest(),
        sequence: 1,
        kind: EventKind::PoweredDownInBase { base },
        at_ic: world.world_ic_now(),
        conditions: UnitMutationPlan::new(
            vec![],
            Some(record.open.unit.revision()),
            Some("ic fixture terminal conditions".into()),
        )
        .map_err(bad)?,
        coverage: None,
        _transition: HostTransition { _private: () },
    }))
}

/// Only a fixture models the missing region's genuine withholding transition.
/// The caller must still execute and acknowledge the real four-owner Open.
#[cfg(test)]
pub(crate) fn fixture_withheld(
    start: Arc<StartPermit>,
    zone: DefinitionRef,
    transition: DefinitionDigest,
) -> Result<Arc<super::host::coverage::WithheldDeparture>, ParticipantError> {
    Ok(Arc::new(
        super::host::coverage::WithheldDeparture::from_transition(
            start,
            zone,
            transition,
            HostTransition { _private: () },
        )?,
    ))
}

#[cfg(test)]
pub(crate) fn fixture_covered_close(
    world: &World,
    actor: ObjId,
    open: CommitToken,
    base: ObjId,
    permit: Arc<super::host::coverage::WithheldDeparture>,
    zone: DefinitionRef,
    transition: DefinitionDigest,
) -> Result<Arc<CloseEvent>, ParticipantError> {
    let mut event = fixture_close(world, actor, open, base)?;
    let actual = Arc::get_mut(&mut event).ok_or_else(|| bad("private fixture event"))?;
    actual.coverage = Some(
        super::host::coverage::CoverageCloseEvidence::from_transition(
            permit,
            zone,
            transition,
            HostTransition { _private: () },
        )?,
    );
    Ok(event)
}

#[cfg(test)]
pub(crate) fn fixture_covered_loss(
    world: &World,
    actor: ObjId,
    open: CommitToken,
    base: ObjId,
    permit: Arc<super::host::coverage::WithheldDeparture>,
    zone: DefinitionRef,
    transition: DefinitionDigest,
) -> Result<Arc<CloseEvent>, ParticipantError> {
    let mut event = fixture_covered_close(world, actor, open, base, permit, zone, transition)?;
    let actual = Arc::get_mut(&mut event).ok_or_else(|| bad("private loss fixture"))?;
    actual.kind = EventKind::Destroyed {};
    actual.conditions = UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetSectionCondition {
            section: mush_tactics::SectionId::new("hull").map_err(bad)?,
            condition: mush_tactics::ConditionId::new(crate::arena::damage::WRECK_CONDITION)
                .map_err(bad)?,
        }],
        Some(actual.unit.revision()),
        Some("private region loss fixture".into()),
    )
    .map_err(bad)?;
    Ok(event)
}
