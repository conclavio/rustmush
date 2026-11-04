//! **The carrier as a unit** (doc 32 cut CF.3; Q-CF-19, Q-CF-38, §4a; doc 27 Q-H10, FO-13).
//!
//! A programme hull settled on a promoted site's pad is on the ground like everyone else
//! (Q-CF-16): it is admitted at its pad pose with its real sections — the Dray, the Dray-P and
//! the Halcyon carry unit specs with sections and facet bands — through the same custody
//! check-out, and its damage comes home through the same IC check-in into the hull's own unit
//! record, which `+ship` reads in port (Q-CF-19). It is shootable from the first pulse; what a
//! hit does to lift, sailing, the bay and the crew is doc 32 F-3, read from doc 36 cut GC.3's
//! table ([`crate::carrier_damage`]).
//!
//! - **Its party** is the roster it launched (§3 Q-CF-4's note): the activation of any open DT
//!   flight on this site whose carrier it is, else a party of one led by whoever commands it
//!   ([`crate::port::coop::helm_holder`]). One commander behind the carrier and its rigs makes
//!   them Friends through the field's standing-friendship rule (Q-CF-5).
//! - **Its crew** (Q-CF-38): everyone physically aboard at admission is "aboard the carrier"
//!   for the field's life, listed on the carrier's row of the live record; doc 32 cut CF.5's crew
//!   state reads those rows (`crate::field::crew::crew_location`) and sets each soul's record
//!   from them at the carrier's check-in.
//! - **It is towed only by a tug of at least half its tonnage** (FO-13; doc 36 cut GC.3, Q-36-8
//!   as ruled): [`refuse_tow`] names the carrier and the tonnage it needs. No shipped design is
//!   that heavy, so in practice a carrier is not towed yet.
//! - **What its damage costs it** (doc 32 F-3; doc 36 cut GC.3) is the one consequence table in
//!   [`crate::carrier_damage`]: a destroyed drive refuses lift; a destroyed hull wrecks it,
//!   strands its crew and wrecks the rigs in its bay.
//! - **It departs mid-field only live, on the leave-behind confirmation** (Q-H10; doc 32 F-6,
//!   ruled 2026-10-31): a live `+ship/lift` naming every unit out removes it from the field
//!   outright (`crate::field::exit`); [`departure_refusal`] is a queued departure's gate.

use mush_core::{CanonicalId, Ctx, ObjId, World};

use super::FieldBook;

/// **A queued departure while the carrier is in a field** (Q-H10, doc 32 §4a, F-6): its units
/// are out on a promoted site, so a departure leaves them; `{name}`, `{site}`, `{units}`.
pub const DEPART_IN_FIELD: &str = "The {name} is in the field on {site}: lifting now leaves \
     {units} on the ground. A queued departure never takes a carrier out of a field; lift live \
     with leave= naming every unit you have out (doc 32 F-6).";

/// **A tow naming a carrier** (FO-13; doc 36 cut GC.3, Q-36-8): `{name}`, `{t}`, `{floor}`,
/// `{tug}`, `{tug_t}`.
pub const TOW_CARRIER: &str = "The {name} is a carrier of {t} t: only a tug of at least {floor} t      can tow it, and the {tug} is {tug_t} t.";

/// Whether `unit` is a programme hull — a carrier (Dray, Dray-P, Halcyon).
#[must_use]
pub fn is_carrier(w: &World, unit: ObjId) -> bool {
    crate::port::ship::hull_view(w, unit).is_some()
}

/// **Everyone physically aboard `carrier` now**: the souls in its bounded subtree (rooms, bay,
/// a seated operator's placement), by id.
#[must_use]
pub(crate) fn souls_aboard(w: &World, carrier: ObjId) -> Vec<ObjId> {
    let Some(airlock) = crate::port::ship::hull_view(w, carrier).and_then(|view| view.airlock())
    else {
        return Vec::new();
    };
    crate::port::voyage::souls_aboard(w, airlock)
}

