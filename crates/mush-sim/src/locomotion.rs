//! Captured ordinary locomotion's Sim audit and uncertified crossing participant.
//!
//! This action is registered but offers no command or legal-action route. Core
//! must supply an actual captured dispatch and its retained admission. The only
//! Sim mutation is a genuine recovery-ledger append; placement and interior
//! records do not change. Core owns the exact actor location postimage.
//!
//! Observer completion is not implemented here. In particular, Atlas may place
//! an unplaced arrival on a window, so this foundation refuses while any window
//! exists, including an unrelated one. A future owner join can narrow that
//! restriction. Existing custody, journey and control exclusions also remain.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use mush_core::ledger::BoundLedgerAppend;
use mush_core::locomotion::transition::{CoreLocomotion, LocomotionDraft, LocomotionIntent};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, ClockStamp, DefinitionDescriptor, DefinitionDigest, DefinitionId,
    DefinitionKind, DefinitionRef, ModError, ModuleCtx, OwnerId, PerceivedRef, PrepareCtx,
    PreparedLedgerAppend, PrincipalId, Registrar, ReplayWrite, RulesValue, World,
};
use serde_json::{Value, json};

use crate::NS;
use crate::data::{DomainId, RECOVERY_LOG, SpaceAnchor, SpatialParent, SpatialState};
use crate::locomotion_capture::{ActorMove, LocomotionCrossings};
use crate::personnel_capture::CaptureFacts;

const OUTCOME: &str = "locomotion_transition_result";
const MAX_DEPENDENCIES: usize = crate::personnel_capture::MAX_DEPENDENCIES;

fn refused(message: &'static str) -> ParticipantError {
    ParticipantError::refused(message)
}

fn unavailable(message: &'static str) -> ActionError {
    crate::tick::err(ActionErrorCode::Unavailable, message)
}

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(bad)?,
        DefinitionId::new(OUTCOME).map_err(bad)?,
        1,
        json!({
            "fields":["actor","source","exit","destination","prepared_at","plan_digest","capture_digest"],
            "capture":"sim.locomotion_crossings",
            "observer_completion":false,
            "personnel_eligibility":false,
        }),
    )
    .map(|descriptor| descriptor.as_current())
    .map_err(bad)
}

pub(crate) fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(bad)?,
        DefinitionId::new(mush_core::locomotion::transition::PARTICIPANT).map_err(bad)?,
        1,
        json!({
            "state":"SpatialState",
            "documents":[],
            "ledger":RECOVERY_LOG,
            "operations":["captured_exit","captured_home"],
            "protocol":"sim_locomotion_capture_v1",
            "capture":"sim.locomotion_crossings",
            "authority":"core_captured_locomotion",
            "window_policy":"refuse_any_window",
            "observer_completion":false,
            "personnel_eligibility":false,
        }),
    )
    .map_err(bad)
}

#[derive(Clone)]
struct LocomotionHandles {
    service: CoreLocomotion<LocomotionParticipant>,
    crossings: LocomotionCrossings,
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let participant = reg.definition(participant_descriptor()?)?;
    let action = reg.definition(mush_core::locomotion::transition::action_descriptor()?)?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        LocomotionParticipant,
    )?;
    let service =
        CoreLocomotion::<LocomotionParticipant>::register(reg, action.clone(), participant)?;
    let crossings = LocomotionCrossings::register(reg)?;
    reg.action_provider(
        action,
        Provider {
            outcome,
            service: service.clone(),
            crossings: crossings.clone(),
        },
    )?;
    reg.state(LocomotionHandles { service, crossings });
    Ok(())
}

pub(crate) fn loaded(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let handles = ctx
        .w
        .module_state::<LocomotionHandles>()
        .ok_or_else(|| bad("Sim locomotion handles are missing"))?;
    handles.service.audit(ctx).map_err(bad)?;
    handles.crossings.audit(ctx, &handles.service).map_err(bad)
}

/// These restrictions are checked without callbacks at bind as well as before
/// each preparation/recheck. In particular a newly opened unrelated window
/// cannot slip through a sparse route-local topology fence.
fn local_policy(state: &SpatialState) -> Result<(), ParticipantError> {
    crate::physical::resident_bound(state)?;
    if !state.unbound_placement_docs.is_empty() {
        return Err(refused("unbound placement facts require repair"));
    }
    if state.domains.values().any(|domain| domain.window.is_some()) {
        return Err(refused(
            "locomotion with a window requires its completion owner",
        ));
    }
    Ok(())
}

struct StateFence {
    nodes: BTreeSet<mush_core::ObjId>,
    domains: BTreeSet<DomainId>,
    digest: DefinitionDigest,
}

