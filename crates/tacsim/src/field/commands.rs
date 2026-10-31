//! **`+field`** — the leader's mark and the surface fire order (doc 32 cut CF.2; Q-CF-7, Q-CF-14,
//! Q-CF-15, §4a).
//!
//! - `+field` — what the actor's own parties stand in: the field open on their site, their
//!   party's letter in it, the letters of the parties they mark Hostile or Friendly. Never anybody
//!   else's marks (Q-CF-7).
//! - `+field/mark <bearing> [<band>]=hostile` on the unpromoted surface — the leader's Hostile
//!   mark on the party of the flight seen there on the presence picture: it promotes the site.
//! - `+field/mark <letter>=hostile|friendly|clear` inside a field — the mark the leader's party
//!   holds on another party, private (it reaches only the kernel's book of the marking party).
//! - `+field/fire <bearing> [<band>]` on the unpromoted surface — the pilot's fire order at a
//!   presence row: it promotes the site, and the shot resolves in the field's first pulses under
//!   the normal lock rules (the host's [`super::FIRE_WINDOW_MS`]).
//!
//! **How a presence row is named** (CF.1 left no stable token, and the picture must not hand
//! one out): by what the viewer reads — its octant and, when two flights share it, its band — and
//! the flight meant is the nearest one that matches, seen from the nearest of the viewer's own
//! units. Nothing about a row's identity is ever printed back.
//!
//! **Who leads** (doc 32 §4a): the roster's leader (doc 26), the pilot of a one-unit party.
//! Doctrine never opens a field by itself; CF.6 hands a delivered `ENGAGE` received on the
//! surface to [`mark_from_order`] as its leader's mark.

use std::collections::BTreeSet;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, PerceivedRef};
use serde_json::json;

use super::parties;
use super::record::PartyKey;
use crate::arena::PartyId;
use crate::arena::party::Mark;
use crate::surface::address::Octant;
use crate::surface::excursion::cells_apart;
use crate::surface::presence::Band;

/// The verb's switches.
pub const SWITCHES: [&str; 2] = ["mark", "fire"];

/// The verb's help.
pub const HELP: &str = "+field shows the field your party stands in. On a site's surface, \
+field/mark <bearing> [near|far|close|distant]=hostile marks the party of the flight you see there \
Hostile, and +field/fire <bearing> [band] orders the unit you fly to fire at it: either opens a \
field on the site. Inside a field, +field/mark <letter>=hostile|friendly|clear sets your party's \
own mark on another party. Only a party's leader marks; marks are private.";

/// `Usage`.
const USAGE: &str = "Usage: +field | +field/mark <bearing> [<band>]=hostile | \
+field/mark <party letter>=hostile|friendly|clear | +field/fire <bearing> [<band>]";

/// **The units `actor` leads on one site's ground**, with the site, for every lane site where
/// there are any.
fn led(ctx: &Ctx<'_>, actor: ObjId) -> Vec<(CanonicalId, Vec<parties::Entrant>, Vec<ObjId>)> {
    let mut out = Vec::new();
    for site in crate::surface::chart::lane_sites(ctx.w) {
        let Some(domain) = parties::site_domain(ctx.w, &site.entry) else {
            continue;
        };
        let units = parties::on_ground(ctx.w, domain);
        let entrants = parties::entrants(ctx, &site.entry, &units);
        if entrants.iter().any(|entrant| entrant.leader == Some(actor)) {
            out.push((site.entry, entrants, units));
        }
    }
    out
}