/// **The carrier's open hull epoch** (doc 23 S.2), through the command lens.
pub(crate) fn open_epoch(ctx: &Ctx<'_>, carrier: ObjId) -> Option<crate::arena::mind::SortieId> {
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    ctx.action_state(
        &handles.families.epochs,
        &crate::port::epoch::EpochKey(carrier),
    )
    .ok()
    .flatten()
    .and_then(|record| record.open.as_ref().map(|open| open.id))
}

/// **How far from the pad's cell a carrier may be stood**, cells (Chebyshev): several hulls
/// can be settled on one pad, and the kernel stands one unit in a cell.
pub const PAD_SPREAD_CELLS: u16 = 8;

/// **Each carrier's pad pose on the field's ground** (Q-CF-19), in unit order.
///
/// A settled hull's own placement is the domain's passive pose, not the pad the surface draws
/// ([`crate::surface::excursion::PAD_CENTRE_CM`]), so the field stands it there: the pad's cell
/// for the first carrier, then the nearest cell passable to it and free of every other unit
/// (`taken`), nearest ring first, row-major within a ring. Its heading is its own. The hull's
/// surface placement is never moved: a carrier stands still in a field (its spec's top speed
/// is nil) and goes home where it was.
///
/// # Errors
///
/// A carrier with no unit record or no placement, or no free cell within
/// [`PAD_SPREAD_CELLS`] of the pad.
pub(crate) fn pad_poses(
    w: &World,
    site: &CanonicalId,
    terrain: &crate::arena::state::ArenaTerrain,
    carriers: &[ObjId],
    taken: &mut std::collections::BTreeSet<(u16, u16)>,
) -> Result<std::collections::BTreeMap<ObjId, mush_sim::data::Pose>, String> {
    // Doc 32 cut S.1 (ruling 4): the site's own pad room.
    let pad = crate::surface::excursion::site_pad_cm(w, site);
    let (px, py) = terrain
        .cell_at(pad.0, pad.1)
        .ok_or_else(|| "the pad is off the field's ground".to_owned())?;
    let side = i64::from(terrain.cell_cm());
    let mut poses = std::collections::BTreeMap::new();
    for carrier in carriers {
        let cells = mush_tactics::module::unit_of(w, *carrier)
            .ok_or_else(|| format!("#{} has no tactical unit record", carrier.0))?;
        let row = crate::arena::state::CostRow::read(cells);
        let (_, own) = mush_sim::pose_of(w, *carrier)
            .ok_or_else(|| format!("#{} is not placed", carrier.0))?;
        let cell = (0..=PAD_SPREAD_CELLS)
            .flat_map(|ring| ring_cells((px, py), ring))
            .find(|cell| !taken.contains(cell) && terrain.passable_for(row, cell.0, cell.1))
            .ok_or_else(|| {
                format!(
                    "the pad has no free ground for #{} within {PAD_SPREAD_CELLS} cells",
                    carrier.0
                )
            })?;
        taken.insert(cell);
        let mut pose = own;
        pose.x_cm = i64::from(cell.0) * side + side / 2;
        pose.y_cm = i64::from(cell.1) * side + side / 2;
        poses.insert(*carrier, pose);
    }
    Ok(poses)
}

/// **How far from its cell a unit sharing it may be stood at admission**, cells (Chebyshev).
pub const UNSTACK_CELLS: u16 = 4;