impl StateFence {
    fn capture(
        state: &SpatialState,
        nodes: BTreeSet<mush_core::ObjId>,
    ) -> Result<Self, ParticipantError> {
        if nodes.len() > MAX_DEPENDENCIES {
            return Err(refused("locomotion dependency limit exceeded"));
        }
        let mut domains = BTreeSet::new();
        for node in &nodes {
            if let SpatialParent::Direct { domain, .. } = state.parent_of(*node) {
                domains.insert(domain);
            }
            if let Some(SpaceAnchor::FixedDomain { domain, .. }) =
                state.spaces.get(node).map(|space| &space.anchor)
            {
                domains.insert(*domain);
            }
            if let Some(domain) = state.derived.domain_by_host.get(node) {
                domains.insert(*domain);
            }
            if let Some(presentations) = state.derived.presentation_of.get(node) {
                if presentations.len() > MAX_DEPENDENCIES {
                    return Err(refused("locomotion domain dependency limit exceeded"));
                }
                domains.extend(presentations.iter().copied());
            }
            if domains.len() > MAX_DEPENDENCIES {
                return Err(refused("locomotion domain dependency limit exceeded"));
            }
        }
        let digest = crate::physical::state_digest(state, &nodes, &domains)?;
        Ok(Self {
            nodes,
            domains,
            digest,
        })
    }

    fn recheck(&self, state: &SpatialState) -> Result<(), ParticipantError> {
        local_policy(state)?;
        if crate::physical::state_digest(state, &self.nodes, &self.domains)? != self.digest {
            return Err(refused("locomotion topology changed"));
        }
        Ok(())
    }
}

struct Captured {
    facts: CaptureFacts,
    plan_digest: DefinitionDigest,
    fence: StateFence,
}

/// Physical contents alone omit an exposed attachment, whose location is the
/// host's room. Refuse those additional projection writes, including when the
/// host is a carried hull or an occupant of its separately anchored cabin.
/// Every consulted child/mount/anchor relation joins the final topology fence.
fn projection_dependencies(
    state: &SpatialState,
    world: &World,
    actor: mush_core::ObjId,
) -> Result<BTreeSet<mush_core::ObjId>, ParticipantError> {
    fn enqueue(
        pending: &mut VecDeque<mush_core::ObjId>,
        nodes: &mut BTreeSet<mush_core::ObjId>,
        node: mush_core::ObjId,
    ) -> Result<(), ParticipantError> {
        if nodes.insert(node) {
            if nodes.len() > MAX_DEPENDENCIES {
                return Err(refused("locomotion projection dependency limit exceeded"));
            }
            pending.push_back(node);
        }
        Ok(())
    }
    let mut nodes = BTreeSet::from([actor]);
    let mut pending = VecDeque::from([actor]);
    let mut work = 0usize;
    while let Some(node) = pending.pop_front() {
        let object = world
            .get(node)
            .ok_or_else(|| refused("locomotion dependency is missing"))?;
        if state.derived.domain_by_host.contains_key(&node) {
            return Err(refused("a moving domain requires its lifecycle owner"));
        }
        let children = state.children_of(node);
        let spaces = state.derived.anchored_spaces.get(&node);
        let room_count = state.spaces.get(&node).map_or(0, |space| space.rooms.len());
        work = work
            .saturating_add(object.contents.len())
            .saturating_add(children.len())
            .saturating_add(spaces.map_or(0, BTreeSet::len))
            .saturating_add(room_count)
            .saturating_add(1);
        if work > 8_192
            || object.contents.len() > MAX_DEPENDENCIES
            || children.len() > MAX_DEPENDENCIES
            || room_count > crate::MAX_SPACE_ROOMS
            || spaces.is_some_and(|spaces| spaces.len() > MAX_DEPENDENCIES)
        {
            return Err(refused("locomotion projection traversal limit exceeded"));
        }
        for child in &object.contents {
            if world
                .get(*child)
                .is_none_or(|child| child.location != Some(node))
            {
                return Err(refused("locomotion contents are inconsistent"));
            }
            enqueue(&mut pending, &mut nodes, *child)?;
        }
        for child in children {
            match state.parent_of(*child) {
                SpatialParent::Attached { host, mount, .. } => {
                    let spec = state
                        .mounts
                        .get(&host)
                        .and_then(|mounts| mounts.mounts.get(&mount))
                        .ok_or_else(|| refused("locomotion attachment is inconsistent"))?;
                    if host != node || spec.exposed {
                        return Err(refused(
                            "an exposed attachment requires its projection owner",
                        ));
                    }
                    if world.get(*child).and_then(|child| child.location) != Some(host)
                        || object.contents.iter().filter(|id| **id == *child).count() != 1
                    {
                        return Err(refused("suppressed attachment presence is inconsistent"));
                    }
                }
                SpatialParent::Interior { space, .. } => {
                    let room = world
                        .get(*child)
                        .and_then(|child| child.location)
                        .ok_or_else(|| refused("interior occupant presence is missing"))?;
                    let interior = state
                        .spaces
                        .get(&space)
                        .ok_or_else(|| refused("interior occupant space is missing"))?;
                    if space != node || (room != space && !interior.rooms.contains(&room)) {
                        return Err(refused("interior occupant presence is inconsistent"));
                    }
                    let holder = world
                        .get(room)
                        .ok_or_else(|| refused("interior room is missing"))?;
                    if holder.kind != mush_core::ObjKind::Room
                        || holder.contents.len() > MAX_DEPENDENCIES
                        || holder.contents.iter().filter(|id| **id == *child).count() != 1
                    {
                        return Err(refused("interior occupant contents are inconsistent"));
                    }
                }
                _ => return Err(refused("locomotion child relation is inconsistent")),
            }
            enqueue(&mut pending, &mut nodes, *child)?;
        }
        if let Some(spaces) = spaces {
            for space in spaces {
                enqueue(&mut pending, &mut nodes, *space)?;
            }
        }
        if let Some(space) = state.spaces.get(&node) {
            for room in &space.rooms {
                enqueue(&mut pending, &mut nodes, *room)?;
            }
        }
    }
    Ok(nodes)
}

