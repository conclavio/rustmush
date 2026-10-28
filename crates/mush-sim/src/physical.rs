//! Closed structural transitions with same-commit personnel crossings (DT.2b2b.1).
//!
//! `@spatial/space` uses this preparation seam. It accepts actual transition
//! requests, never caller-built `Plan`s or object postimages. Other synchronous
//! movement callers retain their existing paths.
//! Version two pairs structural receipts with bounded crossing captures at the
//! actual preparation's world-IC stamp. These are not personnel embarkation
//! intervals, last-port proofs, mission admission, or unit authority.
//! Walking, raw movement/create/destroy, custody, journey and boot repair paths
//! must be migrated or refused before any personnel history is authoritative.
//! CarrierMove, Quarantine and Retire require their journey/destruction/domain
//! lifecycle owners. Station or assignment hosts (even empty records) and focus
//! require an atomic control-generation refresh. This initial seam refuses
//! those cases, including affected descendants and room occupants.
//! The current canonical planner rebuilds global indexes, so this foundation
//! also refuses worlds above an explicit resident spatial census/byte ceiling.
//! It never truncates topology to fit a preparation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::physical_transition::{
    CorePhysicalTransitions, MAX_PHYSICAL_READ_DEPENDENCIES, PhysicalMove, PhysicalTransitionDraft,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, AuthoritativeKey, BehaviorId, CanonicalHash, CanonicalId, ClockRef,
    ClockStamp, Ctx, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRef, Engine, Flags, IdempotencyKeyText, LegalActionQuery, ModError, ObjId, ObjKind,
    OwnerId, PerceivedRef, PrepareCtx, PreparedLedgerAppend, PrincipalId, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, ReplayWrite, RevisionSet, RulesValue, SubjectKey, World,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::data::{
    DomainId, InteriorSpaceRecord, PlacementRecord, RECOVERY_LOG, SpaceAnchor, SpatialParent,
    SpatialState, placement_key, space_key,
};
use crate::transition::{Plan, Transition};
use crate::{NS, graph};

const OUTCOME: &str = "physical_transition_result";
const MAX_PENDING: usize = 64;
const PENDING_SECONDS: i64 = 60;
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_DEPENDENCIES: usize = MAX_PHYSICAL_READ_DEPENDENCIES;
const MAX_ROOM_CONTENTS: usize = 512;
const MAX_FACT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESIDENT_RECORDS: usize = 4096;
const MAX_RESIDENT_BYTES: usize = 4 * 1024 * 1024;

/// Stop serialization before allocating/cloning an oversized input record.
fn encoded_size<T: Serialize + ?Sized>(value: &T, limit: usize) -> Result<usize, ParticipantError> {
    struct Limit {
        used: usize,
        limit: usize,
    }
    impl std::io::Write for Limit {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.used) {
                return Err(std::io::Error::other("structural byte limit"));
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Limit { used: 0, limit };
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| refused("structural byte limit exceeded"))?;
    Ok(writer.used)
}

fn resident_bound(state: &SpatialState) -> Result<(), ParticipantError> {
    let count = state
        .domains
        .len()
        .saturating_add(state.placements.len())
        .saturating_add(state.held_placements.len())
        .saturating_add(state.unbound_placement_docs.len())
        .saturating_add(state.held_assignments.len())
        .saturating_add(state.spaces.len())
        .saturating_add(state.mounts.len())
        .saturating_add(state.stations.len())
        .saturating_add(state.assignments.len())
        .saturating_add(state.control_gen.len());
    if count > MAX_RESIDENT_RECORDS {
        return Err(refused("resident spatial census exceeds foundation limit"));
    }
    let used = encoded_size(
        &(
            &state.domains,
            &state.placements,
            &state.spaces,
            &state.mounts,
            &state.stations,
            &state.assignments,
            &state.control_gen,
            &state.unbound_placement_docs,
        ),
        MAX_RESIDENT_BYTES,
    )?;
    let d = &state.derived;
    let used = used
        + encoded_size(
            &(
                &d.domain_by_host,
                &d.children,
                &d.domain_direct,
                &d.local_objects,
                &d.projected,
                &d.presentation_of,
                &d.anchored_spaces,
                &d.spaces_in_domain,
                &d.assignments_by_actor,
                &d.seats_by_actor,
                &d.member_of,
            ),
            MAX_RESIDENT_BYTES.saturating_sub(used),
        )?;
    let held: Vec<_> = state
        .held_placements
        .iter()
        .chain(state.held_assignments.iter())
        .map(|(key, row)| (key, &row.key, &row.reason))
        .collect();
    encoded_size(&held, MAX_RESIDENT_BYTES.saturating_sub(used))?;
    if !state.recovery.resident_within_bounds(8, MAX_RESIDENT_BYTES) {
        return Err(refused("resident spatial audit exceeds foundation limit"));
    }
    Ok(())
}