/// **Two units on one cell, parted at admission** (doc 32 §17, the fix for G-4's "two hulls on
/// one pad launch their rigs onto one cell"). The kernel stands one unit in a cell, and the
/// surface does not: every hull on one pad launches its rigs onto the pad's one ramp cell, so a
/// second rig launched before the first drove off shared its cell, and admission refused the
/// whole field. Now, in unit order, the first unit keeps its cell; each later unit on a cell
/// already held is stood on the nearest cell (nearest ring first, row-major, within
/// [`UNSTACK_CELLS`]) that no other unit holds and it can stand on, at its own pose moved by
/// whole cells, so its heading and its place within the cell are kept. The move is a surface
/// placement, made before the check-out, like the settled rigs' (Q-CF-31), so the field admits
/// it there and it comes home from there. A surface with no two units on one cell is untouched,
/// so a launch, a drive and every "beside the ramp" test read exactly as before. `taken` gains
/// every unit's cell.
///
/// # Errors
///
/// A unit with no unit record, or no free cell near its own.
pub(crate) fn unstack(
    ctx: &mut Ctx<'_>,
    terrain: &crate::arena::state::ArenaTerrain,
    units: &[ObjId],
    taken: &mut std::collections::BTreeSet<(u16, u16)>,
) -> Result<(), String> {
    let side = i64::from(terrain.cell_cm());
    for unit in units {
        let Some((on, pose)) = mush_sim::pose_of(ctx.w, *unit) else {
            continue;
        };
        let Some(cell) = terrain.cell_of(&pose) else {
            continue;
        };
        if taken.insert(cell) {
            continue;
        }
        let cells = mush_tactics::module::unit_of(ctx.w, *unit)
            .ok_or_else(|| format!("#{} has no tactical unit record", unit.0))?;
        let row = crate::arena::state::CostRow::read(cells);
        let free = (1..=UNSTACK_CELLS)
            .flat_map(|ring| ring_cells(cell, ring))
            .find(|at| !taken.contains(at) && terrain.passable_for(row, at.0, at.1))
            .ok_or_else(|| {
                format!(
                    "#{} shares cell {cell:?} and no free cell is within {UNSTACK_CELLS}",
                    unit.0
                )
            })?;
        taken.insert(free);
        let mut at = pose.clone();
        at.x_cm += (i64::from(free.0) - i64::from(cell.0)) * side;
        at.y_cm += (i64::from(free.1) - i64::from(cell.1)) * side;
        mush_sim::place(ctx, *unit, on, at)
            .map_err(|error| format!("#{} could not be parted ({error})", unit.0))?;
        tracing::info!(
            unit = unit.0,
            from = ?cell,
            to = ?free,
            "tacsim: a unit sharing a cell is parted at admission"
        );
    }
    Ok(())
}

/// The cells `ring` cells from `centre` (Chebyshev), row-major; `ring 0` is the centre: the
/// kernel's own walk, which a join stands a carrier by (doc 36 cut GC.1b).
pub(crate) use crate::arena::join::ring_cells;

/// **The departure gate** (Q-H10): `Some(line)` when `hull` is held by a running field. The
/// gate of a queued departure, which never presumes the confirmation; a live one takes the
/// carrier out of the field instead (doc 32 F-6). The line names the units it would leave.
#[must_use]
pub fn departure_refusal(w: &World, hull: ObjId) -> Option<String> {
    let site = super::field_of_unit(w, hull)?;
    let left: Vec<String> = w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| {
            hosted
                .live
                .units
                .iter()
                .filter(|row| row.unit != hull)
                .map(|row| crate::port::coop::name_of(w, row.unit))
                .collect()
        })
        .unwrap_or_default();
    let units = if left.is_empty() {
        "nobody".to_owned()
    } else {
        left.join(", ")
    };
    Some(
        DEPART_IN_FIELD
            .replace("{name}", &crate::port::coop::name_of(w, hull))
            .replace("{site}", &crate::surface::act::site_name(w, &site))
            .replace("{units}", &units),
    )
}