/// A unit's cell on the site grid.
fn cell_of(ctx: &Ctx<'_>, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(ctx.w, unit)
        .map(|(_, pose)| crate::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
}

/// **The flight a presence row names** (this module's head): `bearing` and an optional `band`
/// from the nearest of `own`, the nearest match, ties to the lower object number (never shown).
fn resolve_row(
    ctx: &Ctx<'_>,
    own: &[ObjId],
    others: &[ObjId],
    bearing: Octant,
    band: Option<Band>,
) -> Option<(ObjId, ObjId)> {
    let own_cells: Vec<(ObjId, (i64, i64))> = own
        .iter()
        .filter_map(|unit| cell_of(ctx, *unit).map(|cell| (*unit, cell)))
        .collect();
    let mut best: Option<(u64, ObjId, ObjId)> = None;
    for other in others {
        let Some(at) = cell_of(ctx, *other) else {
            continue;
        };
        let Some((from, from_cell)) = own_cells
            .iter()
            .min_by_key(|(unit, cell)| (cells_apart(*cell, at), *unit))
        else {
            continue;
        };
        let cells = cells_apart(*from_cell, at);
        if Octant::of(at.0 - from_cell.0, at.1 - from_cell.1) != Some(bearing)
            || band.is_some_and(|band| Band::of(cells) != band)
        {
            continue;
        }
        if best.is_none_or(|(d, o, _)| (cells, *other) < (d, o)) {
            best = Some((cells, *other, *from));
        }
    }
    best.map(|(_, other, from)| (from, other))
}

/// `NE` / `n` / `sw` → the octant.
fn octant(word: &str) -> Option<Octant> {
    [
        Octant::N,
        Octant::NE,
        Octant::E,
        Octant::SE,
        Octant::S,
        Octant::SW,
        Octant::W,
        Octant::NW,
    ]
    .into_iter()
    .find(|octant| octant.abbrev().eq_ignore_ascii_case(word))
}

/// `close` / `near` / `far` / `distant` → the band.
fn band(word: &str) -> Option<Band> {
    [Band::Close, Band::Near, Band::Far, Band::Distant]
        .into_iter()
        .find(|band| band.word().eq_ignore_ascii_case(word))
}

/// **`+field`**.
///
/// # Errors
///
/// The refusal line.
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(crate::TacsimStore::enabled)
    {
        return Err(CmdError::msg("The tactical simulation is not enabled."));
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("mark") {
        return mark(ctx, &raw);
    }
    if ctx.switches.has("fire") {
        return fire(ctx, &raw);
    }
    if !raw.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    status(ctx)
}

/// `+field`: the field the actor's parties stand in.
fn status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let lines: Vec<String> = ctx
        .w
        .module_state::<super::FieldBook>()
        .map(|book| {
            book.running()
                .filter_map(|(site, hosted)| {
                    let party = hosted.live.led_by(actor).or_else(|| {
                        hosted
                            .live
                            .units
                            .iter()
                            .find(|row| row.pilot.as_ref().is_some_and(|(p, _)| *p == actor))
                            .map(|row| row.party)
                    })?;
                    Some(format!(
                        "Field {} is open on {}. Your party is {}; {} parties are on the ground.",
                        hosted.live.field.ordinal,
                        crate::surface::act::site_name(ctx.w, site),
                        char::from(party.letter()),
                        hosted.live.parties.len()
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if lines.is_empty() {
        ctx.out
            .info(ctx.actor, "No field holds a party of yours.".to_owned());
    }
    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.field", line);
    }
    Ok(())
}

/// `+field/mark …`.
fn mark(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let (target, word) = raw
        .split_once('=')
        .map(|(a, b)| (a.trim(), b.trim()))
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let mark = match word.to_ascii_lowercase().as_str() {
        "hostile" => Some(Mark::Hostile),
        "friendly" => Some(Mark::Friendly),
        "clear" => None,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let actor = ctx.actor;
    // Inside a field: a party letter.
    if target.len() == 1 {
        let letter = target.as_bytes().first().copied().unwrap_or(0);
        let party = PartyId::from_letter(char::from(letter.to_ascii_uppercase()))
            .ok_or_else(|| CmdError::msg(USAGE))?;
        return mark_in_field(ctx, actor, party, mark);
    }
    if mark != Some(Mark::Hostile) {
        return Err(CmdError::msg(
            "A Friendly mark or a clear lives in a field's books; on the open surface only a \
             Hostile mark means anything, and it opens a field.",
        ));
    }
    let (bearing, band) = row_words(target)?;
    open_by(ctx, actor, "mark", bearing, band)
}

/// `+field/fire …`.
fn fire(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if in_field(ctx, actor).is_some() {
        return Err(CmdError::msg(
            "You are in a field: fire from your cockpit (LOCK, then FIRE).",
        ));
    }
    let (bearing, band) = row_words(raw)?;
    open_by(ctx, actor, "fire", bearing, band)
}

/// `<bearing> [<band>]`.
fn row_words(text: &str) -> Result<(Octant, Option<Band>), CmdError> {
    let mut words = text.split_whitespace();
    let bearing = words
        .next()
        .and_then(octant)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let band = match words.next() {
        None => None,
        Some(word) => Some(band(word).ok_or_else(|| CmdError::msg(USAGE))?),
    };
    if words.next().is_some() {
        return Err(CmdError::msg(USAGE));
    }
    Ok((bearing, band))
}

/// The running field the actor leads a party in, as its site and party.
fn in_field(ctx: &Ctx<'_>, actor: ObjId) -> Option<(CanonicalId, PartyId)> {
    let book = ctx.w.module_state::<super::FieldBook>()?;
    book.running()
        .find_map(|(site, hosted)| hosted.live.led_by(actor).map(|party| (site.clone(), party)))
}

/// **A mark inside a field** (Q-CF-7): private, the marking party's book only.
fn mark_in_field(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    of: PartyId,
    mark: Option<Mark>,
) -> Result<(), CmdError> {
    let (site, by) = in_field(ctx, actor)
        .ok_or_else(|| CmdError::msg("You lead no party in a field."))?;
    let task = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(&site))
        .filter(|hosted| hosted.live.parties.iter().any(|party| party.party == of))
        .map(|hosted| hosted.task)
        .ok_or_else(|| CmdError::msg("No party in your field has that letter."))?;
    if of == by {
        return Err(CmdError::msg("That is your own party."));
    }
    let tasks = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or_else(|| CmdError::msg("The field host is unavailable."))?;
    tasks
        .offer(task, crate::arena_task::ArenaMsg::Mark { by, of, mark })
        .map_err(|_| CmdError::msg("The field is busy; try the mark again."))?;
    let word = match mark {
        Some(Mark::Hostile) => "Hostile",
        Some(Mark::Friendly) => "Friendly",
        None => "unmarked",
    };
    ctx.out.sys_to_actor(
        actor,
        "tacsim.field",
        format!(
            "Your party now marks party {} {word}.",
            char::from(of.letter())
        ),
    );
    Ok(())
}

/// **A door onto the surface**: resolve the row, refuse what cannot open, and submit the
/// promotion.
fn open_by(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    door: &str,
    bearing: Octant,
    band: Option<Band>,
) -> Result<(), CmdError> {
    let mut sites = led(ctx, actor);
    let (site, entrants, units) = match sites.len() {
        0 => {
            return Err(CmdError::msg(
                "You lead no party on a site's ground.".to_owned(),
            ));
        }
        1 => sites.remove(0),
        _ => {
            return Err(CmdError::msg(
                "You lead parties on more than one site's ground; recover all but one first.",
            ));
        }
    };
    let own_keys: BTreeSet<PartyKey> = entrants
        .iter()
        .filter(|entrant| entrant.leader == Some(actor))
        .map(|entrant| entrant.key)
        .collect();
    let own: Vec<ObjId> = entrants
        .iter()
        .filter(|entrant| own_keys.contains(&entrant.key))
        .map(|entrant| entrant.unit)
        .collect();
    // A fire order is the flown unit's: the actor must sit in one of them.
    let own_for_door: Vec<ObjId> = if door == "fire" {
        own.iter()
            .copied()
            .filter(|unit| parties::seated(ctx.w, *unit).is_some_and(|(p, _)| p == actor))
            .collect()
    } else {
        own.clone()
    };
    if own_for_door.is_empty() {
        return Err(CmdError::msg("You are not flying a unit on that site's ground."));
    }
    let others: Vec<ObjId> = units
        .iter()
        .copied()
        .filter(|unit| !own.contains(unit))
        .collect();
    let (from, at) = resolve_row(ctx, &own_for_door, &others, bearing, band)
        .ok_or_else(|| CmdError::msg("You see no flight there."))?;
    open_site(ctx, actor, &site, door, (from, at))
}

/// **Submit the promotion** (`tacsim_field open`) for `site` by `door` from `from` at `at`.
fn open_site(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    site: &CanonicalId,
    door: &str,
    (from, at): (ObjId, ObjId),
) -> Result<(), CmdError> {
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg("The site records are unavailable."))?;
    if let Some(field) = crate::surface::site_record::open_field(
        |key| {
            ctx.action_state(&handles.families.sites, key)
                .map(|found| found.cloned())
        },
        site,
    )
    .ok()
    .flatten()
    {
        return Err(CmdError::msg(crate::surface::site_record::promoted_refusal(
            &field,
            &crate::surface::act::site_name(ctx.w, site),
        )));
    }
    let action = handles
        .field
        .clone()
        .ok_or_else(|| CmdError::msg("The field host is unavailable."))?;
    // Every rig whose work is in flight on the site, for the promotion to settle.
    let rigs: Vec<String> = crate::surface::act::world_rigs(ctx.w)
        .into_iter()
        .filter(|rig| {
            ctx.action_state(
                &handles.families.excursions,
                &crate::surface::excursion::RigKey(*rig),
            )
            .ok()
            .flatten()
            .is_some_and(|record| {
                record.act.as_ref().is_some_and(|act| &act.site == site)
                    || record.run.as_ref().is_some_and(|run| &run.site == site)
            })
        })
        .map(|rig| rig.0.to_string())
        .collect();
    let now = ctx.w.world_ic_now().at;
    let ordinal = crate::field::close::site_record(ctx, site).map_or(0, |record| record.ordinal);
    let key = super::provider::field_key(actor, site, "open", ordinal.saturating_add(1), now)
        .ok_or_else(|| CmdError::msg("The field key is unavailable."))?;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "site": site.to_string(),
            "op": "open",
            "by": door,
            "from": from.0,
            "at": at.0,
            "rigs": rigs.join(","),
        }),
        key,
    );
    let said = if door == "fire" {
        "Fire order given. The ground goes tactical; the shot is taken as soon as your sensors hold the target."
    } else {
        "Marked Hostile. The ground goes tactical."
    };
    ctx.out
        .sys_to_actor(actor, "tacsim.field", said.to_owned());
    Ok(())
}

/// **CF.6's hook** (doc 32 §4a): a delivered `ENGAGE` received on the unpromoted surface is its
/// leader's Hostile mark on the party of `at`, seen from `from`. Answers whether a promotion was
/// submitted.
///
/// # Errors
///
/// The refusal line, as the verb would have said it.
pub fn mark_from_order(
    ctx: &mut Ctx<'_>,
    leader: ObjId,
    site: &CanonicalId,
    (from, at): (ObjId, ObjId),
) -> Result<(), CmdError> {
    let saved = ctx.actor;
    ctx.actor = leader;
    let result = open_site(ctx, leader, site, "mark", (from, at));
    ctx.actor = saved;
    result
}