fn error(message: &'static str) -> ActionError {
    crate::tick::err(ActionErrorCode::Unavailable, message)
}
fn refused(message: &str) -> ParticipantError {
    ParticipantError::refused(message)
}
fn owner() -> Result<OwnerId, ParticipantError> {
    OwnerId::new(NS).map_err(|_| ParticipantError::Authority)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    at: i64,
    transition: Transition,
    before: DefinitionDigest,
}
/// Only the real provider constructs this request after reading current Sim
/// truth. The participant independently derives and verifies every capture.
#[derive(Clone, Debug, PartialEq)]
struct PreparedCrossingIntent {
    intent: Intent,
    prepared_at: ClockStamp,
    capture_digest: DefinitionDigest,
}
impl Intent {
    fn current(&self, now: i64) -> bool {
        now.checked_sub(self.at)
            .is_some_and(|age| (0..=PENDING_SECONDS).contains(&age))
    }
}
#[derive(Clone, Default)]
struct PhysicalPending(BTreeMap<ObjId, Intent>);
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    service: CorePhysicalTransitions<SpatialParticipant>,
    personnel: crate::personnel::PersonnelCrossings,
}

/// Retained immutable v1 identity; only v2 remains an authoring default.
pub(crate) fn outcome_descriptor_v1() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(OUTCOME).map_err(|e| ModError::msg(e.to_string()))?,
        1,
        json!({"fields":["root","structural_digest"],"personnel_eligibility":false}),
    )
    .map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(OUTCOME).map_err(|e| ModError::msg(e.to_string()))?,
        2,
        json!({"fields":["root","structural_digest"],"capture":"sim.personnel_crossings",
            "personnel_eligibility":false}),
    )
    .map(|d| d.as_current())
    .map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn participant_descriptor_v1() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(mush_core::physical_transition::PARTICIPANT)
            .map_err(|e| ModError::msg(e.to_string()))?,
        1,
        json!({"state":"SpatialState","documents":["placement","space"],
            "ledger":RECOVERY_LOG,"operations":["place","unplace","space","members"],
            "authority":"sim_admin","personnel_eligibility":false}),
    )
    .map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(mush_core::physical_transition::PARTICIPANT)
            .map_err(|e| ModError::msg(e.to_string()))?,
        2,
        json!({"state":"SpatialState","documents":["placement","space"],
            "ledger":RECOVERY_LOG,"operations":["place","unplace","space","members"],
            "protocol":"sim_structural_personnel_capture_v2","capture":"sim.personnel_crossings",
            "authority":"sim_admin","personnel_eligibility":false}),
    )
    .map_err(|e| ModError::msg(e.to_string()))
}

pub(crate) fn register(reg: &mut Registrar<'_>, ic_custody: bool) -> Result<(), ModError> {
    let descriptor = participant_descriptor()?;
    let historical = participant_descriptor_v1()?;
    let expected = [descriptor.clone(), historical.clone()];
    // A configured IC custody owner installs the one exact participant
    // validator for both descriptors, retaining its composition references.
    if !ic_custody {
        reg.definition_validator(
            DefinitionKind::new("owner_participant").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &mush_core::DefinitionValidationContext<'_>| {
                let definition = ctx.definition();
                if expected.iter().any(|expected| {
                    &definition.reference().key().id == expected.id()
                        && definition.reference().version() == expected.version()
                        && definition.payload() == expected.payload()
                        && definition.references() == expected.references()
                        && definition.is_current() == expected.is_current()
                }) {
                    Ok(())
                } else {
                    Err("unknown Sim structural participant definition".into())
                }
            },
        )?;
    }
    let historical_participant = reg.definition(historical)?;
    let historical_action =
        reg.definition(mush_core::physical_transition::action_descriptor_v1()?)?;
    reg.definition(outcome_descriptor_v1()?)?;
    let participant = reg.definition(descriptor)?;
    let action = reg.definition(mush_core::physical_transition::action_descriptor()?)?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    reg.owner_participant(
        participant.clone(),
        BTreeSet::from([action.clone()]),
        SpatialParticipant,
    )?;
    let service = CorePhysicalTransitions::<SpatialParticipant>::register_with_history(
        reg,
        action.clone(),
        participant,
        vec![(historical_action, historical_participant)],
    )?;
    let personnel = crate::personnel::PersonnelCrossings::register(reg)?;
    reg.action_provider(
        action.clone(),
        Provider {
            action: action.clone(),
            outcome,
            service: service.clone(),
            personnel: personnel.clone(),
        },
    )?;
    reg.state(Handles {
        action,
        service,
        personnel,
    });
    reg.state(PhysicalPending::default());
    Ok(())
}

pub(crate) fn loaded(ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
    let handles = ctx
        .w
        .module_state::<Handles>()
        .ok_or_else(|| ModError::msg("Sim structural handles missing"))?;
    handles
        .service
        .audit(ctx)
        .map_err(|e| ModError::msg(e.to_string()))?;
    handles
        .personnel
        .audit(ctx, &handles.service)
        .map_err(|e| ModError::msg(e.to_string()))
}

fn root(transition: &Transition) -> Result<ObjId, ParticipantError> {
    match transition {
        Transition::Place { obj, .. } | Transition::Unplace { obj, .. } => Ok(*obj),
        Transition::Space { space, .. } | Transition::Members { space, .. } => Ok(*space),
        Transition::CarrierMove { .. }
        | Transition::Quarantine { .. }
        | Transition::Retire { .. } => Err(refused("transition requires its lifecycle owner")),
    }
}

