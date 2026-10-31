//! **NPC units go home up their own carrier's ramp** (doc 32 cut CF.6b; Q-CF-18, Q-CF-42,
//! Q-CF-44, Q-CF-47).
//!
//! A unit flown by the doctrine leaves a field the way a pilot's does — beside its own carrier's
//! ramp, under its own power, through CF.5's exit (`super::exit`) — and gets there by the
//! doctrine's own `Withdraw`, never a second AI:
//!
//! 1. **Every party has a home** ([`homes`]): the ramp of the carrier its units came from — a DT
//!    flight's carrier, a rig's hull, a raider's fixture carrier — one cell, written on the
//!    field's ground (`crate::arena::state::ArenaDefinition::homes`) and kept on the live record
//!    so a resume fights on the same ground. A field has no entry zones, so a `Withdraw` drives
//!    home there (`crate::arena::motor::entry_cell`); nothing else reads a home.
//! 2. **A delivered WITHDRAW** (a roster's, `super::orders`; a raider leader's,
//!    `super::raiders::withdraw`) becomes the kernel's `Withdraw`: break off and drive home.
//!    Under the order's HOLD FIRE it fires on nothing on the way (a raider leader's always holds
//!    fire); without it, it stands and fires when cornered, as the doctrine's own `Withdraw`
//!    does, and goes on once it is not.
//! 3. **The host asks it out** ([`tick`], [`ask_out`]): once a world second the field is asked to
//!    let the unit go at its ramp, and the kernel decides on its own live pose and damage
//!    (`crate::arena::withdraw`) — refused while it is still on its way, let go once it stands
//!    beside the ramp. Out, it is checked in like any exit and its recover runs: the DT recover
//!    into the bay, or aboard the raider's carrier.
//!
//! **An NPC with no reachable ramp** (the rule, recorded in doc 32's "CF.6b as built"): its party
//! has no home — its carrier is not on the site's ground (gone, or never there), or the ramp's
//! cell is not ground it can stand on — or it cannot come in under its own power. The `Withdraw`
//! still breaks it off: it holds where it stands, firing only when cornered, and stays in the
//! field. Nothing removes it by another way (Q-CF-18: no edge exit; Q-H3: withdrawal through reachable exits only); it is
//! on the site record where it stood once the field demotes (Q-H4), and its crew's way home is
//! extraction (Q-CF-39), never automatic.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, ObjId, World};

use super::exit::{ExitBook, Then};
use super::parties::Entrant;
use super::record::FieldUnit;
use crate::arena::state::EntryZone;

/// **The units the host last asked out, and the world second it asked**, and the raiders' hulls
/// whose WITHDRAW this boot's task has taken (RAM, module state: a restart offers each again).
#[derive(Clone, Debug, Default)]
pub(crate) struct Asked {
    at: BTreeMap<ObjId, i64>,
    ordered: BTreeSet<ObjId>,
}

/// **The ramp `carrier` comes home to, as a site cell**: a raider's fixture carrier's authored
/// ramp, else a programme hull's bay ramp on its pad.
fn ramp_cell(w: &World, carrier: ObjId) -> Option<(u16, u16)> {
    let (x, y) = match super::raiders::ramp_cell(w, carrier) {
        Some([x, y]) => (x, y),
        None => {
            let (x_cm, y_cm) = super::exit::rig_ramp(w, carrier)?;
            crate::surface::excursion::site_cell(x_cm, y_cm)
        }
    };
    Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?))
}

