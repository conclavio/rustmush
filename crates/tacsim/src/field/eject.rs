//! **Ejection** (doc 42 cut CR.4; Q-42-10 to Q-42-14): `+sortie/eject`, the one extraction a
//! pilot starts without the stranding.
//!
//! **Who may** (Q-42-10): the seated pilot of a doomed unit, from inside it (a remote console has
//! nothing to eject from), never while world time is frozen; an NPC pilot never types it. **When**
//! (Q-42-11): a unit is doomed when it could not bring its pilot home at a check-in, or a control
//! section is already damaged (`super::crew::doomed`).
//!
//! **In a running field** the host asks the arena for one read of its live state
//! ([`crate::arena_task::ArenaMsg::Eject`], `crate::arena::eject`): whether the unit is doomed,
//! the crew's condition its sections give, and the parties holding its cell. The answer
//! (`on_ejected`) takes the first holder with a carrier on the site (Q-42-12 (d), `holder`)
//! and draws, deterministically, against `[tacsim.field].eject_capture_permille` ([`draw`]): an
//! FNV-1a hash over the field's site tag, its ordinal, the unit and the read's arena millisecond,
//! so a replay and a gate agree. **After a field** (the unit standing on the site's ground, no
//! field running) the world's cells decide and nobody holds the cell.
//!
//! **The eject** (`ejected`) releases the seat at once — every channel `Released` to the arena,
//! the station vacated, so the hull is its mind's or nobody's — and writes `Stranded { site, unit
//! }` with a forced extraction due after `extract_s`, carrying the draw's [`EjectCapture`] when
//! there was one. The body stays in the cockpit room until the landing moves it (Q-CF-39): the
//! held gate refuses it movement and operation (`super::crew::PICKUP`) and every load rule skips
//! the record (Draft 0.3, 42-5 (B)). The condition is kept (Q-42-13). The landing is the crew
//! tick's (`super::crew::tick`).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, ObjKind, World};
use smol_str::SmolStr;

use super::crew::{
    CREW_FORMAT, CrewCondition, CrewLocation, CrewRecord, EjectCapture, Extraction, Gate, crew_of,
};
use crate::arena::PartyId;
use crate::arena::eject::{EjectRead, EjectRefusal};
use crate::arena_task::MatchId;

/// The eject asked of a running field: `{unit}`.
pub const EJECT_ASKED: &str = "You reach for the release in #{unit}: if it cannot bring you home, you are out of the seat at the next pulse.";
/// A unit that can still bring its pilot home: `{unit}`.
pub const NOT_DOOMED: &str = "#{unit} can still bring you home: fly it, or wait until it cannot.";
/// A unit flown from a console.
pub const EJECT_REMOTE: &str = "You fly that unit from a console: there is nothing to eject from.";
/// Nobody seated.
pub const EJECT_NOT_FLYING: &str = "You are not seated in a unit on a site.";
/// A stranded pilot.
pub const EJECT_STRANDED: &str =
    "You are already stranded: +sortie/extract asks a recovery service to lift you out.";
/// The eject, told to the pilot: `{unit}`, `{h}`.
pub const EJECTED: &str = "You eject from #{unit}: the seat is empty, the hull is the field's now. Your pickup comes in about {h} IC h; until then you wait in the cockpit.";
/// The eject's capture landing, told to the captive: `{carrier}`, `{captor}`.
pub const EJECT_CAPTURED: &str = "The pickup that comes for you is the {carrier}'s, not yours: you are carried aboard and held there, in {captor}'s custody.";
/// The eject's capture, told to the captor: `{who}`, `{carrier}`.
pub const EJECT_CAPTOR_TOLD: &str = "Your people pick {who} out of the cockpit and carry them aboard the {carrier}: you hold them now. +ship/captives lists them; +ship/release <who> lets one go.";