fn add(nodes: &mut BTreeSet<ObjId>, node: ObjId) -> Result<(), ParticipantError> {
    nodes.insert(node);
    if nodes.len() > MAX_DEPENDENCIES {
        return Err(refused("structural dependency limit exceeded"));
    }
    Ok(())
}

fn parent_node(state: &SpatialState, parent: &SpatialParent) -> Option<ObjId> {
    match parent {
        SpatialParent::Direct { domain, .. } => state.domains.get(domain).map(|d| d.host),
        SpatialParent::Interior { space, .. } => Some(*space),
        SpatialParent::Attached { host, .. } => Some(*host),
        SpatialParent::Unplaced => None,
    }
}

/// Capture the actual affected topology, including zero-projection re-anchors.
/// Every visited relation and room-content entry is charged before filtering.
fn dependencies(
    state: &SpatialState,
    world: &World,
    transition: &Transition,
) -> Result<(ObjId, BTreeSet<ObjId>, BTreeSet<DomainId>), ParticipantError> {
    let root = root(transition)?;
    let mut nodes: BTreeSet<_> = state
        .subtree_bounded(root, crate::journey::MAX_CARRIER_SUBTREE)
        .map_err(|_| refused("structural subtree limit exceeded"))?
        .into_iter()
        .collect();
    add(&mut nodes, root)?;
    let mut domains = BTreeSet::new();
    match transition {
        Transition::Place { parent, .. } => {
            if let Some(parent) = parent_node(state, parent) {
                add(&mut nodes, parent)?;
            }
            if let SpatialParent::Direct { domain, .. } = parent {
                domains.insert(*domain);
            }
        }
        Transition::Space { anchor, .. } => match anchor {
            SpaceAnchor::MovingObject { host } => add(&mut nodes, *host)?,
            SpaceAnchor::FixedDomain { domain, .. } => {
                domains.insert(*domain);
                if let Some(d) = state.domains.get(domain) {
                    add(&mut nodes, d.host)?;
                }
            }
            SpaceAnchor::Unanchored => {}
        },
        Transition::Members { rooms, .. } => {
            if rooms.len() > crate::data::MAX_SPACE_ROOMS {
                return Err(refused("interior room limit exceeded"));
            }
            for room in rooms {
                add(&mut nodes, *room)?;
            }
        }
        _ => {}
    }
    // Ancestors are individually depth-bounded. This includes both the former
    // and proposed carrier roots even when they occupy the same World room.
    for node in nodes.clone() {
        for ancestor in
            graph::ancestors(state, node).map_err(|_| refused("invalid structural ancestry"))?
        {
            add(&mut nodes, ancestor)?;
        }
    }
    let mut visited_relations = 0usize;
    for node in nodes.clone() {
        for child in state.children_of(node) {
            visited_relations += 1;
            if visited_relations > MAX_DEPENDENCIES {
                return Err(refused("structural relation limit exceeded"));
            }
            add(&mut nodes, *child)?;
        }
    }
    let mut rooms = BTreeSet::new();
    // Defining a new space also affects walkers already in its actual room.
    if let Transition::Space { space, .. } | Transition::Members { space, .. } = transition {
        add(&mut rooms, *space)?;
    }
    for node in nodes.clone() {
        if let Some(space) = state.spaces.get(&node) {
            let implicit_room = usize::from(!space.rooms.contains(&space.object));
            if space.rooms.len().saturating_add(implicit_room) > crate::data::MAX_SPACE_ROOMS {
                return Err(refused("interior room limit exceeded"));
            }
            // Legacy empty member sets still represent the space's own room.
            // The set deduplicates it against authored members, so contents
            // are visited and charged exactly once per actual room.
            add(&mut rooms, space.object)?;
            for room in &space.rooms {
                add(&mut rooms, *room)?;
            }
        }
        if let SpatialParent::Direct { domain, .. } = state.parent_of(node) {
            domains.insert(domain);
        }
        if let Some(SpaceAnchor::FixedDomain { domain, .. }) =
            state.spaces.get(&node).map(|s| &s.anchor)
        {
            domains.insert(*domain);
        }
        if let Some(domain) = state.derived.domain_by_host.get(&node) {
            domains.insert(*domain);
        }
    }
    if let Transition::Members {
        rooms: requested, ..
    } = transition
    {
        for room in requested {
            add(&mut rooms, *room)?;
        }
    }
    let mut visited_contents = 0usize;
    for room in rooms {
        add(&mut nodes, room)?;
        if let Some(object) = world.get(room) {
            for occupant in &object.contents {
                visited_contents += 1;
                if visited_contents > MAX_ROOM_CONTENTS {
                    return Err(refused("interior occupancy limit exceeded"));
                }
                let occupant_object = world
                    .get(*occupant)
                    .ok_or_else(|| refused("interior contents are inconsistent"))?;
                if occupant_object.location != Some(room) {
                    return Err(refused("interior contents are inconsistent"));
                }
                if occupant_object.kind.is_actor() {
                    add(&mut nodes, *occupant)?;
                }
            }
        }
    }
    Ok((root, nodes, domains))
}