/// **The carriers settled on `site`'s ground outside its running field** (doc 36 cut GC.1;
/// Q-36-4): a hull that arrived by voyage after admission. Q-CF-16 puts everyone on the ground
/// in the field, so each is owed its admission at its pad as a unit of its own party, which the
/// kernel's join makes ([`join_arrivals`], GC.1b). A launch from it is already refused while the site is promoted
/// (`FIELD_LAUNCH`, Q-CF-17). A hull left out under a licensed mind (Q-36-44) is no arrival, and
/// neither is a carrier that left this field by doc 32 F-6 (`record` names its exit, or its exit
/// is in flight: `crate::field::exit::leaving`): it stands on the pad outside the field until its
/// departure lifts it, and the host never takes it in again while that field runs.
#[must_use]
pub(crate) fn arrivals(
    w: &World,
    site: &CanonicalId,
    record: Option<&crate::surface::site_record::SiteRecord>,
) -> Vec<ObjId> {
    let Some(rows) = w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| hosted.live.units.clone())
    else {
        return Vec::new();
    };
    let Some(domain) = super::parties::site_domain(w, site) else {
        return Vec::new();
    };
    super::parties::on_ground(w, domain)
        .into_iter()
        .filter(|unit| {
            is_carrier(w, *unit)
                && !rows.iter().any(|row| row.unit == *unit)
                && !mush_sim::deployed::is_deployed(w, *unit)
                && !super::admit::held_out(w, *unit)
                && !record.is_some_and(|record| record.exited(*unit))
                && !super::exit::leaving(w, site, *unit)
        })
        .collect()
}

/// **Whether `hull` set down on a running field's pad and joined it** (doc 36 cut GC.1b;
/// Q-36-4): its live row says it arrived. The presence picture shows such a carrier as a row in
/// every party's picture, where one admitted at promotion is the pad.
#[must_use]
pub fn arrived_mid_field(w: &World, hull: ObjId) -> bool {
    let Some(site) = super::field_of_unit(w, hull) else {
        return false;
    };
    w.module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .is_some_and(|hosted| {
            hosted
                .live
                .units
                .iter()
                .any(|row| row.unit == hull && row.arrived)
        })
}

/// **The carriers owed admission to `site`'s running field** (doc 36 cut GC.1): what the tick
/// last noted, in id order; empty with no field running.
#[must_use]
pub fn owed_arrivals(w: &World, site: &CanonicalId) -> Vec<ObjId> {
    w.module_state::<FieldBook>()
        .and_then(|book| book.arrivals.get(site))
        .map(|hulls| hulls.iter().copied().collect())
        .unwrap_or_default()
}

/// **The host's owed arrivals on `site`, brought up to date** (doc 36 cut GC.1): the book holds
/// [`arrivals`] while the field runs and nothing once it is gone; each newly seen one is logged.
pub(crate) fn note_arrivals(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    let record = super::close::site_record(ctx, site);
    let now: std::collections::BTreeSet<ObjId> =
        arrivals(ctx.w, site, record.as_ref()).into_iter().collect();
    let Some(book) = ctx.w.module_state_mut::<FieldBook>() else {
        return;
    };
    let before = if now.is_empty() {
        book.arrivals.remove(site)
    } else {
        book.arrivals.insert(site.clone(), now.clone())
    }
    .unwrap_or_default();
    if book.hosted(site).is_none() {
        book.join_refused.remove(site);
    }
    for hull in now.difference(&before) {
        tracing::info!(site = %site, hull = hull.0, "tacsim: a carrier settled mid-field; it is owed its admission at its pad");
    }
}

/// **A carrier taken into a running field**, told to everyone on the site (doc 36 cut GC.1b;
/// Q-36-4): `{name}`, `{n}` the ordinal. A ship setting down is a physical fact anyone there
/// observes, never whose it is.
pub const ARRIVED_IN_FIELD: &str = "The {name} has set down on the pad and is in field {n}; \
     nothing launches from it while the field runs.";

/// **Take every owed arrival on `site` into its running field** (doc 36 cut GC.1b; Q-36-4), at
/// the tick after it is seen: each is checked out under the field's mark, given its row on the
/// live record (its party: the roster it launched, else its commander's party of one, on the
/// lowest letter no party holds when the field has no such party yet, doc 36a §11 row 2), and
/// sent to the kernel as
/// [`crate::arena_task::ArenaMsg::Join`] at its pad pose ([`pad_poses`]), which admits it at the
/// next pulse. The row is written beside the check-out, so a restart before the kernel took it
/// finds the row and the resume joins it again (`super::boot`). A hull the kernel refused is not
/// asked again while this field runs.
pub(crate) fn join_arrivals(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    let refused = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.join_refused.get(site).cloned())
        .unwrap_or_default();
    for hull in owed_arrivals(ctx.w, site) {
        if refused.contains(&hull) {
            continue;
        }
        match join(ctx, site, hull) {
            Ok(()) => {
                if let Some(book) = ctx.w.module_state_mut::<FieldBook>()
                    && let Some(owed) = book.arrivals.get_mut(site)
                {
                    owed.remove(&hull);
                }
            }
            Err(why) => {
                tracing::warn!(site = %site, hull = hull.0, reason = %why, "tacsim: an arriving carrier could not be taken into the field yet");
            }
        }
    }
}