/// **The ejects asked of running fields** (RAM): each unit and the pilot who asked. A restart
/// before the answer loses the request, and the pilot asks again.
#[derive(Clone, Debug, Default)]
pub(crate) struct EjectBook(pub BTreeMap<ObjId, ObjId>);

/// **`+sortie/eject`** (this module's head).
///
/// # Errors
///
/// The refusal line.
pub(crate) fn eject_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    super::crew::gate(ctx, Gate::Move)?;
    let actor = ctx.actor;
    if ctx
        .w
        .get(actor)
        .is_none_or(|object| object.kind != ObjKind::Player)
    {
        return Err(CmdError::msg(EJECT_NOT_FLYING));
    }
    if let Some(record) = crew_of(ctx.w, actor) {
        if matches!(record.location, CrewLocation::Stranded { .. }) {
            return Err(CmdError::msg(EJECT_STRANDED));
        }
        if record.extraction.is_some() {
            return Err(CmdError::msg(
                "Your extraction is already on its way: +sortie/condition shows when it lands.",
            ));
        }
    }
    if ctx.w.world_timeline().is_frozen() {
        return Err(CmdError::msg(
            "World time is frozen; eject once it resumes.",
        ));
    }
    let seat = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.seat_of_pilot(actor));
    let Some((task, unit)) = seat else {
        return after_field(ctx, actor);
    };
    if !super::crew::inside(ctx.w, actor, unit) {
        return Err(CmdError::msg(EJECT_REMOTE));
    }
    if ctx
        .w
        .module_state::<EjectBook>()
        .is_some_and(|book| book.0.contains_key(&unit))
    {
        return Err(CmdError::msg("You are already reaching for the release."));
    }
    let tasks = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or_else(|| CmdError::msg("The field host is unavailable."))?;
    tasks
        .offer(task, crate::arena_task::ArenaMsg::Eject { unit })
        .map_err(|_| CmdError::msg("The field is busy; try the eject again."))?;
    if let Some(book) = ctx.w.module_state_mut::<EjectBook>() {
        book.0.insert(unit, actor);
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.field",
        EJECT_ASKED.replace("{unit}", &unit.0.to_string()),
    );
    Ok(())
}

/// **The eject after a field** (this module's head): the pilot seated in the unit they stand in,
/// on a site's ground with no field running; the world's cells decide, and nobody holds the cell.
fn after_field(ctx: &mut Ctx<'_>, actor: ObjId) -> Result<(), CmdError> {
    let unit = seated_unit(ctx.w, actor).ok_or_else(|| CmdError::msg(EJECT_NOT_FLYING))?;
    let site = match crate::surface::act::where_is(ctx.w, unit) {
        crate::surface::act::Where::Ground { domain, .. } => {
            crate::surface::act::site_of_domain(ctx.w, domain)
        }
        crate::surface::act::Where::Bay { .. } | crate::surface::act::Where::Elsewhere => None,
    }
    .ok_or_else(|| CmdError::msg(EJECT_NOT_FLYING))?;
    let cells = mush_tactics::module::unit_of(ctx.w, unit)
        .ok_or_else(|| CmdError::msg(EJECT_NOT_FLYING))?;
    if !super::crew::doomed(cells) {
        return Err(CmdError::msg(
            NOT_DOOMED.replace("{unit}", &unit.0.to_string()),
        ));
    }
    let condition = super::crew::cells_condition(cells);
    ejected(ctx, actor, &site, unit, condition, None);
    Ok(())
}

/// **The unit `actor` is seated in and stands inside** (doc 38a review, Astra #12): the first
/// holder up `actor`'s location chain, as [`super::crew::inside`] walks it, whose seat is theirs.
/// A pilot seated in a cab stands in the cab's room, never in the hull itself.
fn seated_unit(w: &World, actor: ObjId) -> Option<ObjId> {
    let mut here = w.get(actor).and_then(|object| object.location);
    for _ in 0..16 {
        let at = here?;
        if super::parties::seated(w, at).is_some_and(|(seated, _)| seated == actor) {
            return Some(at);
        }
        here = w.get(at).and_then(|object| object.location);
    }
    None
}