/// Apply the same lifecycle and control exclusions to every actual affected
/// object, including people and hulls discovered only by physical capture.
fn check_affected_nodes(
    state: &SpatialState,
    world: &World,
    nodes: &BTreeSet<ObjId>,
) -> Result<(), ParticipantError> {
    if nodes.len() > MAX_DEPENDENCIES {
        return Err(refused("structural dependency limit exceeded"));
    }
    let focus = world.module_state::<crate::focus::FocusState>();
    let journeys = world.module_state::<crate::journey::JourneyBook>();
    if journeys.is_some_and(|book| !book.held.is_empty()) {
        return Err(refused("held journey facts require repair"));
    }
    for &node in nodes {
        if world
            .get(node)
            .is_none_or(|object| object.flags.contains(Flags::GOING))
        {
            return Err(refused("affected object is unavailable"));
        }
        if state.held_placements.contains_key(&node) || state.is_quarantined(node) {
            return Err(refused("affected placement requires recovery or repair"));
        }
        crate::deployed::refuse_if_custodied(world, node)
            .map_err(|_| refused("affected object is held by simulation authority"))?;
        // The existing placement/unplacement contract advances cgen for any
        // station or assignment host, including empty records. Until that
        // control owner joins this commit, no such host is admitted here.
        if state.stations.contains_key(&node)
            || state.assignments.contains_key(&node)
            || state.held_assignments.contains_key(&node)
            || focus.is_some_and(|f| f.has_focus_on(node) || f.focus_of_actor(node).is_some())
        {
            return Err(refused(
                "affected control requires an atomic control refresh",
            ));
        }
        if journeys.is_some_and(|book| {
            crate::journey::under_way_root_of(state, book, node).is_some()
                || (book.get(node).is_some() && !state.is_placed(node))
        }) {
            return Err(refused("affected journey requires its lifecycle owner"));
        }
    }
    Ok(())
}

fn current_facts(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    transition: &Transition,
) -> Result<(ObjId, DefinitionDigest), ParticipantError> {
    resident_bound(state)?;
    if !state.unbound_placement_docs.is_empty() {
        return Err(refused("unbound placement facts require repair"));
    }
    if !crate::commands::is_admin(world, actor)
        || world
            .get(actor)
            .is_none_or(|o| !o.kind.is_actor() || o.flags.contains(Flags::GOING))
    {
        return Err(refused("permission denied"));
    }
    // Keep the staff command's authored-space contract at the actual owner
    // boundary; a typed caller does not inherit checks from that command.
    match transition {
        Transition::Space { space, .. } | Transition::Members { space, .. } => {
            let interior =
                BehaviorId::new("sim.interior").map_err(|_| ParticipantError::Authority)?;
            if world.get(*space).is_none_or(|object| {
                object.kind != ObjKind::Room || object.flags.contains(Flags::GOING)
            }) || !world.has_behavior(*space, &interior)
            {
                return Err(refused("interior space must be a live authored room"));
            }
        }
        Transition::Unplace { by, .. } if *by != Some(actor) => {
            return Err(refused(
                "unplacement attribution must match the issuing actor",
            ));
        }
        _ => {}
    }
    let (root, nodes, domains) = dependencies(state, world, transition)?;
    check_affected_nodes(state, world, &nodes)?;
    let journeys = world.module_state::<crate::journey::JourneyBook>();
    let mut records = Vec::with_capacity(nodes.len());
    let mut relations = 0usize;
    let mut remaining_bytes = MAX_FACT_BYTES;
    for node in nodes {
        let presentation = state.derived.presentation_of.get(&node);
        relations = relations.saturating_add(state.children_of(node).len());
        if relations > MAX_DEPENDENCIES {
            return Err(refused("structural relation limit exceeded"));
        }
        if presentation.is_some_and(|v| v.len() > MAX_DEPENDENCIES) {
            return Err(refused("domain relation limit exceeded"));
        }
        let journey = journeys.and_then(|book| book.get(node));
        let used = encoded_size(
            &(
                world.get(node),
                state.placements.get(&node),
                state.spaces.get(&node),
                state.mounts.get(&node),
                state.stations.get(&node),
                state.assignments.get(&node),
                state.children_of(node),
                presentation,
                journey,
            ),
            remaining_bytes,
        )?;
        remaining_bytes = remaining_bytes.saturating_sub(used);
        records.push(json!({
            "object":node,"world":world.get(node),"placement":state.placements.get(&node),
            "held":state.held_placements.get(&node).map(|held| (&held.key, &held.reason)),"space":state.spaces.get(&node),
            "mounts":state.mounts.get(&node),"stations":state.stations.get(&node),
            "assignments":state.assignments.get(&node),"control_generation":state.control_gen.get(&node),
            "member_of":state.derived.member_of.get(&node),"domain_host":state.derived.domain_by_host.get(&node),
            "presentation":presentation,"children":state.children_of(node),"journey":journey,
        }));
    }
    let domain_records: Vec<_> = domains
        .into_iter()
        .map(|d| (d, state.domains.get(&d)))
        .collect();
    encoded_size(&domain_records, remaining_bytes)?;
    let value = json!({"records":records,"domains":domain_records,
        "recovery_room":state.recovery_room,"in_transit_room":state.in_transit_room});
    mush_core::canonical_json_bytes_with_limit(&value, MAX_FACT_BYTES)
        .map_err(|_| refused("structural facts exceed byte limit"))?;
    let digest = DefinitionDigest::for_canonical_value("sim.physical.before.v1", &value)
        .map_err(|_| refused("structural facts are invalid"))?;
    Ok((root, digest))
}

