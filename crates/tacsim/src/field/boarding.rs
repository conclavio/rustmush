//! **A player's hull boards only a carrier still there** (doc 40, the ramp exit's boarding
//! window, extended to player hulls; the owner's ruling, 2026-11-04, "back in the field").
//!
//! A recovery or a load counts only once the hull is aboard. Three paths leave a running field at
//! a carrier's ramp and board as a later step: `+tv/<n> recover` (`super::exit::Then::Rig`, the
//! rig's recover act), `+sortie/recover` (`Then::Sortie`, the DT recover) and FR.5b's coupled
//! `+tv/<n> load go` (`Then::TowLoad` and its casualty's `Then::Towed`, the tow's load act). If the
//! carrier is wrecked or gone before the hull is aboard, the hull stays on the site **in the
//! field**, its pilot still flying it. Three pieces hold that:
//!
//! 1. **The kernel's check at the release.** Every ask names the carrier the hull boards
//!    ([`carrier_of`]), and the arena keeps the hull while that carrier is a wreck or not in the
//!    field (`crate::arena::withdraw::WithdrawRefusal::CarrierLost`), as #468 does for a raider.
//!    The hull never left, so its pilot keeps its seat and its frames.
//! 2. **Each boarding re-checks the carrier when it lands** ([`carrier_lost`]): the rig's recover
//!    places nothing in the bay (`crate::surface::tick`'s world step), the DT recover's due step
//!    places nothing and its finish puts the flight back in the field
//!    (`crate::detachment::sortie`), and the tow's load leaves the casualty where it stands
//!    (`crate::surface::tow`).
//! 3. **A hull out at the ramp whose carrier is lost goes back into the field** ([`tick`]): the
//!    host remembers every hull let go at a ramp for one of those paths ([`Boarding`]) until it is
//!    aboard, and while it stands on the site's ground beside the ramp with its carrier lost it is
//!    taken back in by the kernel's join where it stands (`super::carrier::rejoin`), its pilot
//!    seated again, a coupled pair coupled again. Its exit still on its way is dropped, so nothing
//!    boards it later.
//!
//! **Why this shape, not one batch.** A raider's hull goes aboard in the batch that lets it go
//! (#468); a player's cannot, since the recover, the DT recover and the load are surface acts
//! with their own durations, records and refusals that run after the exit lands. Moving them into
//! `on_withdrawn` would rewrite three surface acts; the re-check and the join leave each act as it
//! was.
//!
//! **What the host knows of a carrier in a field.** The carrier's own record is checked out and
//! stale while the field runs; the kernel's ledger names each unit destroyed
//! (`EventKind::UnitDestroyed`), which [`note_chunk`] keeps by task, and a resume seeds it from
//! the snapshot it resumes ([`seed`]). A carrier is **lost** while a field runs on the site and
//! it is one of those, or it no longer stands settled on the site (lifted, gone). With no field
//! running nothing here applies.
//!
//! **A hull aboard a carrier wrecked later** is a wreck in the bay: doc 36 cut GC.3's
//! `wreck_bay_rigs` already wrecks every hull placed in a wrecked carrier's bay rooms
//! (`crate::port::vehicle::rigs_in_bay`), the rig, a DT unit and a loaded casualty alike.
//!
//! **Across a restart** [`Boarding`] is RAM: a hull out at the ramp when the server went down is
//! not remembered, and stands beside the ramp out of the field until its recover runs, as before
//! (the re-check at landing still refuses to board a lost carrier).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId, World};

use super::FieldBook;
use super::exit::Then;
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena_task::MatchId;

/// **One hull out at a ramp, not yet aboard** ([`Boarding`]).
#[derive(Clone, Debug)]
pub(crate) struct Board {
    /// The site whose field let it go.
    pub site: CanonicalId,
    /// The carrier it boards.
    pub carrier: ObjId,
    /// Who asked (told when it goes back in).
    pub requester: ObjId,
    /// The path: `Rig`, `Sortie`, `TowLoad` or `Towed`.
    pub then: Then,
}

/// **The hulls let go at a ramp and not yet aboard**, by unit (module state, RAM).
#[derive(Clone, Debug, Default)]
pub(crate) struct Boarding(pub BTreeMap<ObjId, Board>);