/// **The arena's answer to an eject** (the bridge's branch; this module's head).
pub(crate) fn on_ejected(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    unit: ObjId,
    outcome: Result<EjectRead, EjectRefusal>,
) {
    let Some(pilot) = ctx
        .w
        .module_state_mut::<EjectBook>()
        .and_then(|book| book.0.remove(&unit))
    else {
        return;
    };
    let read = match outcome {
        Ok(read) => read,
        Err(EjectRefusal::NotDoomed) => {
            ctx.out.sys_to_actor(
                pilot,
                "tacsim.field",
                NOT_DOOMED.replace("{unit}", &unit.0.to_string()),
            );
            return;
        }
        Err(EjectRefusal::NotHere) => {
            ctx.out.sys_to_actor(
                pilot,
                "tacsim.field",
                "That unit is not in the field.".to_owned(),
            );
            return;
        }
    };
    // The pilot must still sit where they asked from.
    let seated = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.seat_of_pilot(pilot))
        == Some((task, unit));
    let Some(site) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.site_of_task(task).cloned())
    else {
        return;
    };
    if !seated || !super::crew::inside(ctx.w, pilot, unit) {
        return;
    }
    let capture = holder(ctx.w, &site, &read).filter(|_| {
        let ordinal = ctx
            .w
            .module_state::<super::FieldBook>()
            .and_then(|book| book.hosted(&site))
            .map_or(0, |hosted| hosted.live.field.ordinal);
        draw(&site, ordinal, unit, read.at_ms) < super::config(ctx.w).eject_capture_permille
    });
    ejected(ctx, pilot, &site, unit, read.condition, capture);
}

/// **The first holder with a carrier on the site** (Q-42-12 (d)): `read`'s holders, nearest
/// first, each with a carrier of its own in the field — one of its own units, or one of a party
/// its leader leads — that has not fallen, and a leader to be the captor.
fn holder(w: &World, site: &CanonicalId, read: &EjectRead) -> Option<EjectCapture> {
    holder_of(w, site, &read.holders)
}

/// **[`holder`]'s rule over any holders, nearest first** (doc 42b cut SU.3, Q-42b-9: a raider's
/// yield names its holder by the eject's rule).
pub(crate) fn holder_of(
    w: &World,
    site: &CanonicalId,
    holders: &[PartyId],
) -> Option<EjectCapture> {
    let hosted = w.module_state::<super::FieldBook>()?.hosted(site)?;
    let live = &hosted.live;
    let leader_of = |party: PartyId| {
        live.parties
            .iter()
            .find(|row| row.party == party)
            .and_then(|row| row.leader)
    };
    holders.iter().find_map(|party| {
        let captor = leader_of(*party)?;
        let carrier = live
            .units
            .iter()
            .filter(|row| row.party == *party || leader_of(row.party) == Some(captor))
            .map(|row| row.unit)
            .find(|unit| super::carrier::is_carrier(w, *unit) && !fallen(w, *unit))?;
        Some(EjectCapture {
            party: *party,
            carrier,
            captor,
        })
    })
}

/// Whether `carrier`'s hull has fallen (doc 36 cut GC.3's `CrewStranded`).
pub(crate) fn fallen(w: &World, carrier: ObjId) -> bool {
    crate::carrier_damage::damage_of(w, carrier)
        .is_some_and(|damage| damage.has(crate::carrier_damage::CarrierLoss::CrewStranded))
}

/// **The eject's draw**, 0 to 999 (Q-42-12): FNV-1a over the field's site tag and ordinal (GC.10's
/// `cause_prefix`), the unit and the read's arena millisecond. A capture when it is below the
/// configured per mille, so 1,000 always captures and 0 never does.
#[must_use]
pub fn draw(site: &CanonicalId, ordinal: u32, unit: ObjId, at_ms: i64) -> u32 {
    permille(&format!(
        "{}eject:{}:{at_ms}",
        super::standing::cause_prefix(site, ordinal),
        unit.0
    ))
}