fn plan_value(plan: &Plan) -> Value {
    json!({"placements":plan.set_placements,"spaces":plan.set_spaces,"retire":plan.retire,
        "projections":plan.projections,"audit":plan.audit})
}

fn evaluate(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    intent: &Intent,
) -> Result<(ObjId, Plan, DefinitionDigest), ParticipantError> {
    if !intent.current(world.now()) {
        return Err(refused("structural request expired"));
    }
    let (root, before) = current_facts(state, world, actor, &intent.transition)?;
    if before != intent.before {
        return Err(refused("structural pre-state changed"));
    }
    let plan = crate::transition::plan_at(state, world, &intent.transition, intent.at)
        .map_err(|_| refused("structural transition refused"))?;
    if !plan.retire.is_empty()
        || plan.projections.len() > crate::journey::MAX_CARRIER_SUBTREE
        || plan.set_placements.len() + plan.set_spaces.len() > MAX_DEPENDENCIES
    {
        return Err(refused("structural transition exceeds bounds"));
    }
    let value = json!({"before":before,"plan":plan_value(&plan)});
    mush_core::canonical_json_bytes_with_limit(&value, MAX_FACT_BYTES)
        .map_err(|_| refused("structural patch exceeds byte limit"))?;
    let digest = DefinitionDigest::for_canonical_value("sim.physical.transition.v1", &value)
        .map_err(|_| refused("structural patch is invalid"))?;
    Ok((root, plan, digest))
}

struct CapturedPlan {
    root: ObjId,
    plan: Plan,
    plan_digest: DefinitionDigest,
    capture: crate::personnel_capture::CaptureFacts,
    digest: DefinitionDigest,
    nodes: BTreeSet<ObjId>,
    domains: BTreeSet<DomainId>,
}

/// Preparation stamps are exact live truth; later rechecks retain that stamp
/// and permit only forward progress within its original world-IC generation.
fn check_capture_clock(
    world: &World,
    prepared_at: &ClockStamp,
    preparing: bool,
) -> Result<(), ParticipantError> {
    let now = world.world_ic_now();
    if !matches!(prepared_at.clock, ClockRef::WorldIc { generation } if generation != 0)
        || now.clock != prepared_at.clock
        || now.at < prepared_at.at
        || (preparing && now.at != prepared_at.at)
    {
        return Err(refused("personnel capture clock changed"));
    }
    Ok(())
}

fn capture_plan(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    intent: &Intent,
    prepared_at: &ClockStamp,
) -> Result<CapturedPlan, ParticipantError> {
    let (root, plan, plan_digest) = evaluate(state, world, actor, intent)?;
    let (_, mut nodes, mut domains) = dependencies(state, world, &intent.transition)?;
    let capture = crate::personnel_capture::derive(
        state,
        world,
        &intent.transition,
        &plan,
        prepared_at,
        &nodes,
    )?;
    for node in &capture.read_dependencies {
        add(&mut nodes, *node)?;
    }
    // Capturing dependencies is not admission: a physically nested hull or
    // unplaced cabin occupant may be absent from the structural subtree.
    check_affected_nodes(state, world, &nodes)?;
    // Capture may visit an actual walker's enclosing space even when the
    // walker has no structural placement. Fence those canonical rows at bind
    // too, including any domain record an added ancestry node consulted.
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
        if domains.len() > MAX_DEPENDENCIES {
            return Err(refused("structural domain dependency limit exceeded"));
        }
    }
    let digest = crate::personnel::combined_digest(&plan_digest, &capture.capture_digest)?;
    Ok(CapturedPlan {
        root,
        plan,
        plan_digest,
        capture,
        digest,
        nodes,
        domains,
    })
}

fn evaluate_capture(
    state: &SpatialState,
    world: &World,
    actor: ObjId,
    request: &PreparedCrossingIntent,
    preparing: bool,
) -> Result<CapturedPlan, ParticipantError> {
    check_capture_clock(world, &request.prepared_at, preparing)?;
    let captured = capture_plan(state, world, actor, &request.intent, &request.prepared_at)?;
    if captured.capture.capture_digest != request.capture_digest {
        return Err(refused("personnel capture facts changed"));
    }
    Ok(captured)
}

fn write(key: String, value: &impl Serialize) -> Result<ReplayWrite, ParticipantError> {
    Ok(ReplayWrite::ModuleJsonSet {
        namespace: owner()?,
        key: AuthoritativeKey::parse_canonical(key).map_err(|_| ParticipantError::Encoding)?,
        value: serde_json::to_value(value).map_err(|_| ParticipantError::Encoding)?,
    })
}

