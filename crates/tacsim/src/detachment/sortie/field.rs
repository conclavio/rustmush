//! The accepted IC roster, refreshed from actual owner records and placement.
use super::{
    facts,
    model::*,
    prepare,
    runtime::{self, Read, SortieHandles},
};
use mush_core::*;
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
use mush_tactics::detachment::{planning::PlannedController, roster::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CrewFact {
    pub unit: ObjId,
    pub operator: ObjId,
    pub controls: bool,
    pub operate: bool,
    pub command_authorized: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerFacts {
    pub crew: Vec<CrewFact>,
}

pub(super) fn roster(op: &OperationRecord) -> Result<DetachmentRoster, String> {
    let members = op
        .roster
        .plan
        .slots
        .iter()
        .map(|s| {
            Ok(Member {
                slot: s.slot,
                unit: s.unit.ok_or("Unit unavailable.")?,
                controller: match s.controller {
                    PlannedController::Player { character } => Controller::Player { character },
                    PlannedController::Npc { character } => Controller::Npc { character },
                    _ => return Err("Field controller unavailable.".to_owned()),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    DetachmentRoster::new(
        DetachmentId(op.roster.detachment),
        op.id.activation,
        members,
        op.roster.plan.npc_commander,
        LeadershipPolicy::default(),
    )
    .map_err(|e| e.to_string())
}
fn queries(op: &OperationRecord) -> Result<Vec<mush_sim::crew_control::CrewControlQuery>, String> {
    roster(op)?
        .members()
        .iter()
        .map(|m| {
            Ok(mush_sim::crew_control::CrewControlQuery {
                unit: m.unit,
                operator: m.controller.character().ok_or("No operator.")?,
            })
        })
        .collect()
}
fn rights(
    world: &World,
) -> Result<
    &mush_core::read_service::OwnerReadHandle<
        mush_org::unit_authority::reader::UnitAuthorityReader,
    >,
    String,
> {
    world
        .module_state::<crate::detachment::planning::PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
        .ok_or_else(|| "Unit rights unavailable.".into())
}
fn combine(
    query: Vec<mush_sim::crew_control::CrewControlQuery>,
    controls: Vec<bool>,
    allowed: UnitAuthorityResponse,
) -> Result<OwnerFacts, String> {
    let UnitAuthorityResponse::CommandAvailability { allowed, operate } = allowed else {
        return Err("Unit rights unavailable.".into());
    };
    if query.len() != controls.len() || query.len() != allowed.len() || query.len() != operate.len()
    {
        return Err("Incomplete owner facts.".into());
    }
    Ok(OwnerFacts {
        crew: query
            .into_iter()
            .zip(controls)
            .zip(allowed)
            .zip(operate)
            .map(|(((q, controls), command_authorized), operate)| CrewFact {
                unit: q.unit,
                operator: q.operator,
                controls,
                operate,
                command_authorized,
            })
            .collect(),
    })
}
pub(super) fn read_owner_facts(
    ctx: &Ctx<'_>,
    h: &SortieHandles,
    op: &OperationRecord,
) -> Result<OwnerFacts, String> {
    let query = queries(op)?;
    let controls = ctx
        .read_owner(
            h.controls.as_ref().ok_or("Crew controls unavailable.")?,
            query.clone(),
        )
        .map_err(|e| e.to_string())?;
    let allowed = ctx
        .read_owner(
            rights(ctx.w)?,
            UnitAuthorityRequest::CommandAvailability {
                crew: query.iter().map(|q| (q.unit, q.operator)).collect(),
            },
        )
        .map_err(|e| e.to_string())?;
    combine(query, controls, allowed)
}
pub(super) fn verify_owner_facts(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    h: &SortieHandles,
    id: OperationId,
    expected: &OwnerFacts,
) -> Result<(), ActionError> {
    let op = prepare::operation(&mut &mut *txn, h, id).map_err(runtime::error)?;
    let query = queries(&op).map_err(runtime::error)?;
    let rights = rights(ctx.world().map_err(runtime::error)?)
        .map_err(runtime::error)?
        .clone();
    let controls = ctx
        .read_owner_preimage(
            txn,
            h.controls
                .as_ref()
                .ok_or_else(|| runtime::error("Crew controls unavailable."))?,
            query.clone(),
        )
        .map_err(runtime::error)?;
    let allowed = ctx
        .read_owner_preimage(
            txn,
            &rights,
            UnitAuthorityRequest::CommandAvailability {
                crew: query.iter().map(|q| (q.unit, q.operator)).collect(),
            },
        )
        .map_err(runtime::error)?;
    if combine(query, controls, allowed).map_err(runtime::error)? != *expected {
        return Err(runtime::error("Field authority changed; refresh again."));
    }
    Ok(())
}
pub(super) fn reconcile(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    op: &OperationRecord,
    owner: &OwnerFacts,
) -> Result<(DetachmentRoster, Vec<Value>), String> {
    let expected = queries(op)?;
    if owner.crew.len() != expected.len()
        || owner
            .crew
            .iter()
            .zip(&expected)
            .any(|(a, b)| a.unit != b.unit || a.operator != b.operator)
    {
        return Err("Field assignments changed.".into());
    }
    let mut roster = match &op.command {
        Some(saved) => DetachmentRoster::restore(saved).map_err(|e| e.to_string())?,
        None => roster(op)?,
    };
    // IC is continuous over rate changes and restart; freeze stops the timers.
    let now = u64::try_from(world.world_ic_now().at)
        .map_err(|_| "Field clock unavailable.")?
        .checked_mul(1000)
        .ok_or("Field clock exhausted.")?;
    let mut observations = Vec::new();
    let mut live = Vec::new();
    let active = read.get(&h.active, &op.id.reference())?.as_ref() == Some(&op.roster);
    for (member, proof) in roster.members().iter().zip(&owner.crew) {
        let character = proof.operator;
        let npc = matches!(member.controller, Controller::Npc { .. });
        let contract = if npc {
            read.get(&h.contracts, &character)?
        } else {
            None
        };
        let employed = !npc
            || contract.as_ref().is_some_and(|c| {
                c.validate().is_ok()
                    && c.npc == character
                    && c.employer == op.id.organizer
                    && c.terms.digest().ok().as_ref() == op.roster.npc_terms.get(&character)
                    && c.terms.duties.contains(&op.roster.duty)
            });
        let current = if read.get(&h.heads, &member.unit)?.is_some() {
            Some(prepare::current(read, h, member.unit)?.1)
        } else {
            None
        };
        let field = current.as_ref().is_some_and(|f| {
            f.operation == op.id
                && f.operator == character
                && f.phase == Phase::Field
                && prepare::membership(op, f).is_ok()
                && facts::on_site(world, f)
        });
        let station = facts::station(world, character, member.unit, npc).ok();
        let reserved = facts::reservation(
            &op.roster,
            member.unit,
            read.get(&h.reservations, &member.unit)?,
        )
        .is_ok()
            && facts::reservation(
                &op.roster,
                character,
                read.get(&h.reservations, &character)?,
            )
            .is_ok();
        let unit = mush_tactics::module::unit_of(world, member.unit);
        let condition = unit.and_then(|u| crate::field_ops::assess_tacsim_unit(u, None).ok());
        let connection = if npc || world.has_live_conns(character) {
            Connection::Connected
        } else if world.is_connected(character) {
            Connection::Linkdead {
                grace_until_ms: u64::MAX,
            }
        } else {
            Connection::Disconnected
        };
        live.push(MemberFacts {
            slot: member.slot,
            capable: employed
                && facts::live(
                    world,
                    character,
                    if npc { ObjKind::Npc } else { ObjKind::Player },
                ),
            controls_unit: field
                && active
                && reserved
                && facts::live(world, member.unit, ObjKind::Thing)
                && station.is_some()
                && proof.controls
                && proof.operate
                && op.ended.is_none(),
            unit_disabled: condition
                .as_ref()
                .is_none_or(|c| !c.has_power || !c.has_control || (!c.can_move && !c.can_fire)),
            connection,
            command_authorized: proof.command_authorized && op.ended.is_none(),
        });
        observations.push(json!({"unit":member.unit,"station":station,"condition":condition,"flight":current,"contract":contract,
            "connected":world.is_connected(character),"live":world.has_live_conns(character),
            "person":world.get(character).map(|o|(o.kind,o.location,o.flags.contains(Flags::GOING)))}));
    }
    roster.reconcile(now, &live).map_err(|e| e.to_string())?;
    Ok((roster, observations))
}

pub(super) fn refresh(ctx: &mut Ctx<'_>, id: OperationId) -> Result<(), String> {
    let h = ctx
        .w
        .module_state::<SortieHandles>()
        .cloned()
        .ok_or("Field host unavailable.")?;
    let op = prepare::operation(&mut &*ctx, &h, id)?;
    let facts = read_owner_facts(ctx, &h, &op)?;
    runtime::submit(
        ctx,
        runtime::Command::Refresh {
            operation: id,
            facts,
        },
    )
}