/// **A raider crew's morale draw**, 0 to 999 (doc 42b cut SU.3, Q-42b-8): [`draw`]'s FNV-1a with
/// the pressure step folded in where the eject folds the millisecond, so a resumed field draws
/// each step exactly as before.
#[must_use]
pub fn yield_draw(site: &CanonicalId, ordinal: u32, hull: ObjId, step: u16) -> u32 {
    permille(&format!(
        "{}yield:{}:{step}",
        super::standing::cause_prefix(site, ordinal),
        hull.0
    ))
}

/// **The parties holding `hull`'s cell in `arena`** (doc 42b cut SU.3, Q-42b-9), nearest unit
/// first: each Hostile to the hull's party with a live armed unit that has the hull inside its own
/// reach. The eject's (a) and (b) as a read on any hull; [`holder_of`] then wants a carrier.
#[must_use]
pub(crate) fn holding_parties(arena: &crate::arena::Arena, hull: ObjId) -> Vec<PartyId> {
    let Some(target) = arena.unit(hull) else {
        return Vec::new();
    };
    let cell_cm = arena.definition.terrain.cell_cm();
    let at = &target.pose;
    let mut near: Vec<(i128, PartyId)> = arena
        .units
        .iter()
        .filter(|holder| holder.id != hull && holder.team != target.team)
        .filter(|holder| {
            let reach = holder.fight_inner_cm(cell_cm);
            arena.relations.relation(holder.team, target.team)
                == crate::arena::party::Relation::Hostile
                && !holder.limits.wreck
                && (holder.armed_at_checkout() || holder.shots_fired > 0)
                && reach != 0
                && crate::arena::index::within_cm(&holder.pose, at, reach)
        })
        .map(|holder| {
            let dx = i128::from(holder.pose.x_cm) - i128::from(at.x_cm);
            let dy = i128::from(holder.pose.y_cm) - i128::from(at.y_cm);
            let dz = i128::from(holder.pose.z_cm) - i128::from(at.z_cm);
            (dx * dx + dy * dy + dz * dz, holder.team)
        })
        .collect();
    near.sort();
    let mut parties: Vec<PartyId> = Vec::new();
    for (_, party) in near {
        if !parties.contains(&party) {
            parties.push(party);
        }
    }
    parties
}

/// FNV-1a over `seed`, per mille.
fn permille(seed: &str) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in seed.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    u32::try_from(hash % 1_000).unwrap_or(0)
}

