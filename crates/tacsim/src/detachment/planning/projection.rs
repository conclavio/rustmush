//! Live preparation observations, separate from permission and launch admission.
//!
//! The caller must first obtain the draft from the authorized planning reader.
//! No cached preparation flag, station projection or voyage remote exception is
//! an authority source. These reports deliberately use the draft's object IDs,
//! never a name lookup that could reveal an inaccessible character or hull.

use mush_core::{AccountId, ChannelId, ClockStamp, Ctx, Flags, ObjId, ObjKind, World};
use mush_org::unit_authority::UnitCapability;
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
use mush_sim::data::{SpatialState, StationAssignment, StationId};
use mush_sim::focus::{FocusState, ReachProof};
use mush_tactics::detachment::planning::{ConsentState, Draft, PlannedController};
use serde::Serialize;

/// An ephemeral physical preimage, re-read at offer, prepare and final validation.
/// This is neither a lease nor a portable authorization token. The mutation's
/// final `ControlLease` witness must independently guard Sim's actual helm lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ReadyStationProof {
    actor: ObjId,
    unit: ObjId,
    station: StationId,
    console: ObjId,
    assignment: StationAssignment,
    control_generation: u64,
    focus_account: AccountId,
    focus_acquired_at: ClockStamp,
}

impl ReadyStationProof {
    #[must_use]
    pub(crate) const fn unit(&self) -> ObjId {
        self.unit
    }
}

fn live(world: &World, object: ObjId) -> bool {
    world
        .get(object)
        .is_some_and(|record| !record.flags.contains(Flags::GOING))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presence {
    Aboard,
    Away,
    Unavailable,
}

impl Presence {
    const fn label(self) -> &'static str {
        match self {
            Self::Aboard => "physically aboard",
            Self::Away => "not physically aboard",
            Self::Unavailable => "physical presence unavailable",
        }
    }
}

/// Current physical presence, including nested hulls and actual interior rooms.
/// A remote focus never contributes an edge. An unplaced walker is observed from
/// its current room; a placed actor must also agree with Sim's projection.
fn presence(world: &World, object: ObjId, carrier: ObjId) -> Presence {
    let Some(state) = world.module_state::<SpatialState>() else {
        return Presence::Unavailable;
    };
    let Some(record) = world.get(object) else {
        return Presence::Unavailable;
    };
    if !live(world, object)
        || !live(world, carrier)
        || (record.kind != ObjKind::Thing && !record.kind.is_actor())
        || world
            .get(carrier)
            .is_none_or(|record| record.kind != ObjKind::Thing)
        || state.is_quarantined(object)
        || state.is_quarantined(carrier)
    {
        return Presence::Unavailable;
    }
    if object == carrier {
        return Presence::Away;
    }
    let start = if state.is_placed(object) {
        if mush_sim::graph::project(state, world, object) != record.location {
            return Presence::Unavailable;
        }
        object
    } else if record.kind.is_actor() {
        let Some(room) = record.location else {
            return Presence::Unavailable;
        };
        if !live(world, room)
            || world
                .get(room)
                .is_none_or(|room| room.kind != ObjKind::Room)
        {
            return Presence::Unavailable;
        }
        let space = state.derived.member_of.get(&room).copied().unwrap_or(room);
        let Some(interior) = state.spaces.get(&space) else {
            return Presence::Away;
        };
        if !interior.rooms.contains(&room) && !(room == space && interior.rooms.is_empty()) {
            return Presence::Unavailable;
        }
        space
    } else {
        return Presence::Away;
    };
    let Ok(ancestors) = mush_sim::graph::ancestors(state, start) else {
        return Presence::Unavailable;
    };
    if !live(world, start)
        || state.is_quarantined(start)
        || ancestors.iter().any(|ancestor| {
            !live(world, *ancestor)
                || state.is_quarantined(*ancestor)
                || (state.is_placed(*ancestor)
                    && mush_sim::graph::project(state, world, *ancestor)
                        != world.get(*ancestor).and_then(|record| record.location))
        })
    {
        return Presence::Unavailable;
    }
    if ancestors.contains(&carrier) {
        Presence::Aboard
    } else {
        Presence::Away
    }
}

