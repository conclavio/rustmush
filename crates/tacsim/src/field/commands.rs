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
//! - `+field/void` after a field (doc 36 cut GC.10, Q-36-11) — the leader cancels the standing
//!   their party's fire on a friendly party cost ([`super::standing`]).
//!
//! **How a presence row is named** (CF.1 left no stable token, and the picture must not hand
//! one out): by what the viewer reads — its octant and, when two flights share it, its band — and
//! the flight meant is the nearest one that matches, seen from the nearest of the viewer's own
//! units. Nothing about a row's identity is ever printed back.
//!
//! **Who leads** (doc 32 §4a): the roster's leader (doc 26), the pilot of a one-unit party —
//! read now, from the operation's command roster, so a succession moves the mark with it (doc
//! 36a §11 row 6, `parties::led_now`). **Who fires** (Q-CF-14; doc 36a §11 row 7): the pilot
//! seated in a unit on the ground, leader or not; the fire door is the flown unit's, the mark
//! door the leader's. Doctrine never opens a field by itself; CF.6 hands a delivered `ENGAGE`
//! received on the surface to [`mark_from_order`] as its leader's mark.

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
pub const SWITCHES: [&str; 3] = ["mark", "fire", "void"];

/// The verb's help.
pub const HELP: &str = "+field shows the field your party stands in. On a site's surface, \
+field/mark <bearing> [near|far|close|distant]=hostile marks the party of the flight you see there \
Hostile, and +field/fire <bearing> [band] orders the unit you fly to fire at it: either opens a \
field on the site. Inside a field, +field/mark <letter>=hostile|friendly|clear sets your party's \
own mark on another party. Only a party's leader marks; marks are private. After a field, +field/void cancels the standing your party's fire on a friendly party cost, until your next command or ten IC minutes.";

/// `Usage`.
const USAGE: &str = "Usage: +field | +field/mark <bearing> [<band>]=hostile | \
+field/mark <party letter>=hostile|friendly|clear | +field/fire <bearing> [<band>] | +field/void";

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

/// **The unit `actor` flies on a site's ground** (doc 36a §11 row 7; Q-CF-14: "a unit firing"
/// opens a field, whoever leads its party), with the site, its entrants and its units: the fire
/// door's own. The first lane site where a unit on the ground seats the actor.
fn flown(
    ctx: &Ctx<'_>,
    actor: ObjId,
) -> Option<(CanonicalId, Vec<parties::Entrant>, Vec<ObjId>, ObjId)> {
    for site in crate::surface::chart::lane_sites(ctx.w) {
        let Some(domain) = parties::site_domain(ctx.w, &site.entry) else {
            continue;
        };
        let units = parties::on_ground(ctx.w, domain);
        let Some(unit) = units
            .iter()
            .copied()
            .find(|unit| parties::seated(ctx.w, *unit).is_some_and(|(pilot, _)| pilot == actor))
        else {
            continue;
        };
        let entrants = parties::entrants(ctx, &site.entry, &units);
        return Some((site.entry, entrants, units, unit));
    }
    None
}