/// **The units each field's kernel named destroyed**, by task (RAM; [`note_chunk`], [`seed`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Destroyed(pub BTreeMap<MatchId, BTreeSet<ObjId>>);

/// The line the pilot hears when a hull out at the ramp goes back into the field.
pub const BACK_IN_FIELD: &str = "The {carrier} is lost before #{unit} is aboard: it stays in field {ordinal}, and you still have it.";

/// Whether `then` is a path this module holds.
pub(crate) const fn holds(then: &Then) -> bool {
    matches!(
        then,
        Then::Rig { .. } | Then::Sortie { .. } | Then::TowLoad { .. } | Then::Towed { .. }
    )
}

/// **The carrier `unit` boards on its way out** (this module's head, 1): a raider's fixture
/// carrier; a rig's own hull, from its open excursion; a DT unit's flight's carrier; a casualty in
/// tow, its tug's.
pub(crate) fn carrier_of(ctx: &Ctx<'_>, unit: ObjId, then: &Then) -> Option<ObjId> {
    match then {
        Then::Raider => super::raiders::carrier_of(ctx.w, unit),
        Then::Rig { .. } | Then::TowLoad { .. } => rig_hull(ctx, unit),
        Then::Towed { tug } => rig_hull(ctx, *tug),
        Then::Sortie { .. } => crate::detachment::sortie::field_carrier(ctx, unit),
        Then::Lift { .. } | Then::RaiderLift => None,
    }
}

/// A rig's hull, from its open excursion.
fn rig_hull(ctx: &Ctx<'_>, tv: ObjId) -> Option<ObjId> {
    let handles = ctx.w.module_state::<crate::port::families::PortHandles>()?;
    ctx.action_state(
        &handles.families.excursions,
        &crate::surface::excursion::RigKey(tv),
    )
    .ok()
    .flatten()
    .and_then(|record| record.excursion.as_ref().map(|excursion| excursion.hull))
}

/// **Whether `carrier` is lost to a hull boarding it on `site`** (this module's head): a field
/// runs on the site, and the carrier is one its kernel named destroyed, or it no longer stands
/// settled on the site. `false` with no field running.
#[must_use]
pub fn carrier_lost(w: &World, site: &CanonicalId, carrier: ObjId) -> bool {
    let Some(book) = w.module_state::<FieldBook>() else {
        return false;
    };
    let Some(hosted) = book.hosted(site) else {
        return false;
    };
    let settled =
        crate::surface::tick::hull_view_berth(w, carrier).is_some_and(|(entry, ..)| entry == *site);
    if !settled {
        return true;
    }
    w.module_state::<Destroyed>()
        .and_then(|destroyed| destroyed.0.get(&hosted.task))
        .is_some_and(|units| units.contains(&carrier))
}

/// **The units a chunk of `task`'s ledger names destroyed** (this module's head; the bridge's
/// chunk reader).
pub(crate) fn note_chunk(ctx: &mut Ctx<'_>, task: MatchId, events: &[LedgerEvent]) {
    let fallen: Vec<ObjId> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::UnitDestroyed { unit, .. } => Some(*unit),
            _ => None,
        })
        .collect();
    if fallen.is_empty() {
        return;
    }
    if let Some(destroyed) = ctx.w.module_state_mut::<Destroyed>() {
        destroyed.0.entry(task).or_default().extend(fallen);
    }
}

/// **A resumed field's wrecks** (this module's head): the units the snapshot it resumes holds as
/// wrecks, under its new task.
pub(crate) fn seed(w: &mut World, task: MatchId, wrecks: BTreeSet<ObjId>) {
    if wrecks.is_empty() {
        return;
    }
    if let Some(destroyed) = w.module_state_mut::<Destroyed>() {
        destroyed.0.entry(task).or_default().extend(wrecks);
    }
}

/// **Remember a hull let go at a ramp** for a path this module holds (`super::exit::on_withdrawn`).
pub(crate) fn note_out(ctx: &mut Ctx<'_>, unit: ObjId, board: Board) {
    if !holds(&board.then) {
        return;
    }
    if let Some(boarding) = ctx.w.module_state_mut::<Boarding>() {
        boarding.0.insert(unit, board);
    }
}