struct SpatialParticipant;
struct Patch {
    intent: PreparedCrossingIntent,
    digest: DefinitionDigest,
    plan: Plan,
    audit: PreparedLedgerAppend,
    audit_write: ReplayWrite,
    fence: StateFence,
}
struct StateFence {
    nodes: BTreeSet<ObjId>,
    domains: BTreeSet<DomainId>,
    digest: DefinitionDigest,
}
fn state_digest(
    state: &SpatialState,
    nodes: &BTreeSet<ObjId>,
    domains: &BTreeSet<DomainId>,
) -> Result<DefinitionDigest, ParticipantError> {
    let mut records = Vec::with_capacity(nodes.len());
    let mut relations = 0usize;
    for node in nodes {
        relations = relations.saturating_add(state.children_of(*node).len());
        relations = relations.saturating_add(
            state
                .derived
                .anchored_spaces
                .get(node)
                .map_or(0, BTreeSet::len),
        );
        if relations > MAX_DEPENDENCIES {
            return Err(refused("structural relation limit exceeded"));
        }
        records.push(json!({"object":node,"placement":state.placements.get(node),
            "held":state.held_placements.get(node).map(|held| (&held.key,&held.reason)),
            "space":state.spaces.get(node),"mounts":state.mounts.get(node),
            "stations":state.stations.get(node),"assignments":state.assignments.get(node),
            "held_assignments":state.held_assignments.get(node).map(|held| (&held.key,&held.reason)),
            "control_generation":state.control_gen.get(node),"children":state.children_of(*node),
            "anchored_spaces":state.derived.anchored_spaces.get(node),
            "domain_by_host":state.derived.domain_by_host.get(node),
            "presentation_of":state.derived.presentation_of.get(node),
            "member_of":state.derived.member_of.get(node)}));
    }
    let mut domain_records = Vec::with_capacity(domains.len());
    for domain in domains {
        relations = relations.saturating_add(
            state
                .derived
                .domain_direct
                .get(domain)
                .map_or(0, BTreeSet::len),
        );
        relations = relations.saturating_add(
            state
                .derived
                .spaces_in_domain
                .get(domain)
                .map_or(0, BTreeSet::len),
        );
        if relations > MAX_DEPENDENCIES {
            return Err(refused("structural relation limit exceeded"));
        }
        domain_records.push(json!({"id":domain,"record":state.domains.get(domain),
            "direct":state.derived.domain_direct.get(domain),"spaces":state.derived.spaces_in_domain.get(domain)}));
    }
    let value = json!({"records":records,"domains":domain_records,
        "recovery_room":state.recovery_room,"in_transit_room":state.in_transit_room});
    mush_core::canonical_json_bytes_with_limit(&value, MAX_FACT_BYTES)
        .map_err(|_| refused("structural facts exceed byte limit"))?;
    DefinitionDigest::for_canonical_value("sim.physical.local.v1", &value)
        .map_err(|_| ParticipantError::Encoding)
}
struct Bound<'a> {
    state: &'a mut SpatialState,
    after: Box<SpatialState>,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        *self.state = *self.after;
    }
}
impl OwnerParticipant for SpatialParticipant {
    type State = SpatialState;
    type Request = PreparedCrossingIntent;
    type Output = PhysicalTransitionDraft;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;

    fn prepare(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        intent: PreparedCrossingIntent,
    ) -> Result<OwnerPreparation<Self::Output, Patch>, ParticipantError> {
        let actor = ctx.invocation().actor();
        require_pending(ctx.world(), actor, &intent.intent)?;
        let CapturedPlan {
            root,
            plan,
            digest,
            nodes,
            domains,
            ..
        } = evaluate_capture(state, ctx.world(), actor, &intent, true)?;
        let moves = plan
            .projections
            .iter()
            .filter_map(|&(object, destination)| {
                let source = ctx.world().get(object).and_then(|o| o.location);
                (source != Some(destination)).then_some(PhysicalMove {
                    object,
                    source,
                    destination,
                })
            })
            .collect();
        let output = PhysicalTransitionDraft::new(ctx, root, digest, moves, nodes.clone())?;
        let mut writes = Vec::new();
        for placement in &plan.set_placements {
            writes.push(write(
                placement_key(placement.object).to_string(),
                placement,
            )?);
        }
        for space in &plan.set_spaces {
            writes.push(write(space_key(space.object).to_string(), space)?);
        }
        let audit = state
            .recovery
            .prepare_append(RECOVERY_LOG, plan.audit.clone())
            .map_err(|_| refused("structural audit unavailable"))?;
        let audit_write = audit
            .authoritative_replay_write(owner()?)
            .map_err(|_| ParticipantError::Encoding)?;
        writes.push(audit_write.clone());
        let fence = StateFence {
            digest: state_digest(state, &nodes, &domains)?,
            nodes,
            domains,
        };
        Ok(OwnerPreparation::new(
            output,
            Patch {
                intent,
                digest,
                plan,
                audit,
                audit_write,
                fence,
            },
            writes,
        ))
    }

