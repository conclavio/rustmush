//! **A player discharges hired crew at a port** (doc 48 cut EX.7b).
//!
//! `+sortie/discharge #<npc>` ends a hire by its employer's choice, from any port's station. It
//! is doc 42's capture exit (Q-42-26) taken by choice, in the same two halves:
//!
//! - **The world half, at once** (`discharge`): the final wage is settled
//!   (`wages::settle_final`: the IC day in progress charged whole beside the arrears, each paid
//!   if the account covers it, and the wage ended, so no further period accrues); the crew member leaves any seat they hold in a
//!   unit standing in a carrier's bay and is back in their hire port's pool room, where the
//!   offers already put an uncontracted NPC on offer again; and the exit is owed to the roster
//!   (`crate::field::NpcExit`).
//! - **The roster's half**: `tacsim_crew_captured`, the one action over both families a hire
//!   holds, submitted here as the employer and again by the tow tick until it lands. The
//!   contract and the roster reservation end together, so the roster's seat is free with it.
//!
//! **Refused**, each with its own sentence: a character who is not the hire's employer; an
//! employer away from every port's station; a crew member in custody, in an open field, or out
//! on a sortie (inside a unit that is not standing in a carrier's bay, or whose flight is open).
//! **Never refused for want of money**: what the employer cannot pay stands as their debt.

use mush_core::{Ctx, Flags, ObjId, ObjKind, World, WorldEvent};

use super::CrewHandles;

/// `+sortie/discharge` usage.
pub const DISCHARGE_USAGE: &str =
    "Usage: +sortie/discharge #<npc>, from a port's station, for a hire of your own.";
/// Not the employer, or no such contract.
pub const NOT_YOURS: &str =
    "No crew contract of yours names that character: only the employer discharges a hire.";
/// The employer is away from every port's station.
pub const AT_PORT: &str =
    "Discharge a hire from a port's station: its hall settles the final wage.";
/// The crew member is out with a unit: `{name}`.
pub const ON_SORTIE: &str =
    "{name} is out on a sortie: bring the unit back to its carrier's bay and end the flight first.";
/// The crew member is in an open field: `{name}`.
pub const IN_FIELD: &str =
    "{name} is in an open field: discharge them once the field is behind them.";
/// The crew member is in custody: `{name}`.
pub const CAPTIVE: &str = "{name} is in custody: a hire another party holds cannot be discharged.";
/// One of the employer's own acts is still in flight.
pub const BUSY: &str = "One of your requests is still being settled: discharge again in a moment.";
/// The economy refused the settlement, or the hire has no pool room to go back to.
pub const NOT_NOW: &str = "The hiring hall cannot settle a discharge just now: nothing has changed, so try again shortly.";
/// A discharge whose final wage is paid: `{name}`, `{paid}` and `{payee}`.
pub const DISCHARGED_PAID: &str = "{name} is discharged: {paid} paid to {payee} as the final wage.";
/// A discharge whose final wage the employer cannot pay: `{name}`, `{owing}` and `{payee}`.
pub const DISCHARGED_OWING: &str =
    "{name} is discharged: the final wage, {owing}, stands as your debt to {payee}.";
/// A discharge whose final wage is paid in part: `{name}`, `{paid}` and `{owing}`.
pub const DISCHARGED_PART: &str =
    "{name} is discharged: {paid} of the final wage is paid and {owing} stands as your debt.";
/// A discharge that owed no wage: `{name}`.
pub const DISCHARGED_FREE: &str = "{name} is discharged: no wage was charged, so nothing is owed.";
/// Where the crew member went: `{name}` and `{room}`.
pub const RETURNED: &str = "The contract is ended. {name} is back at {room}, on offer there again.";
/// `+sortie/contract` for an ended hire whose wage still stands as a debt: `{npc}`, `{owing}`.
pub const STANDING_DEBT: &str = "No crew contract with #{npc} stands; {owing} of its wage is your debt, collected when your account covers it.";
/// What the discharged crew member is told.
const NPC_TOLD: &str = "Your employer has discharged you: your crew agreement is ended and your final wage settled. You are back at the hiring hall, free to take another offer.";

/// The NPC a discharge names.
fn named(raw: &str) -> Option<ObjId> {
    let raw = raw.trim();
    let digits = raw.strip_prefix('#').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|id| *id > 0).map(ObjId)
}