/// **Each party's home** (this module's head, 1): the ramp of the first carrier on the ground its
/// units came from, in unit order; a party whose carrier is not admitted with it has none.
pub(crate) fn homes(w: &World, entrants: &[Entrant], rows: &[FieldUnit]) -> Vec<EntryZone> {
    let mut homes: BTreeMap<crate::arena::PartyId, (u16, u16)> = BTreeMap::new();
    for entrant in entrants {
        let Some(carrier) = entrant.carrier else {
            continue;
        };
        if !entrants.iter().any(|other| other.unit == carrier) {
            continue;
        }
        let Some(party) = rows
            .iter()
            .find(|row| row.unit == entrant.unit)
            .map(|row| row.party)
        else {
            continue;
        };
        if homes.contains_key(&party) {
            continue;
        }
        if let Some(cell) = ramp_cell(w, carrier) {
            homes.insert(party, cell);
        }
    }
    homes
        .into_iter()
        .map(|(team, (x, y))| EntryZone {
            team,
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        })
        .collect()
}

/// **Ask the field holding `unit` to let it go at its ramp**, at most once a world second, as
/// `requester` (an NPC: the refusals it is told reach nobody). Nothing when the unit's party has
/// no home (the head's rule), its exit is already in flight, or it names no ramp.
pub(crate) fn ask_out(ctx: &mut Ctx<'_>, unit: ObjId, requester: ObjId, then: Then) {
    let Some(site) = super::field_of_unit(ctx.w, unit) else {
        return;
    };
    let homed = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(&site))
        .and_then(|hosted| {
            let party = hosted.live.party_of(unit)?;
            Some(hosted.live.homes.iter().any(|home| home.team == party))
        })
        .unwrap_or(false);
    if !homed
        || ctx
            .w
            .module_state::<ExitBook>()
            .is_some_and(|book| book.0.contains_key(&unit))
    {
        return;
    }
    let now = ctx.w.now();
    if ctx
        .w
        .module_state::<Asked>()
        .and_then(|asked| asked.at.get(&unit))
        .is_some_and(|at| *at >= now)
    {
        return;
    }
    let ramp = match then {
        Then::Raider => super::raiders::hull_ramp(ctx.w, unit),
        Then::Sortie { .. } => crate::detachment::sortie::field_ramp(ctx, unit),
        Then::Rig { .. } | Then::Lift { .. } => None,
    };
    let Some(ramp) = ramp else {
        return;
    };
    if let Some(asked) = ctx.w.module_state_mut::<Asked>() {
        asked.at.insert(unit, now);
    }
    let saved = ctx.actor;
    ctx.actor = requester;
    // A refusal here (the field busy, not yet running) is offered again next second.
    let _ = super::exit::request(ctx, unit, Some(ramp), then);
    ctx.actor = saved;
}

/// **The host's pass for raiders** (this module's head, 3): every raider's hull in a running
/// field under its leader's WITHDRAW is offered the order until this boot's task has taken it
/// (so a restart or a busy queue never loses it; a duplicate the kernel refuses harmlessly) and
/// asked out at its ramp.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let units: Vec<ObjId> = ctx
        .w
        .module_state::<super::FieldBook>()
        .map(|book| {
            book.live
                .values()
                .flat_map(|hosted| hosted.live.units.iter().map(|row| row.unit))
                .collect()
        })
        .unwrap_or_default();
    for unit in units {
        if !super::raiders::withdrawing(ctx.w, unit) {
            continue;
        }
        let Some(leader) = super::raiders::leader_of(ctx.w, unit) else {
            continue;
        };
        let ordered = ctx
            .w
            .module_state::<Asked>()
            .is_some_and(|asked| asked.ordered.contains(&unit));
        if !ordered
            && super::raiders::order_home(ctx, unit)
            && let Some(asked) = ctx.w.module_state_mut::<Asked>()
        {
            asked.ordered.insert(unit);
        }
        ask_out(ctx, unit, leader, Then::Raider);
    }
    // Forget the asks of units no field holds any more.
    let held: Vec<ObjId> = ctx
        .w
        .module_state::<Asked>()
        .map(|asked| asked.at.keys().chain(&asked.ordered).copied().collect())
        .unwrap_or_default();
    for unit in held {
        if super::field_of_unit(ctx.w, unit).is_none()
            && let Some(asked) = ctx.w.module_state_mut::<Asked>()
        {
            asked.at.remove(&unit);
            asked.ordered.remove(&unit);
        }
    }
}