    fn recheck(
        &self,
        state: &SpatialState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        require_pending(ctx.world(), ctx.invocation().actor(), &patch.intent.intent)?;
        let CapturedPlan { plan, digest, .. } = evaluate_capture(
            state,
            ctx.world(),
            ctx.invocation().actor(),
            &patch.intent,
            false,
        )?;
        if digest != patch.digest {
            return Err(refused("structural plan changed"));
        }
        if state_digest(state, &patch.fence.nodes, &patch.fence.domains)? != patch.fence.digest {
            return Err(refused("structural local state changed"));
        }
        let current = state
            .recovery
            .prepare_append(RECOVERY_LOG, plan.audit)
            .map_err(|_| refused("structural audit unavailable"))?
            .authoritative_replay_write(owner()?)
            .map_err(|_| ParticipantError::Encoding)?;
        if current != patch.audit_write {
            return Err(refused("structural audit changed"));
        }
        Ok(())
    }

    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut SpatialState,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        resident_bound(state)?;
        if !state.unbound_placement_docs.is_empty() {
            return Err(refused("unbound placement facts require repair"));
        }
        if state_digest(state, &patch.fence.nodes, &patch.fence.domains)? != patch.fence.digest {
            return Err(refused("structural local state changed"));
        }
        // Preserve unrelated current records. No lookup, serialization, rebuild
        // or fallible ledger operation remains after the first owner applies.
        let mut after = Box::new(state.clone());
        for placement in patch.plan.set_placements {
            after.placements.insert(placement.object, placement);
        }
        for space in patch.plan.set_spaces {
            after.spaces.insert(space.object, space);
        }
        after
            .recovery
            .apply_prepared(patch.audit)
            .map_err(|_| refused("structural audit changed"))?;
        after.derived = graph::rebuild(&after);
        Ok(Bound { state, after })
    }

    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == NS => {
                let key = key.as_str();
                if let Ok(record) = serde_json::from_value::<PlacementRecord>(value.clone()) {
                    return key == placement_key(record.object).as_str()
                        && serde_json::to_value(&record).ok().as_ref() == Some(value);
                }
                if let Ok(record) = serde_json::from_value::<InteriorSpaceRecord>(value.clone()) {
                    return key == space_key(record.object).as_str()
                        && serde_json::to_value(&record).ok().as_ref() == Some(value);
                }
                false
            }
            ReplayWrite::LedgerChunkSet { write } => {
                write.namespace().as_str() == NS && write.log_id() == RECOVERY_LOG
            }
            _ => false,
        }
    }
}

fn fingerprint(actor: ObjId, intent: &Intent) -> Result<String, ActionError> {
    CanonicalHash::for_canonical_value(
        "sim.physical.request.v1",
        1,
        &json!({"actor":actor,"intent":intent}),
        MAX_REQUEST_BYTES,
    )
    .map(|hash| hash.to_hex())
    .map_err(|_| error("structural input exceeds bounds"))
}
fn require_pending(world: &World, actor: ObjId, intent: &Intent) -> Result<(), ParticipantError> {
    if world
        .module_state::<PhysicalPending>()
        .and_then(|p| p.0.get(&actor))
        != Some(intent)
    {
        return Err(refused("structural request changed"));
    }
    Ok(())
}

/// The v2 durable outcome identifies the changed root, but does not record
/// whether a space was new or re-anchored. Project only what it proves, without
/// consulting current names or adding fields to that retained schema.
fn project_structural_receipt(
    expected_action: &DefinitionRef,
    expected_outcome: &DefinitionRef,
    action: &DefinitionRef,
    outcome: &RulesValue,
) -> Result<mush_proto::ProjectedActionResult, ActionError> {
    let invalid = || crate::tick::err(ActionErrorCode::Integrity, "structural receipt is invalid");
    if action != expected_action || outcome.schema() != expected_outcome {
        return Err(invalid());
    }
    let fields = outcome
        .data()
        .as_object()
        .filter(|fields| fields.len() == 2)
        .ok_or_else(invalid)?;
    let root = fields
        .get("root")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .map(ObjId)
        .ok_or_else(invalid)?;
    let _: DefinitionDigest =
        serde_json::from_value(fields.get("structural_digest").ok_or_else(invalid)?.clone())
            .map_err(|_| invalid())?;
    mush_proto::ProjectedActionResult::new(
        "sim.physical_transition",
        format!("Spatial update for {root} committed."),
        json!({"root":root}),
    )
    .map_err(|_| {
        crate::tick::err(
            ActionErrorCode::BoundsExceeded,
            "structural receipt exceeds public bounds",
        )
    })
}

struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    service: CorePhysicalTransitions<SpatialParticipant>,
    personnel: crate::personnel::PersonnelCrossings,
}
impl Provider {
    fn input(&self, invocation: &ActionInvocation) -> Result<Intent, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || invocation.principal()
                != &(PrincipalId::Object {
                    id: invocation.actor(),
                })
            || invocation
                .parameters()
                .as_object()
                .is_none_or(|p| p.len() != 2)
        {
            return Err(error("structural invocation refused"));
        }
        let value = invocation
            .parameters()
            .get("input")
            .ok_or_else(|| error("structural input missing"))?;
        mush_core::canonical_json_bytes_with_limit(value, MAX_REQUEST_BYTES)
            .map_err(|_| error("structural input exceeds bounds"))?;
        let input: Intent =
            serde_json::from_value(value.clone()).map_err(|_| error("structural input invalid"))?;
        if invocation
            .parameters()
            .get("request")
            .and_then(Value::as_str)
            != Some(fingerprint(invocation.actor(), &input)?.as_str())
        {
            return Err(error("structural request differs"));
        }
        Ok(input)
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.input(invocation).map(|_| ())
    }
    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        _viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        // Core admits only the event-time eligible viewer. The durable v2
        // outcome contains no actor identity to infer or replace that check.
        project_structural_receipt(&self.action, &self.outcome, action, outcome).map(Some)
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.projected_subject() != &PerceivedRef::SelfActor
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
        {
            return Ok(None);
        }
        let Some(intent) = engine
            .world
            .module_state::<PhysicalPending>()
            .and_then(|p| p.0.get(&query.actor()))
        else {
            return Ok(None);
        };
        let Some(state) = crate::state(&engine.world) else {
            return Ok(None);
        };
        if evaluate(state, &engine.world, query.actor(), intent).is_err() {
            return Ok(None);
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| error("structural definitions unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| error("structural guards unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| error("structural definition unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({"input":intent}),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("request")
                    .map_err(|_| error("structural parameter invalid"))?,
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
            }])
            .map_err(|_| error("structural parameters invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "sim-physical",
                "Prepare structural transition",
                "Commit the checked Sim topology and matching physical projection.",
            )
            .map_err(|_| error("structural presentation invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| error("structural guards invalid"))?,
        }))
    }
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let input = self.input(invocation)?;
        let captured = {
            let world = context
                .world()
                .map_err(|_| error("structural world unavailable"))?;
            let state = crate::state(world).ok_or_else(|| error("spatial state unavailable"))?;
            capture_plan(
                state,
                world,
                invocation.actor(),
                &input,
                &world.world_ic_now(),
            )
            .map_err(|_| error("personnel capture unavailable"))?
        };
        let request = PreparedCrossingIntent {
            intent: input,
            prepared_at: captured.capture.prepared_at.clone(),
            capture_digest: captured.capture.capture_digest,
        };
        let receipt = context
            .prepare_physical_transition(&self.service, txn, request)
            .map_err(|_| error("structural transition refused"))?;
        if receipt.structural_digest() != &captured.digest {
            return Err(error("personnel capture receipt differs"));
        }
        self.personnel
            .append(txn, &receipt, &captured.capture, &captured.plan_digest)
            .map_err(|_| error("personnel capture could not be recorded"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "root":receipt.root(),"structural_digest":receipt.structural_digest(),
            }),
        )
        .map_err(|_| error("structural outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// Stage an admin's bounded request through the ordinary authenticated action
/// coordinator. Successful submission does not mutate placement or location;
/// installation waits for the writer's exact durable acknowledgement.
pub fn submit(ctx: &mut Ctx<'_>, transition: Transition) -> Result<(), String> {
    if ctx.dispatch_actor() != Some(ctx.actor) {
        return Err("Structural request requires the actual command actor.".into());
    }
    encoded_size(&transition, MAX_REQUEST_BYTES).map_err(|e| e.to_string())?;
    if ctx.authoritative_pending(ctx.actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("An authoritative request is already pending.".into());
    }
    let handles = ctx
        .w
        .module_state::<Handles>()
        .cloned()
        .ok_or("Structural preparation unavailable.")?;
    let state = crate::state(ctx.w).ok_or("Spatial state unavailable.")?;
    let (_, before) =
        current_facts(state, ctx.w, ctx.actor, &transition).map_err(|e| e.to_string())?;
    let input = Intent {
        at: ctx.w.now(),
        transition,
        before,
    };
    // Only run this after current_facts has bounded the resident state and
    // request. Preserve the canonical staff-facing planner refusal here;
    // provider preparation and final take independently derive the plan again.
    crate::transition::plan_at(state, ctx.w, &input.transition, input.at)
        .map_err(|error| error.to_string())?;
    evaluate(state, ctx.w, ctx.actor, &input).map_err(|e| e.to_string())?;
    let digest = fingerprint(ctx.actor, &input).map_err(|e| e.to_string())?;
    let now = ctx.w.now();
    let pending = ctx
        .w
        .module_state_mut::<PhysicalPending>()
        .ok_or("Structural preparation unavailable.")?;
    pending.0.retain(|_, input| input.current(now));
    if pending.0.len() >= MAX_PENDING && !pending.0.contains_key(&ctx.actor) {
        return Err("Structural preparation is busy.".into());
    }
    pending.0.insert(ctx.actor, input);
    let key = IdempotencyKeyText::parse_canonical(format!("sim-physical/{digest}"))
        .map_err(|e| e.to_string())?;
    ctx.submit_authoritative(
        handles.action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"request":digest}),
        key,
    );
    Ok(())
}

#[cfg(test)]
#[path = "physical_tests.rs"]
mod tests;