/// **What a join hands the kernel for one hull** (doc 36 cut GC.1b): its check-out row, its
/// seat, and the host's standing friendships and marks naming its party (Q-CF-5, Q-CF-44),
/// beside the live record with the hull's row (and its party, when new) added.
pub(crate) struct Joining {
    /// The check-out row.
    pub unit: crate::arena::state::CustodyUnit,
    /// Its seat.
    pub pilot: Option<crate::arena::state::Pilot>,
    /// The host's rules for its party.
    pub host: crate::arena::FieldHost,
    /// The live record with its row.
    pub live: super::FieldLive,
}

/// **Build `hull`'s join into the field `live` records** (doc 36 cut GC.1b), at `pose`. A resume
/// that finds the hull's row already on the live record keeps its party.
///
/// # Errors
///
/// The hull is not on the ground, the alphabet is spent, or its check-out row cannot be built.
pub(crate) fn joining(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    live: &super::FieldLive,
    hull: ObjId,
    pose: mush_sim::data::Pose,
) -> Result<Joining, String> {
    use super::record::{FieldParty, FieldUnit};
    let mut live = live.clone();
    let units: Vec<ObjId> = live
        .units
        .iter()
        .map(|row| row.unit)
        .filter(|unit| *unit != hull)
        .chain([hull])
        .collect();
    let entrants = super::parties::entrants(ctx, site, &units);
    let entrant = entrants
        .iter()
        .find(|entrant| entrant.unit == hull)
        .cloned()
        .ok_or_else(|| format!("#{} is not on the ground", hull.0))?;
    let known = live
        .units
        .iter()
        .find(|row| row.unit == hull)
        .map(|row| row.party)
        .or_else(|| {
            live.parties
                .iter()
                .find(|party| party.key == entrant.key)
                .map(|party| party.party)
        });
    let mut allied: Vec<(crate::arena::PartyId, crate::arena::PartyId)> = Vec::new();
    let party = match known {
        Some(party) => party,
        None => {
            // Doc 36a §11 row 2: the lowest free letter, never the count. A refused join
            // (`on_joined`) leaves a gap in the middle of the record, and a count would hand the
            // next arrival the last letter in use: another party's relations and marks, and a
            // record the boot refuses.
            let party = live
                .next_party()
                .ok_or("more parties are on the ground than a field letters")?;
            // Doc 40 cut FR.1: a refused join (`on_joined`) frees its letter but leaves it on the
            // other parties' standing rows; the letter's new holder opens under its own stances.
            for row in &mut live.parties {
                row.stance_hostile.retain(|of| *of != party);
            }
            live.parties.push(FieldParty {
                party,
                key: entrant.key,
                leader: entrant.leader,
                stance_hostile: Vec::new(),
                director: entrant.director,
            });
            live.parties.sort_by_key(|party| party.party);
            // Doc 40 cut FR.1 (Q-38-23, §4.2): the stance table read at the join, for the pairs
            // the new party opens only — every other pair keeps what the field was built with.
            let stances = super::parties::opening_stances(ctx.w, &entrants, &live.parties);
            for (by, of) in stances
                .hostile
                .iter()
                .filter(|(by, of)| *by == party || *of == party)
            {
                if let Some(row) = live.parties.iter_mut().find(|row| row.party == *by)
                    && !row.stance_hostile.contains(of)
                {
                    row.stance_hostile.push(*of);
                    row.stance_hostile.sort_unstable();
                }
            }
            allied = stances
                .allied
                .into_iter()
                .filter(|(a, b)| *a == party || *b == party)
                .collect();
            party
        }
    };
    if !live.units.iter().any(|row| row.unit == hull) {
        live.units.push(FieldUnit {
            unit: hull,
            party,
            pilot: entrant.pilot.clone(),
            epoch: entrant.epoch,
            aboard: entrant.aboard.clone(),
            arrived: true,
        });
        live.units.sort_by_key(|row| row.unit);
    }
    let mut friends: Vec<(crate::arena::PartyId, crate::arena::PartyId)> =
        super::parties::friends(&entrants, &live.parties)
            .into_iter()
            .filter(|(a, b)| *a == party || *b == party)
            .collect();
    for pair in allied {
        if !friends.contains(&pair) {
            friends.push(pair);
        }
    }
    friends.sort_unstable();
    // The standing Hostile marks naming the party, from the live record's rows (doc 40 cut
    // FR.1): a fixture raider's on every other party (doc 32 cut CF.6, Q-CF-44), a newcomer's
    // too, and each declared Hostile the join opened.
    let marks = super::parties::standing_marks(&live.parties, Some(party));
    let (unit, pilot) = super::admit::custody_unit(ctx, &entrant, party, pose, 0)?;
    Ok(Joining {
        unit,
        pilot,
        host: crate::arena::FieldHost {
            all_hostile: false,
            marks,
            friends,
            end: crate::arena::party::EndCondition::Host,
            every_class: true,
            weapons_hold: true,
        },
        live,
    })
}