/// A unit's cell on the site grid.
fn cell_of(ctx: &Ctx<'_>, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(ctx.w, unit)
        .map(|(_, pose)| crate::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
}

/// **The flight a presence row names** (this module's head): `bearing` and an optional `band`
/// from the nearest of `own`, the nearest match, ties to the lower object number (never shown).
pub(crate) fn resolve_row(
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
pub(crate) fn octant(word: &str) -> Option<Octant> {
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
pub(crate) fn band(word: &str) -> Option<Band> {
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
    // Doc 42 cut CR.0 (Q-42-2): `/mark`, `/fire` and `/void` are the held gate's; the read passes.
    if ctx.switches.iter().next().is_some() {
        crate::field::crew::gate(ctx, crate::field::crew::Gate::Operate)?;
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("mark") {
        return mark(ctx, &raw);
    }
    if ctx.switches.has("fire") {
        return fire(ctx, &raw);
    }
    // Doc 36 cut GC.10 (Q-36-11): the leader's void of friendly fire's standing.
    if ctx.switches.has("void") {
        if !raw.is_empty() {
            return Err(CmdError::msg(USAGE));
        }
        super::standing::void(ctx);
        return Ok(());
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
                    let party = parties::led_now(ctx, &hosted.live, actor).or_else(|| {
                        hosted
                            .live
                            .units
                            .iter()
                            .find(|row| row.pilot.as_ref().is_some_and(|(p, _)| *p == actor))
                            .map(|row| row.party)
                    })?;
                    Some(
                        super::FIELD_STATUS
                            .replace("{n}", &hosted.live.field.ordinal.to_string())
                            .replace("{site}", &crate::surface::act::site_name(ctx.w, site))
                            .replace("{party}", &party.letter().to_string())
                            .replace("{parties}", &hosted.live.parties.len().to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if lines.is_empty() {
        ctx.out.info(ctx.actor, super::NO_FIELD.to_owned());
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
    // Inside a field: a party letter. On open ground a lone `N`, `E`, `S` or `W` is an octant
    // (doc 32 cut CF.6: the two readings of one letter split on where the leader stands).
    if target.len() == 1 && in_field(ctx, actor).is_some() {
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
    if in_field(ctx, actor).is_some() || seated_in_field(ctx, actor) {
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

/// The running field the actor leads a party in now (doc 36a §11 row 6: a roster's current
/// leader, `parties::led_now`), as its site and party.
fn in_field(ctx: &Ctx<'_>, actor: ObjId) -> Option<(CanonicalId, PartyId)> {
    let book = ctx.w.module_state::<super::FieldBook>()?;
    book.running().find_map(|(site, hosted)| {
        parties::led_now(ctx, &hosted.live, actor).map(|party| (site.clone(), party))
    })
}

/// Whether a running field holds a unit `actor` sits in (doc 36a §11 row 7): a seated
/// subordinate is told to fire from the cockpit, as a leader is, never that they lead nothing.
fn seated_in_field(ctx: &Ctx<'_>, actor: ObjId) -> bool {
    ctx.w
        .module_state::<super::FieldBook>()
        .is_some_and(|book| {
            book.running().any(|(_, hosted)| {
                hosted.live.units.iter().any(|row| {
                    row.pilot.as_ref().is_some_and(|(pilot, _)| *pilot == actor)
                        || parties::seated(ctx.w, row.unit).is_some_and(|(pilot, _)| pilot == actor)
                })
            })
        })
}

/// **A mark inside a field** (Q-CF-7): private, the marking party's book only.
///
/// **The owner's ruling of 2026-11-04** (doc 40's FR.5 cut note): the mark also goes to the books of
/// the unit parties whose hulls the leader seats in that field ([`seated_parties`]), so one mark
/// covers every hull they command. A hull that is its own party (a seated pilot with no roster)
/// was fired on in its own book and would otherwise keep the tempo Engaged after its leader
/// stood down.
fn mark_in_field(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    of: PartyId,
    mark: Option<Mark>,
) -> Result<(), CmdError> {
    let (site, by) =
        in_field(ctx, actor).ok_or_else(|| CmdError::msg("You lead no party in a field."))?;
    // Doc 40 cut FR.1 (§4.2, round 2's 40-3): a standing Hostile mark (a declared stance, a
    // raider's) outlives the clear, in each marking book by its own row.
    let (task, books) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(&site))
        .filter(|hosted| hosted.live.parties.iter().any(|party| party.party == of))
        .map(|hosted| {
            let standing = |book: PartyId| {
                hosted
                    .live
                    .parties
                    .iter()
                    .find(|party| party.party == book)
                    .is_some_and(|party| party.stance_hostile.contains(&of))
            };
            let books: Vec<(PartyId, bool)> = std::iter::once(by)
                .chain(seated_parties(ctx, &hosted.live, actor))
                .filter(|book| *book != of)
                .collect::<BTreeSet<PartyId>>()
                .into_iter()
                .map(|book| (book, standing(book)))
                .collect();
            (hosted.task, books)
        })
        .ok_or_else(|| CmdError::msg("No party in your field has that letter."))?;
    if of == by {
        return Err(CmdError::msg("That is your own party."));
    }
    let tasks = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or_else(|| CmdError::msg("The field host is unavailable."))?;
    let mut cleared_to_standing = false;
    let asked = mark;
    let mut mark = mark;
    for (book, standing) in books {
        let to_standing = asked.is_none() && standing;
        let marked = if to_standing {
            Some(Mark::Hostile)
        } else {
            asked
        };
        if book == by {
            cleared_to_standing = to_standing;
            mark = marked;
        }
        tasks
            .offer(
                task,
                crate::arena_task::ArenaMsg::Mark {
                    by: book,
                    of,
                    mark: marked,
                },
            )
            .map_err(|_| CmdError::msg("The field is busy; try the mark again."))?;
    }
    let word = match mark {
        Some(Mark::Hostile) => "Hostile",
        Some(Mark::Friendly) => "Friendly",
        None => "unmarked",
    };
    let line = if cleared_to_standing {
        super::PARTY_CLEARED_TO_STANDING
    } else {
        super::PARTY_MARKED
    };
    ctx.out.sys_to_actor(
        actor,
        "tacsim.field",
        line.replace("{party}", &of.letter().to_string())
            .replace("{word}", word),
    );
    Ok(())
}

/// **The unit parties whose hulls `actor` seats in a field** (the owner's ruling of 2026-11-04):
/// each row of the live record whose pilot is `actor`, by the record's own pilot or the seat as it
/// stands now, as its party.
fn seated_parties(ctx: &Ctx<'_>, live: &super::record::FieldLive, actor: ObjId) -> Vec<PartyId> {
    live.units
        .iter()
        .filter(|row| {
            row.pilot.as_ref().is_some_and(|(pilot, _)| *pilot == actor)
                || parties::seated(ctx.w, row.unit).is_some_and(|(pilot, _)| pilot == actor)
        })
        .map(|row| row.party)
        .collect()
}

/// **A door onto the surface**: resolve the row, refuse what cannot open, and submit the
/// promotion. The mark door is the leader's ([`led`]); the fire door is the flown unit's
/// ([`flown`]; doc 36a §11 row 7): the shooter's party is its own, whoever leads it.
fn open_by(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    door: &str,
    bearing: Octant,
    band: Option<Band>,
) -> Result<(), CmdError> {
    let (site, entrants, units, own_keys, own_for_door): (
        CanonicalId,
        Vec<parties::Entrant>,
        Vec<ObjId>,
        BTreeSet<PartyKey>,
        Vec<ObjId>,
    ) = if door == "fire" {
        // A fire order is the flown unit's: the actor must sit in a unit on the ground.
        let (site, entrants, units, unit) = flown(ctx, actor)
            .ok_or_else(|| CmdError::msg("You are not flying a unit on a site's ground."))?;
        let key = entrants
            .iter()
            .find(|entrant| entrant.unit == unit)
            .map(|entrant| entrant.key)
            .ok_or_else(|| CmdError::msg("You are not flying a unit on a site's ground."))?;
        (site, entrants, units, BTreeSet::from([key]), vec![unit])
    } else {
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
        if own.is_empty() {
            return Err(CmdError::msg(
                "You are not flying a unit on that site's ground.",
            ));
        }
        (site, entrants, units, own_keys, own)
    };
    let own: Vec<ObjId> = entrants
        .iter()
        .filter(|entrant| own_keys.contains(&entrant.key))
        .map(|entrant| entrant.unit)
        .collect();
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
        return Err(CmdError::msg(
            crate::surface::site_record::promoted_refusal(
                &field,
                &crate::surface::act::site_name(ctx.w, site),
            ),
        ));
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
    ctx.out.sys_to_actor(actor, "tacsim.field", said.to_owned());
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