/// Where a hire stands, as a discharge reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// In an ordinary room: a station, or anywhere on foot.
    Ashore,
    /// In the rooms of a ship.
    Ship(ObjId),
    /// Inside a unit.
    Unit(ObjId),
}

/// **Where `who` stands**: the ship whose rooms they are in, else the unit they are inside (its
/// own object, or the moving object their room is an interior of), else ashore.
fn standing(w: &World, who: ObjId) -> Standing {
    let Some(mut here) = w.get(who).and_then(|o| o.location) else {
        return Standing::Ashore;
    };
    if let Some((view, _)) = crate::port::ship::hull_of_room(w, here) {
        return Standing::Ship(view.hull());
    }
    for _ in 0..8 {
        if mush_tactics::module::unit_of(w, here).is_some() {
            return Standing::Unit(here);
        }
        let host = mush_sim::anchored_space_of(w, here).and_then(|space| {
            match w
                .module_state::<mush_sim::SpatialState>()?
                .spaces
                .get(&space)
                .map(|s| &s.anchor)
            {
                Some(mush_sim::SpaceAnchor::MovingObject { host }) => Some(*host),
                _ => None,
            }
        });
        if let Some(host) = host {
            return Standing::Unit(host);
        }
        match w.get(here).and_then(|o| o.location) {
            Some(up) => here = up,
            None => break,
        }
    }
    Standing::Ashore
}

/// Whether `unit` stands in a carrier's bay: an interior of a ship's own rooms.
fn in_a_bay(w: &World, unit: ObjId) -> bool {
    let Some(state) = w.module_state::<mush_sim::SpatialState>() else {
        return false;
    };
    match state.parent_of(unit) {
        mush_sim::SpatialParent::Interior { space, .. } => {
            crate::port::ship::hull_of_room(w, space).is_some()
        }
        _ => false,
    }
}

/// Whether `unit`'s last flight is still open (departing, in a field, or returning). A history
/// that cannot be read counts as open.
fn flight_open(ctx: &Ctx<'_>, unit: ObjId) -> bool {
    let Some(h) = ctx
        .w
        .module_state::<crate::detachment::sortie::SortieHandles>()
    else {
        return false;
    };
    let head = match ctx.action_state(&h.heads, &unit) {
        Ok(Some(head)) => head,
        Ok(None) => return false,
        Err(_) => return true,
    };
    match ctx.action_state(&h.flights, &head.flight) {
        Ok(Some(flight)) => flight.phase.open(),
        Ok(None) | Err(_) => true,
    }
}

/// **Why `npc` cannot be discharged where they stand**, if they cannot: in custody, in an open
/// field, or out on a sortie.
fn held_back(ctx: &Ctx<'_>, npc: ObjId, at: Standing) -> Option<&'static str> {
    let w: &World = ctx.w;
    if crate::field::crew_of(w, npc).is_some_and(|record| record.hold().is_some()) {
        return Some(CAPTIVE);
    }
    match at {
        Standing::Ashore => None,
        Standing::Ship(hull) => crate::field::field_of_unit(w, hull).map(|_| IN_FIELD),
        Standing::Unit(unit) => {
            if crate::field::field_of_unit(w, unit).is_some() {
                Some(IN_FIELD)
            } else if !in_a_bay(w, unit) || flight_open(ctx, unit) {
                Some(ON_SORTIE)
            } else {
                None
            }
        }
    }
}

/// Vacate every station `who` holds across `host` and any other host recorded in `SpatialState`.
fn vacate_all(ctx: &mut Ctx<'_>, who: ObjId, primary_host: Option<ObjId>) {
    let mut hosts: std::collections::BTreeSet<ObjId> = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.derived.assignments_by_actor.get(&who))
        .into_iter()
        .flatten()
        .copied()
        .collect();
    if let Some(host) = primary_host {
        hosts.insert(host);
    }
    for host in hosts {
        let held: Vec<mush_sim::StationId> = ctx
            .w
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&host))
            .map(|seats| {
                seats
                    .seats
                    .iter()
                    .filter(|(_, seat)| seat.actor == who)
                    .map(|(station, _)| station.clone())
                    .collect()
            })
            .unwrap_or_default();
        for station in held {
            if let Err(error) = mush_sim::vacate_station(ctx, host, &station, who) {
                tracing::warn!(
                    npc = who.0,
                    unit = host.0,
                    ?error,
                    "tacsim: a discharged hire's seat did not vacate"
                );
            }
        }
    }
}