/// **`hull`'s pad pose on the field's ground** (Q-CF-19): [`pad_poses`] for it alone; the kernel
/// stands it on the nearest free cell when another unit holds that one.
///
/// # Errors
///
/// As [`pad_poses`]'.
pub(crate) fn pad_pose(
    w: &World,
    site: &CanonicalId,
    terrain: &crate::arena::state::ArenaTerrain,
    hull: ObjId,
) -> Result<mush_sim::data::Pose, String> {
    let mut taken = std::collections::BTreeSet::new();
    pad_poses(w, site, terrain, &[hull], &mut taken)?
        .remove(&hull)
        .ok_or_else(|| format!("#{} has no pad pose", hull.0))
}

/// One arrival's join ([`join_arrivals`]).
fn join(ctx: &mut Ctx<'_>, site: &CanonicalId, hull: ObjId) -> Result<(), String> {
    let (task, live) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| (hosted.task, hosted.live.clone()))
        .ok_or_else(|| "no field runs there".to_owned())?;
    let (ground, tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .and_then(|registry| Some((registry.ground_of(task)?, registry.tasks().clone())))
        .ok_or_else(|| "the field's task is gone".to_owned())?;
    let pose = pad_pose(ctx.w, site, &ground.terrain, hull)?;
    let joining = joining(ctx, site, &live, hull, pose)?;
    // Doc 36a lane 3 (P0-8): the site's record must have a row for everyone on the ground.
    if let Some(domain) = super::parties::site_domain(ctx.w, site) {
        super::admit::index_room(ctx.w, domain)?;
    }
    let mark = mush_sim::deployed::DeployedMark::field(site.as_str(), live.field.ordinal);
    mush_tactics::custody::check_out(ctx, hull, mark)
        .map_err(|error| format!("#{} could not be taken into the field ({error})", hull.0))?;
    let controller = joining.unit.controller;
    let armed = !joining.unit.weapons.is_empty();
    let message = crate::arena_task::ArenaMsg::Join {
        unit: Box::new(joining.unit),
        pilot: joining.pilot,
        host: joining.host,
        spread: PAD_SPREAD_CELLS,
    };
    if tasks.offer(task, message).is_err() {
        let _ = mush_tactics::custody::check_in(ctx, hull, None);
        return Err("the field is busy".to_owned());
    }
    set_live(ctx, site, joining.live);
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        registry.bind_joining(hull, task, controller, armed);
    }
    tracing::info!(site = %site, hull = hull.0, task = task.0, "tacsim: an arriving carrier is joining the field");
    Ok(())
}

