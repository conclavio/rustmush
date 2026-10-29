//! Fresh, private observations of the accepted roster. No cached launch permit.
use super::{ActivationHandles, ActiveDetachment, Reservation, runtime};
use crate::detachment::crew::personnel;
use crate::detachment::planning::{DraftRef, PlanningHandles, projection};
use mush_core::{Ctx, Flags, ObjKind};
use mush_org::unit_authority::UnitCapability;
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
use mush_tactics::detachment::planning::{PlanId, PlannedController};

fn line(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    ctx.out
        .sys_to_actor(ctx.actor, "tacsim.detachment", text.into());
}
fn authority(ctx: &Ctx<'_>, request: UnitAuthorityRequest) -> Option<bool> {
    let handle = ctx
        .w
        .module_state::<PlanningHandles>()?
        .unit_authority_reader
        .as_ref()?;
    match ctx.read_owner(handle, request).ok()? {
        UnitAuthorityResponse::Authority { allowed } => Some(allowed),
        _ => None,
    }
}
fn status(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
fn permitted(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "allowed now",
        Some(false) => "denied now",
        None => "unavailable",
    }
}
fn reservations(ctx: &Ctx<'_>, record: &ActiveDetachment) -> bool {
    let Some(handles) = ctx.w.module_state::<ActivationHandles>() else {
        return false;
    };
    record.entities().into_iter().all(|entity| {
        ctx.action_state(&handles.reservations, &entity)
            .ok()
            .flatten()
            == Some(&Reservation::for_entity(record, entity))
    })
}

pub(super) fn report(ctx: &mut Ctx<'_>, id: PlanId, revision: u64) -> Result<(), String> {
    let unavailable =
        "Accepted roster unavailable. Use +sortie/active for the current activation revision.";
    let record = runtime::read(
        ctx,
        DraftRef {
            organizer: ctx.actor,
            id,
        },
    )?
    .filter(|r| r.active() && r.revision == revision)
    .ok_or(unavailable)?;
    if ctx
        .w
        .module_state::<ActivationHandles>()
        .is_none_or(|h| !h.enabled)
        || ctx
            .w
            .module_state::<crate::TacsimStore>()
            .is_none_or(|s| !s.enabled())
        || ctx
            .w
            .get(ctx.actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(unavailable.into());
    }
    let carrier = record.plan.carrier;
    if !projection::physically_aboard(ctx.w, ctx.actor, carrier) {
        return Err("Board the selected carrier to inspect launch preflight.".into());
    }
    line(
        ctx,
        format!(
            "Launch preflight for draft {}, activation revision {} (accepted draft revision {}, {:?}).",
            id.0, record.revision, record.source_revision, record.duty
        ),
    );
    line(
        ctx,
        format!(
            "Accepted unit/person reservations intact: {}.",
            status(reservations(ctx, &record))
        ),
    );
    carrier_report(ctx, carrier);
    for slot in &record.plan.slots {
        let unit = slot.unit.ok_or(unavailable)?;
        let operator = slot.controller.character().ok_or(unavailable)?;
        line(
            ctx,
            format!(
                "Slot {}: unit #{}; operator #{}.",
                slot.slot.get(),
                unit.0,
                operator.0
            ),
        );
        unit_report(ctx, carrier, unit, operator);
        personnel_report(ctx, &record, slot.controller, unit)?;
    }
    line(
        ctx,
        "Observations only. Launch must recheck these facts and admit actual station control, crew capability and any selected mind. This report moves nothing and grants no launch permission.",
    );
    Ok(())
}

fn carrier_report(ctx: &mut Ctx<'_>, carrier: mush_core::ObjId) {
    let site = crate::port::ship::hull_view(ctx.w, carrier)
        .and_then(|view| crate::surface::act::site_berth(ctx.w, &view).ok())
        .map(|berth| format!("{}; {}", berth.name, berth.ramp_label));
    line(
        ctx,
        match site {
            Some(site) => format!("Carrier surface berth: {site}."),
            None => {
                "Carrier surface berth unavailable: settle a bay-equipped carrier at a field site."
                    .into()
            }
        },
    );
    if crate::port::voyage::voyage_blocks(ctx.w, carrier) {
        line(
            ctx,
            "Carrier departure clearance is filed or unreadable; surface launch is blocked.",
        );
    }
}

fn unit_report(
    ctx: &mut Ctx<'_>,
    carrier: mush_core::ObjId,
    unit: mush_core::ObjId,
    operator: mush_core::ObjId,
) {
    line(
        ctx,
        format!(
            "  Unit currently aboard selected carrier: {}.",
            status(projection::physically_aboard(ctx.w, unit, carrier))
        ),
    );
    let deploy = authority(
        ctx,
        UnitAuthorityRequest::Authorize {
            unit,
            capability: UnitCapability::Deploy,
            delegate: false,
            mission: None,
        },
    );
    line(
        ctx,
        format!("  Your Deploy authority: {}.", permitted(deploy)),
    );
    // Accepted IDs alone do not grant condition or other-principal reads.
    if deploy == Some(true) {
        line(
            ctx,
            format!(
                "  Operator OperateCrew authority: {}.",
                permitted(authority(
                    ctx,
                    UnitAuthorityRequest::CrewUse { unit, operator }
                ))
            ),
        );
        let condition = mush_tactics::module::unit_of(ctx.w, unit)
            .and_then(|unit| crate::field_ops::assess_tacsim_unit(unit, None).ok());
        if let Some(condition) = condition {
            line(
                ctx,
                format!(
                    "  Unit condition: {}; power {}; control {}; self movement {}; weapons {}.",
                    condition.state.as_str(),
                    status(condition.has_power),
                    status(condition.has_control),
                    status(condition.can_move),
                    status(condition.can_fire)
                ),
            );
        } else {
            line(ctx, "  Unit condition unavailable.");
        }
    }
}

fn personnel_report(
    ctx: &mut Ctx<'_>,
    record: &ActiveDetachment,
    controller: PlannedController,
    unit: mush_core::ObjId,
) -> Result<(), String> {
    let carrier = record.plan.carrier;
    let unavailable = "Accepted crew contracts unavailable.";
    match controller {
        PlannedController::Npc { character } => {
            let handles = ctx
                .w
                .module_state::<ActivationHandles>()
                .ok_or(unavailable)?;
            let contract = ctx
                .action_state(&handles.contracts, &character)
                .ok()
                .flatten()
                .cloned();
            let terms = personnel::check(
                ctx.w,
                record.organizer,
                carrier,
                character,
                record.plan.npc_commander == Some(character),
                contract,
            );
            let status = match terms {
                Err(issue) => issue.label().to_owned(),
                Ok(terms) if !terms.duties.contains(&record.duty) => {
                    "duty outside employment agreement".into()
                }
                Ok(terms) if terms.digest().ok().as_ref() != record.npc_terms.get(&character) => {
                    "accepted contract terms changed".into()
                }
                Ok(_) => "aboard now; accepted contract and duty still match".into(),
            };
            line(ctx, format!("  NPC personnel: {status}."));
        }
        PlannedController::Player { character } => {
            line(
                ctx,
                format!(
                    "  Player currently aboard selected carrier: {}; accepted physical helm station present: {}.",
                    status(projection::physically_aboard(ctx.w, character, carrier)),
                    status(projection::physical_station(ctx.w, character, unit).is_some())
                ),
            );
        }
        _ => return Err(unavailable.into()),
    }
    Ok(())
}