/// Terminate a standing crew contract whose recurring wage reached delinquency (or whose employer
/// lost their active credit account before any wage opened), returning `true` when the world half
/// landed and `false` when deferred (e.g. NPC is currently on a sortie, in an open field, or held).
pub(super) fn terminate_delinquent(ctx: &mut Ctx<'_>, contract: &super::CrewContract) -> bool {
    let npc = contract.npc;
    let live_npc = ctx
        .w
        .get(npc)
        .is_some_and(|o| o.kind == ObjKind::Npc && !o.flags.contains(Flags::GOING));
    if !live_npc || crate::field::crew::exit_applies(ctx.w, contract) {
        return false;
    }
    let at = standing(ctx.w, npc);
    if held_back(ctx, npc, at).is_some() {
        return false;
    }
    let Some(room) = crate::field::crew::pool_room(ctx.w, npc, contract.terms.port.as_str()) else {
        return false;
    };
    let primary_host = match at {
        Standing::Ship(host) | Standing::Unit(host) => Some(host),
        Standing::Ashore => None,
    };
    vacate_all(ctx, npc, primary_host);
    send_back(ctx, npc, room);
    let now = ctx.w.world_ic_now().at;
    crate::field::crew::owe_npc_exit(
        ctx,
        crate::field::NpcExit {
            format: crate::field::crew::NPC_EXIT_FORMAT,
            npc,
            organizer: contract.employer,
            recovered_ic: now,
        },
    );
    let npc_name = crate::port::coop::name_of(ctx.w, npc);
    let room_name = crate::port::coop::name_of(ctx.w, room);
    ctx.out.sys_to_actor(
        contract.employer,
        "tacsim.detachment",
        format!(
            "{npc_name}'s crew contract ended for delinquent wages; they have returned to {room_name}."
        ),
    );
    mush_npc::notify_role_event(
        ctx,
        npc,
        "Your crew contract ended for delinquent wages; you are back at the hiring hall.",
    );
    true
}

/// `who` goes back to `room`, their pool room, as a captured hull's NPC does.
fn send_back(ctx: &mut Ctx<'_>, who: ObjId, room: ObjId) {
    let from = ctx.w.get(who).and_then(|object| object.location);
    if from == Some(room) {
        return;
    }
    if ctx.w.move_to(who, room).is_err() {
        tracing::warn!(
            npc = who.0,
            "tacsim: a discharged hire could not go back to the pool"
        );
        return;
    }
    let placed = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|state| state.is_placed(who));
    if placed && let Err(error) = mush_sim::unplace(ctx, who, "crew discharge") {
        tracing::warn!(npc = who.0, %error, "tacsim: a discharged hire's placement did not clear");
    }
    ctx.emit_world_event(&WorldEvent::Moved {
        who,
        from,
        to: room,
    });
}