/// Physical observation only. It does not establish an embarkation interval or
/// presence at the carrier's last-port departure, and grants no access rights.
#[must_use]
pub(crate) fn physically_aboard(world: &World, object: ObjId, carrier: ObjId) -> bool {
    presence(world, object, carrier) == Presence::Aboard
}

/// The assigned player is personally at the current physical helm console.
/// No `ChannelGrant` or `seat::channel_lease_holder` mirror is consulted.
#[must_use]
pub(crate) fn ready_station(
    world: &World,
    actor: ObjId,
    draft: &Draft,
) -> Option<ReadyStationProof> {
    let unit = draft.assigned_player(actor)?.unit?;
    if world.get(actor)?.kind != ObjKind::Player
        || world.get(unit)?.kind != ObjKind::Thing
        || mush_tactics::module::unit_of(world, unit).is_none()
        || !physically_aboard(world, unit, draft.plan.carrier)
        || !physically_aboard(world, actor, draft.plan.carrier)
    {
        return None;
    }
    physical_station(world, actor, unit)
}

fn physical_station(world: &World, actor: ObjId, unit: ObjId) -> Option<ReadyStationProof> {
    if !live(world, actor) || !live(world, unit) {
        return None;
    }
    let state = world.module_state::<SpatialState>()?;
    if state.is_quarantined(unit) || state.held_assignment(unit).is_some() {
        return None;
    }
    let focus = world.module_state::<FocusState>()?.focus_of_actor(actor)?;
    if focus.unit != unit || focus.actor != actor || focus.driver != actor || focus.proxy {
        return None;
    }
    let ReachProof::Console { room } = &focus.reach else {
        return None;
    };
    let host = state.stations.get(&unit)?;
    let assignments = state.assignments.get(&unit)?;
    if host.host != unit || assignments.host != unit {
        return None;
    }
    let spec = host.stations.get(&focus.station)?;
    let assignment = assignments.seats.get(&focus.station)?;
    assignment.check().ok()?;
    let helm = ChannelId::new("helm")?;
    // Membership checks avoid cloning an authored channel set just to test helm.
    if assignment.actor != actor
        || !spec.channels.contains(&helm)
        || assignment
            .granted
            .as_ref()
            .is_some_and(|channels| !channels.contains(&helm))
    {
        return None;
    }
    let console = spec
        .space
        .or_else(|| world.get(unit).and_then(|unit| unit.location))?;
    if *room != console
        || !live(world, console)
        || world.get(console)?.kind != ObjKind::Room
        || world.get(actor)?.location != Some(console)
    {
        return None;
    }
    Some(ReadyStationProof {
        actor,
        unit,
        station: focus.station.clone(),
        console,
        assignment: assignment.clone(),
        control_generation: state.control_generation(unit),
        focus_account: focus.account,
        focus_acquired_at: focus.acquired_at.clone(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Permission {
    Allowed,
    Denied,
    Unavailable,
}

impl Permission {
    const fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed now",
            Self::Denied => "denied now",
            Self::Unavailable => "unresolved (authority service unavailable)",
        }
    }
}

fn permission(ctx: &Ctx<'_>, unit: ObjId, capability: UnitCapability) -> Permission {
    let Some(reader) = ctx
        .w
        .module_state::<super::PlanningHandles>()
        .and_then(|handles| handles.unit_authority_reader.as_ref())
    else {
        return Permission::Unavailable;
    };
    match ctx.read_owner(
        reader,
        UnitAuthorityRequest::Authorize {
            unit,
            capability,
            delegate: false,
            mission: None,
        },
    ) {
        Ok(UnitAuthorityResponse::Authority { allowed: true }) => Permission::Allowed,
        Ok(UnitAuthorityResponse::Authority { allowed: false }) => Permission::Denied,
        _ => Permission::Unavailable,
    }
}

/// The same local-or-examinable boundary as an explicit `look #dbref`, plus
/// the current perception hooks. Merely mentioning an ID in a plan is no read
/// capability. In particular, knowing a person's ID never tracks them remotely.
fn observation_allowed(world: &World, viewer: ObjId, object: ObjId, perceives: bool) -> bool {
    let Some(record) = world.get(object) else {
        return false;
    };
    let examinable = mush_core::perm::can_examine(world, viewer, object);
    !record.flags.contains(Flags::GOING)
        && (viewer == object || perceives)
        && (mush_core::is_nearby(world, viewer, object) || examinable)
        && (!record.flags.contains(Flags::DARK) || viewer == object || examinable)
}

const UNOBSERVED: &str = "unavailable (missing or not visible)";

fn report_lines(
    world: &World,
    viewer: ObjId,
    draft: &Draft,
    observe: impl Fn(ObjId) -> bool,
    mut authorize: impl FnMut(ObjId, UnitCapability) -> Permission,
) -> Result<Vec<String>, String> {
    draft.validate().map_err(|error| error.to_string())?;
    if !draft.visible_to(viewer) {
        return Err("Preparation report unavailable.".to_owned());
    }
    let carrier = draft.plan.carrier;
    let observed = |object| live(world, object) && observe(object);
    let carrier_observed = observed(carrier)
        && world
            .get(carrier)
            .is_some_and(|record| record.kind == ObjKind::Thing);
    let mut lines = vec![
        format!(
            "Preparation observations for draft {} revision {}. This is not launch admission.",
            draft.id.0, draft.revision,
        ),
        format!(
            "Carrier {carrier}: {}. You are {}.",
            if carrier_observed {
                "exists now"
            } else {
                UNOBSERVED
            },
            if carrier_observed {
                presence(world, viewer, carrier).label()
            } else {
                UNOBSERVED
            },
        ),
    ];
    for slot in &draft.plan.slots {
        let unit_text = slot.unit.map_or_else(
            || "unit unassigned".to_owned(),
            |unit| {
                let label = if observed(unit)
                    && carrier_observed
                    && mush_tactics::module::unit_of(world, unit).is_some()
                {
                    presence(world, unit, carrier).label()
                } else {
                    UNOBSERVED
                };
                format!("unit {unit}: {label}")
            },
        );
        let controller_text = match slot.controller {
            PlannedController::Unassigned => "controller unassigned".to_owned(),
            PlannedController::Automated => "automated controller requested".to_owned(),
            PlannedController::Player { character } | PlannedController::Npc { character } => {
                let expected = if matches!(slot.controller, PlannedController::Player { .. }) {
                    ObjKind::Player
                } else {
                    ObjKind::Npc
                };
                let label = if observed(character)
                    && carrier_observed
                    && world
                        .get(character)
                        .is_some_and(|person| person.kind == expected)
                {
                    presence(world, character, carrier).label()
                } else {
                    UNOBSERVED
                };
                format!("character {character}: {label}")
            }
        };
        lines.push(format!(
            "Slot {} — {unit_text}; {controller_text}.",
            slot.slot.get()
        ));
        if let Some(unit) = slot.unit {
            if viewer == draft.organizer {
                lines.push(format!(
                    "  Your deploy permission for {unit}: {}.",
                    authorize(unit, UnitCapability::Deploy).label()
                ));
            }
            if matches!(slot.controller, PlannedController::Player { character } if character == viewer)
            {
                lines.push(format!("  Your operate/crew permission for {unit}: {}. Physical helm station aboard this carrier: {}.",
                    authorize(unit, UnitCapability::OperateCrew).label(),
                    if observed(unit) && carrier_observed && ready_station(world, viewer, draft).is_some()
                        { "present now" } else { "not established" }));
            } else {
                lines.push("  Other principals' operate/crew permission: unresolved; this report only checks your own authority.".to_owned());
            }
        }
        if let PlannedController::Player { character } = slot.controller {
            let declaration = draft
                .consents
                .get(&character)
                .filter(|consent| {
                    consent.revision == draft.revision && consent.plan_hash == draft.plan_hash
                })
                .map_or("not accepted", |consent| match consent.state {
                    ConsentState::Accepted => "accepted",
                    ConsentState::Ready => "ready declared for this exact revision",
                    ConsentState::Declined => "declined",
                });
            lines.push(format!("  Player {character}: {declaration}."));
        }
    }
    if let Some(commander) = draft.plan.npc_commander {
        lines.push(format!(
            "Requested NPC commander {commander}: {}.",
            if observed(commander)
                && carrier_observed
                && world
                    .get(commander)
                    .is_some_and(|person| person.kind == ObjKind::Npc)
            {
                presence(world, commander, carrier).label()
            } else {
                UNOBSERVED
            }
        ));
    }
    lines.extend([
        "Before activation: recheck current carrier containment, NPC contracts, agreed duties and capability checks. No boarding-history evidence is required.".to_owned(),
        "Unresolved before activation: mind licences, installed controller/profile compatibility, financial responsibility and explicit mission-command grants.".to_owned(),
        "Unresolved before activation: unit reservations and complete atomic launch preflight. No units are reserved or launched by this report.".to_owned(),
        "A ready declaration checks the real helm lease when committed. Current station, lease, permissions and every launch prerequisite must be checked again at activation.".to_owned(),
    ]);
    Ok(lines)
}

/// Render only after the planning owner-read service authorized this exact draft.
/// Org reads always retain the authenticated viewer; there is no impersonation
/// of an assigned pilot, organizer or NPC to fill someone else's permission row.
pub fn report(ctx: &mut Ctx<'_>, draft: &Draft) -> Result<(), String> {
    let lines = report_lines(
        ctx.w,
        ctx.actor,
        draft,
        |object| {
            observation_allowed(
                ctx.w,
                ctx.actor,
                object,
                ctx.can_perceive(ctx.actor, object),
            )
        },
        |unit, capability| permission(ctx, unit, capability),
    )?;
    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.detachment", line);
    }
    // An invitation exposes the plan, not another employer's private contracts.
    // The organizer resolves their crew only while actually aboard this hull.
    if ctx.actor != draft.organizer {
        return Ok(());
    }
    let has_npcs = draft.plan.npc_commander.is_some()
        || draft
            .plan
            .slots
            .iter()
            .any(|slot| matches!(slot.controller, PlannedController::Npc { .. }));
    if !has_npcs {
        return Ok(());
    }
    if !physically_aboard(ctx.w, ctx.actor, draft.plan.carrier) {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.detachment",
            "Board the selected carrier to resolve its current NPC crew.",
        );
        return Ok(());
    }
    let handles = ctx
        .w
        .module_state::<crate::detachment::crew::CrewHandles>()
        .ok_or("Crew contracts unavailable.")?;
    let personnel = crate::detachment::crew::personnel::resolve(ctx.w, draft, |npc| {
        ctx.action_state(&handles.contracts, &npc)
            .map(|row| row.cloned())
            .map_err(|e| e.to_string())
    })?;
    for row in personnel {
        let role = row.slot.map_or_else(
            || "Commander".to_owned(),
            |slot| format!("Slot {} NPC", slot.get()),
        );
        let status = match row.terms {
            Ok(terms) => format!(
                "aboard now; contract recorded; {}{}",
                crate::detachment::crew::describe_terms(&terms),
                if row.commander {
                    "; nominated commander covered"
                } else {
                    ""
                }
            ),
            Err(issue) => issue.label().to_owned(),
        };
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.detachment",
            format!("{role} #{} — {status}.", row.character.0),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