fn capture(
    state: &SpatialState,
    world: &World,
    route: &ActorMove,
    stamp: &ClockStamp,
    preparing: bool,
) -> Result<Captured, ParticipantError> {
    local_policy(state)?;
    crate::physical::check_capture_clock(world, stamp, preparing)?;
    // A Direct placement must be moved by its simulation owner. An Interior
    // placement can remain unchanged only for an actual exit into a member.
    match state.parent_of(route.actor) {
        SpatialParent::Unplaced => {}
        SpatialParent::Interior { space, .. }
            if route.exit.is_some()
                && state
                    .spaces
                    .get(&space)
                    .is_some_and(|record| record.rooms.contains(&route.destination)) => {}
        _ => return Err(refused("locomotion requires a placement owner")),
    }
    let projections = projection_dependencies(state, world, route.actor)?;
    let mut facts = crate::locomotion_capture::derive(state, world, route, stamp)?;
    facts.read_dependencies.extend(projections);
    if facts.read_dependencies.len() > MAX_DEPENDENCIES {
        return Err(refused("locomotion dependency limit exceeded"));
    }
    crate::physical::check_affected_nodes(state, world, &facts.read_dependencies)?;
    let fence = StateFence::capture(state, facts.read_dependencies.clone())?;
    let plan_digest = DefinitionDigest::for_canonical_value(
        "sim.locomotion.plan.v1",
        &json!({
            "route":crate::locomotion_capture::plan_digest(route)?,
            "topology":fence.digest,
        }),
    )
    .map_err(|_| ParticipantError::Encoding)?;
    Ok(Captured {
        facts,
        plan_digest,
        fence,
    })
}

fn route_of(intent: &LocomotionIntent) -> ActorMove {
    ActorMove {
        actor: intent.actor(),
        source: intent.source(),
        exit: intent.exit(),
        destination: intent.destination(),
    }
}

fn audit_value(intent: &LocomotionIntent, captured: &Captured) -> Value {
    json!({
        "op":"locomotion_v1",
        "commit":intent.commit(),
        "action":intent.action(),
        "actor":intent.actor(),
        "source":intent.source(),
        "exit":intent.exit(),
        "destination":intent.destination(),
        "at_utc":intent.at_utc(),
        "prepared_at":intent.prepared_at(),
        "plan_digest":captured.plan_digest,
        "capture_digest":captured.facts.capture_digest,
    })
}

fn prepare_audit(
    state: &SpatialState,
    value: Value,
) -> Result<(PreparedLedgerAppend, ReplayWrite), ParticipantError> {
    let audit = state
        .recovery
        .prepare_append(RECOVERY_LOG, value)
        .map_err(|_| refused("locomotion audit is unavailable"))?;
    let write = audit
        .authoritative_replay_write(OwnerId::new(NS).map_err(|_| ParticipantError::Authority)?)
        .map_err(|_| ParticipantError::Encoding)?;
    Ok((audit, write))
}

struct Patch {
    intent: LocomotionIntent,
    plan_digest: DefinitionDigest,
    capture_digest: DefinitionDigest,
    audit: PreparedLedgerAppend,
    audit_write: ReplayWrite,
    fence: StateFence,
}

struct Bound<'a> {
    audit: BoundLedgerAppend<'a>,
}

impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        // The exact write is already in the acknowledged authoritative bundle.
        // Applying this prebound append has no callback or fallible work.
        let _ = self.audit.apply();
    }
}

struct LocomotionParticipant;

impl OwnerParticipant for LocomotionParticipant {
    type State = SpatialState;
    type Request = LocomotionIntent;
    type Output = LocomotionDraft;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;