/// **The eject** (this module's head): the seat released, the record stranded with a forced
/// extraction carrying `capture`, the condition kept, and the pilot told.
fn ejected(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    site: &CanonicalId,
    unit: ObjId,
    condition: CrewCondition,
    capture: Option<EjectCapture>,
) {
    let Some(port) = super::crew::destination(ctx.w, who) else {
        ctx.out.sys_to_actor(
            who,
            "tacsim.field",
            crate::ic_return::NO_CORRIDOR.to_owned(),
        );
        return;
    };
    super::crew::release_seat(ctx, who, unit);
    let now = ctx.w.world_ic_now().at;
    let delay = i64::from(super::config(ctx.w).extract_s);
    let prior = crew_of(ctx.w, who).cloned();
    let organizer = super::crew::organizer_of(ctx.w, site, unit)
        .or_else(|| prior.as_ref().and_then(|record| record.organizer));
    let condition = prior
        .as_ref()
        .map_or(condition, |record| record.condition.max(condition));
    super::crew::put(
        ctx,
        CrewRecord {
            format: CREW_FORMAT,
            character: who,
            location: CrewLocation::Stranded {
                site: site.clone(),
                unit,
            },
            condition,
            at_ic: now,
            organizer,
            extraction: Some(Extraction {
                port: SmolStr::new(port.id),
                requested_ic: now,
                due_ic: now.saturating_add(delay),
                abandoned: false,
                forced: true,
                capture,
            }),
            treatment: None,
            resolve_permille: prior.as_ref().and_then(|record| record.resolve_permille),
        },
    );
    ctx.out.sys_to_actor(
        who,
        "tacsim.field",
        EJECTED
            .replace("{unit}", &unit.0.to_string())
            .replace("{h}", &super::crew::hours(delay).to_string()),
    );
    tracing::info!(
        who = who.0,
        unit = unit.0,
        "tacsim: a pilot ejected from a doomed unit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 38a review, Astra #12**: a pilot seated in a hull from a cab room inside it is found
    /// in that hull, as a pilot standing in the hull is; a stranger's seat is nobody's.
    #[test]
    fn review_72h_a_cab_seated_pilot_is_flying_the_hull() {
        let mut w = World::new_minimal();
        let hull = w
            .create(
                ObjKind::Thing,
                "Rig",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("hull");
        let cab = w
            .create(ObjKind::Thing, "Cab", mush_core::GOD, Some(hull))
            .expect("cab");
        let pilot = w
            .create(ObjKind::Player, "Pilot", mush_core::GOD, Some(cab))
            .expect("pilot");
        let stranger = w
            .create(ObjKind::Player, "Stranger", mush_core::GOD, Some(cab))
            .expect("stranger");
        let mut spatial = mush_sim::SpatialState::default();
        spatial.assignments.insert(
            hull,
            mush_sim::data::HostAssignments {
                host: hull,
                seats: [(
                    mush_sim::StationId::new("cockpit").expect("station"),
                    mush_sim::data::StationAssignment {
                        actor: pilot,
                        assigned_at: 0,
                        granted: None,
                    },
                )]
                .into_iter()
                .collect(),
            },
        );
        w.module_state_insert(spatial);
        assert_eq!(seated_unit(&w, pilot), Some(hull));
        assert_eq!(seated_unit(&w, stranger), None);
    }

    /// **Doc 42b cut SU.3** (Q-42b-8): the morale draw is the eject's FNV-1a keyed by the step,
    /// never the millisecond: the same inputs draw the same, and the step and the hull each
    /// change it.
    #[test]
    fn su3_the_yield_draw_is_keyed_by_the_step() {
        let site = CanonicalId::new("ahuna_port").expect("a site");
        let draw = |hull: u32, step: u16| yield_draw(&site, 3, ObjId(hull), step);
        assert_eq!(draw(40, 2), draw(40, 2));
        assert!(draw(40, 2) < 1_000);
        let spread: std::collections::BTreeSet<u32> = (1..=10).map(|step| draw(40, step)).collect();
        assert!(spread.len() > 5, "the steps draw apart: {spread:?}");
        assert_ne!(
            (1..=10).map(|step| draw(40, step)).collect::<Vec<_>>(),
            (1..=10).map(|step| draw(41, step)).collect::<Vec<_>>()
        );
    }

    /// Doc 42 cut CR.4 (Q-42-12): the draw is a pure function of its seed, spread over 0..1000,
    /// so 1,000 always captures and 0 never does.
    #[test]
    fn cr4_the_draw_is_seeded_and_bounded() {
        let site = CanonicalId::new("ahuna_flank").expect("an id");
        let a = draw(&site, 3, ObjId(40), 12_000);
        assert_eq!(
            a,
            draw(&site, 3, ObjId(40), 12_000),
            "the same seed draws alike"
        );
        assert!(a < 1_000);
        let spread: std::collections::BTreeSet<u32> = (0..64)
            .map(|at| draw(&site, 3, ObjId(40), at * 100))
            .collect();
        assert!(spread.len() > 32, "the pulse moves the draw: {spread:?}");
    }
}