/// **`+sortie/discharge #<npc>`** (this module's head): what the employer is told.
///
/// # Errors
///
/// The refusal's one sentence; nothing is changed.
pub(super) fn discharge(
    ctx: &mut Ctx<'_>,
    raw: &str,
    handles: &CrewHandles,
) -> Result<Vec<String>, String> {
    let actor = ctx.actor;
    let npc = named(raw).ok_or(DISCHARGE_USAGE)?;
    // The caller's own agreement first: nothing about the person is read for anyone else. A
    // hire whose exit is already owed has left.
    let contract = ctx
        .action_state(&handles.contracts, &npc)
        .map_err(|_| NOT_YOURS)?
        .filter(|c| c.employer == actor && c.npc == npc && c.validate().is_ok())
        .cloned()
        .ok_or(NOT_YOURS)?;
    let live = |id: ObjId, kind: ObjKind| {
        ctx.w
            .get(id)
            .is_some_and(|o| o.kind == kind && !o.flags.contains(Flags::GOING))
    };
    if !live(actor, ObjKind::Player)
        || !live(npc, ObjKind::Npc)
        || crate::field::crew::exit_applies(ctx.w, &contract)
    {
        return Err(NOT_YOURS.into());
    }
    if super::wages::port_here(ctx.w, actor).is_none() {
        return Err(AT_PORT.into());
    }
    let name = crate::port::coop::name_of(ctx.w, npc);
    let at = standing(ctx.w, npc);
    if let Some(why) = held_back(ctx, npc, at) {
        return Err(why.replace("{name}", &name));
    }
    if ctx.authoritative_pending(actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err(BUSY.into());
    }
    let room =
        crate::field::crew::pool_room(ctx.w, npc, contract.terms.port.as_str()).ok_or(NOT_NOW)?;
    // The world half. The settlement is its one step that may refuse, so it goes first.
    let wage = super::wages::settle_final(ctx, &contract).map_err(|error| {
        tracing::warn!(npc = npc.0, %error, "tacsim: a discharge's wage was not settled");
        NOT_NOW.to_string()
    })?;
    let primary_host = match at {
        Standing::Ship(host) | Standing::Unit(host) => Some(host),
        Standing::Ashore => None,
    };
    vacate_all(ctx, npc, primary_host);
    send_back(ctx, npc, room);
    let now = ctx.w.world_ic_now().at;
    crate::field::crew::owe_npc_exit(
        ctx,
        crate::field::NpcExit {
            format: crate::field::crew::NPC_EXIT_FORMAT,
            npc,
            organizer: actor,
            recovered_ic: now,
        },
    );
    super::runtime::submit_discharged(ctx, npc, now, contract.hired_at.at);
    mush_npc::notify_role_event(ctx, npc, NPC_TOLD);
    let settled = match wage {
        None => DISCHARGED_FREE.replace("{name}", &name),
        Some(wage) if wage.owing == 0 => DISCHARGED_PAID
            .replace("{name}", &name)
            .replace("{paid}", &super::wages::money_minor(ctx.w, wage.paid))
            .replace("{payee}", &wage.payee),
        Some(wage) if wage.paid == 0 => DISCHARGED_OWING
            .replace("{name}", &name)
            .replace("{owing}", &super::wages::money_minor(ctx.w, wage.owing))
            .replace("{payee}", &wage.payee),
        Some(wage) => DISCHARGED_PART
            .replace("{name}", &name)
            .replace("{paid}", &super::wages::money_minor(ctx.w, wage.paid))
            .replace("{owing}", &super::wages::money_minor(ctx.w, wage.owing)),
    };
    let returned = RETURNED
        .replace("{name}", &name)
        .replace("{room}", &crate::port::coop::name_of(ctx.w, room));
    Ok(vec![settled, returned])
}

/// **`+sortie/contract`'s line for a hire that has ended** (doc 48 cut EX.7b): what of its wage
/// still stands as the asker's own debt, if anything does.
pub(super) fn standing_debt(w: &World, employer: ObjId, npc: ObjId) -> Option<String> {
    let owing = super::wages::arrears_of(w, employer, npc);
    (owing > 0).then(|| {
        STANDING_DEBT
            .replace("{npc}", &npc.0.to_string())
            .replace("{owing}", &super::wages::money_minor(w, owing))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ex7b_a_discharge_names_one_npc_by_number() {
        assert_eq!(named("#12"), Some(ObjId(12)));
        assert_eq!(named(" 12 "), Some(ObjId(12)));
        for bad in ["", "#", "#0", "0", "Mendes", "#12=x", "#-3", "12 13"] {
            assert_eq!(named(bad), None, "{bad:?}");
        }
    }

    /// Every sentence at its widest fill stays within 120 columns.
    #[test]
    fn ex7b_the_sentences_fit_120_columns() {
        let name = "Sana Okonkwo-Castellanos";
        let payee = "Freeport Registry of Ceres";
        let filled = [
            DISCHARGE_USAGE.to_string(),
            NOT_YOURS.to_string(),
            AT_PORT.to_string(),
            ON_SORTIE.replace("{name}", name),
            IN_FIELD.replace("{name}", name),
            CAPTIVE.replace("{name}", name),
            BUSY.to_string(),
            NOT_NOW.to_string(),
            DISCHARGED_PAID
                .replace("{name}", name)
                .replace("{paid}", "5,620.00cr")
                .replace("{payee}", payee),
            DISCHARGED_OWING
                .replace("{name}", name)
                .replace("{owing}", "5,620.00cr")
                .replace("{payee}", payee),
            DISCHARGED_PART
                .replace("{name}", name)
                .replace("{paid}", "5,620.00cr")
                .replace("{owing}", "5,620.00cr"),
            DISCHARGED_FREE.replace("{name}", name),
            RETURNED
                .replace("{name}", name)
                .replace("{room}", "The Registry's Hiring Hall, C10"),
            STANDING_DEBT
                .replace("{npc}", "4294967295")
                .replace("{owing}", "5,620.00cr"),
        ];
        for line in filled {
            assert!(line.chars().count() <= 120, "{} wide: {line}", line.len());
        }
    }
}