/// **The host's pass** (this module's head, 3): each hull out at a ramp is forgotten once it is
/// aboard (off the site's ground), back in a field, or its site's field is gone; one whose
/// carrier is lost while it stands on the site's ground goes back into the field. A pair goes
/// back tug first, so its casualty joins the tug's party coupled.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let rows: Vec<(ObjId, Board)> = ctx
        .w
        .module_state::<Boarding>()
        .map(|boarding| {
            boarding
                .0
                .iter()
                .map(|(unit, board)| (*unit, board.clone()))
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let mut forget: Vec<ObjId> = Vec::new();
    let mut back: Vec<(ObjId, Board)> = Vec::new();
    for (unit, board) in rows {
        // Doc 40 (the 38b review's F-15): a join with the kernel keeps its row until the
        // kernel's answer (`super::carrier::on_joined`): taken, the row goes there; refused, it
        // is asked again from here.
        let pending = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.rejoining.get(&board.site))
            .is_some_and(|hulls| hulls.contains(&unit));
        if pending {
            continue;
        }
        let running = ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.hosted(&board.site))
            .map(|hosted| hosted.live.units.iter().any(|row| row.unit == unit));
        let Some(held) = running else {
            forget.push(unit);
            continue;
        };
        let on_site = super::parties::site_domain(ctx.w, &board.site).is_some_and(|domain| {
            mush_sim::pose_of(ctx.w, unit).is_some_and(|(on, _)| on == domain)
        });
        if held || !on_site {
            forget.push(unit);
            continue;
        }
        if carrier_lost(ctx.w, &board.site, board.carrier) {
            back.push((unit, board));
        }
    }
    if let Some(boarding) = ctx.w.module_state_mut::<Boarding>() {
        for unit in &forget {
            boarding.0.remove(unit);
        }
    }
    // The tug before its casualty.
    back.sort_by_key(|(_, board)| matches!(board.then, Then::Towed { .. }));
    for (unit, board) in back {
        go_back(ctx, unit, &board);
    }
}

/// **`unit` back into its site's field** ([`tick`]): its exit in flight dropped, then the
/// kernel's join where it stands, a casualty in its tug's party and coupled to it.
fn go_back(ctx: &mut Ctx<'_>, unit: ObjId, board: &Board) {
    // The pair: the casualty's tug, or the tug's casualty, by the paths' own words.
    let tow = match &board.then {
        Then::Towed { tug } => Some((unit, *tug)),
        Then::TowLoad { casualty, .. } => Some((*casualty, unit)),
        _ => None,
    };
    let party = match &board.then {
        Then::Towed { tug } => ctx
            .w
            .module_state::<FieldBook>()
            .and_then(|book| book.hosted(&board.site))
            .and_then(|hosted| hosted.live.party_of(*tug)),
        _ => None,
    };
    if matches!(board.then, Then::Towed { .. }) && party.is_none() {
        // Its tug is not back in yet: the next pass.
        return;
    }
    match super::carrier::rejoin(ctx, &board.site, unit, party, tow) {
        Ok(()) => {
            // Doc 40 (the 38b review's F-15): its exit on its way dropped and the record's struck;
            // its row stays until the kernel takes the join (`super::carrier::on_joined`).
            super::exit::forget(ctx, &board.site, unit, board.requester);
            let ordinal = ctx
                .w
                .module_state::<FieldBook>()
                .and_then(|book| book.hosted(&board.site))
                .map_or(0, |hosted| hosted.live.field.ordinal);
            if !matches!(board.then, Then::Towed { .. }) {
                let line = BACK_IN_FIELD
                    .replace(
                        "{carrier}",
                        &crate::port::coop::name_of(ctx.w, board.carrier),
                    )
                    .replace("{unit}", &unit.0.to_string())
                    .replace("{ordinal}", &ordinal.to_string());
                ctx.out.sys_to_actor(board.requester, "tacsim.field", line);
            }
            tracing::info!(site = %board.site, unit = unit.0, carrier = board.carrier.0, "tacsim: a hull out at the ramp goes back into the field, its carrier lost");
        }
        Err(why) => {
            tracing::warn!(site = %board.site, unit = unit.0, reason = %why, "tacsim: a hull out at the ramp could not go back into the field yet");
        }
    }
}