    fn prepare(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        intent: LocomotionIntent,
    ) -> Result<OwnerPreparation<LocomotionDraft, Patch>, ParticipantError> {
        if intent.actor() != ctx.invocation().actor()
            || intent.action() != ctx.action()
            || intent.commit() != ctx.commit_token()
            || intent.at_utc() != ctx.world().now()
        {
            return Err(ParticipantError::Authority);
        }
        let captured = capture(
            state,
            ctx.world(),
            &route_of(&intent),
            intent.prepared_at(),
            true,
        )?;
        let output = LocomotionDraft::new(
            ctx,
            &intent,
            captured.plan_digest,
            captured.facts.capture_digest,
            captured.facts.read_dependencies.clone(),
        )?;
        let (audit, audit_write) = prepare_audit(state, audit_value(&intent, &captured))?;
        Ok(OwnerPreparation::new(
            output,
            Patch {
                intent,
                plan_digest: captured.plan_digest,
                capture_digest: captured.facts.capture_digest,
                audit,
                audit_write: audit_write.clone(),
                fence: captured.fence,
            },
            vec![audit_write],
        ))
    }

    fn recheck(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if patch.intent.actor() != ctx.invocation().actor()
            || patch.intent.action() != ctx.action()
            || patch.intent.commit() != ctx.commit_token()
            || ctx.world().now() < patch.intent.at_utc()
        {
            return Err(ParticipantError::Authority);
        }
        let captured = capture(
            state,
            ctx.world(),
            &route_of(&patch.intent),
            patch.intent.prepared_at(),
            false,
        )?;
        if captured.plan_digest != patch.plan_digest
            || captured.facts.capture_digest != patch.capture_digest
        {
            return Err(refused("locomotion capture changed"));
        }
        patch.fence.recheck(state)?;
        let (_, audit_write) = prepare_audit(state, audit_value(&patch.intent, &captured))?;
        if audit_write != patch.audit_write {
            return Err(refused("locomotion audit changed"));
        }
        Ok(())
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut SpatialState,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        patch.fence.recheck(state)?;
        // Reuse Ledger's exact prepared-tail binding. This avoids cloning the
        // complete SpatialState or rebuilding indexes for an audit-only write.
        let audit = state
            .recovery
            .bind_prepared(patch.audit)
            .map_err(|_| refused("locomotion audit changed"))?;
        Ok(Bound { audit })
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write, ReplayWrite::LedgerChunkSet { write }
            if write.namespace().as_str() == NS && write.log_id() == RECOVERY_LOG)
    }
}

struct Provider {
    outcome: DefinitionRef,
    service: CoreLocomotion<LocomotionParticipant>,
    crossings: LocomotionCrossings,
}

impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor
            || invocation.principal()
                != &(PrincipalId::Object {
                    id: invocation.actor(),
                })
            || !invocation.targets().is_empty()
            || invocation
                .parameters()
                .as_object()
                .is_none_or(|parameters| !parameters.is_empty())
        {
            return Err(unavailable("captured locomotion invocation refused"));
        }
        Ok(())
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let route = {
            let admission = context
                .admit_captured_locomotion(txn)
                .map_err(|_| unavailable("actual captured locomotion is required"))?;
            ActorMove {
                actor: admission.actor(),
                source: admission.source(),
                exit: admission.exit(),
                destination: admission.destination(),
            }
        };
        let captured = {
            let world = context
                .world()
                .map_err(|_| unavailable("locomotion world unavailable"))?;
            let state =
                crate::state(world).ok_or_else(|| unavailable("spatial state unavailable"))?;
            capture(state, world, &route, &world.world_ic_now(), true)
                .map_err(|_| unavailable("locomotion capture unavailable"))?
        };
        let receipt = context
            .prepare_captured_locomotion(txn, &self.service)
            .map_err(|_| unavailable("captured locomotion refused"))?;
        if receipt.actor() != route.actor
            || receipt.source() != route.source
            || receipt.exit() != route.exit
            || receipt.destination() != route.destination
            || receipt.prepared_at() != &captured.facts.prepared_at
            || receipt.plan_digest() != &captured.plan_digest
            || receipt.capture_digest() != &captured.facts.capture_digest
        {
            return Err(unavailable("locomotion receipt differs from capture"));
        }
        self.crossings
            .append(txn, &receipt, &captured.facts)
            .map_err(|_| unavailable("locomotion crossing could not be recorded"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor":receipt.actor(),"source":receipt.source(),"exit":receipt.exit(),
                "destination":receipt.destination(),"prepared_at":receipt.prepared_at(),
                "plan_digest":receipt.plan_digest(),"capture_digest":receipt.capture_digest(),
            }),
        )
        .map_err(|_| unavailable("locomotion outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests;