/// The host's live record of `site`, replaced and written. A record that breaks its own
/// invariants replaces nothing (doc 36a §11 row 2): the book and the lane never hold what the
/// boot would refuse.
fn set_live(ctx: &mut Ctx<'_>, site: &CanonicalId, live: super::FieldLive) {
    if let Err(error) = live.check() {
        tracing::error!(%error, site = %site, "tacsim: a field's live record is not replaced: it breaks its own invariants");
        return;
    }
    if let Some(hosted) = ctx
        .w
        .module_state_mut::<FieldBook>()
        .and_then(|book| book.live.get_mut(site))
    {
        hosted.live = live.clone();
    }
    super::record::write_live(ctx, &live);
}

/// **The kernel's answer to a join** (the bridge's branch; doc 36 cut GC.1b): taken in, the site
/// is told; refused, the hull is checked back in as it went out, its row leaves the live record,
/// and it is not asked again while this field runs.
pub(crate) fn on_joined(
    ctx: &mut Ctx<'_>,
    task: crate::arena_task::MatchId,
    unit: ObjId,
    outcome: Result<crate::arena::state::Designator, String>,
) {
    let Some((site, live)) = ctx.w.module_state::<FieldBook>().and_then(|book| {
        let site = book.site_of_task(task)?.clone();
        let live = book.hosted(&site)?.live.clone();
        Some((site, live))
    }) else {
        return;
    };
    match outcome {
        Ok(name) => {
            tracing::info!(site = %site, hull = unit.0, name = ?name, "tacsim: an arriving carrier joined the field");
            let line = ARRIVED_IN_FIELD
                .replace("{name}", &crate::port::coop::name_of(ctx.w, unit))
                .replace("{n}", &live.field.ordinal.to_string());
            super::tell_site(ctx, &site, &line);
        }
        Err(why) => {
            tracing::warn!(site = %site, hull = unit.0, reason = %why, "tacsim: the field refused an arriving carrier; it stands outside the field");
            let _ = mush_tactics::custody::check_in(ctx, unit, None);
            let mut live = live;
            live.units.retain(|row| row.unit != unit);
            // Its own party of one, when the join lettered one for it.
            live.parties
                .retain(|party| party.key != super::record::PartyKey::Unit { unit });
            set_live(ctx, &site, live);
            if let Some(registry) = ctx
                .w
                .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
            {
                registry.unbind_joining(unit, task);
            }
            if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
                book.join_refused.entry(site).or_default().insert(unit);
            }
        }
    }
}

/// **The tow gate for a carrier** (FO-13; doc 36 cut GC.3, Q-36-8 as ruled): `Some(line)` when
/// `target` is a carrier and `tug` is lighter than half its tonnage
/// ([`crate::carrier_damage::tug_floor_kg`]), or either mass does not read. No shipped design is
/// that heavy, so in practice every carrier is refused.
#[must_use]
pub fn refuse_tow(w: &World, target: ObjId, tug: ObjId) -> Option<String> {
    if !is_carrier(w, target) {
        return None;
    }
    let carrier_kg = crate::carrier_damage::mass_kg(w, target);
    let tug_kg = crate::carrier_damage::mass_kg(w, tug);
    let floor_kg = carrier_kg.map(crate::carrier_damage::tug_floor_kg);
    if let (Some(floor_kg), Some(tug_kg)) = (floor_kg, tug_kg)
        && tug_kg >= floor_kg
    {
        return None;
    }
    let tonnes = |kg: Option<u64>| kg.map_or_else(|| "?".to_owned(), |kg| (kg / 1_000).to_string());
    Some(
        TOW_CARRIER
            .replace("{name}", &crate::port::coop::name_of(w, target))
            .replace("{t}", &tonnes(carrier_kg))
            .replace("{floor}", &tonnes(floor_kg))
            .replace("{tug}", &crate::port::coop::name_of(w, tug))
            .replace("{tug_t}", &tonnes(tug_kg)),
    )
}
