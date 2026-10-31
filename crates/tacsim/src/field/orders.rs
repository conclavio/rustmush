//! **Hired NPC units in a field fly the doctrine under their delivered orders** (doc 32 cut CF.6;
//! Q-CF-9, Q-CF-41, Q-CF-42, §4a).
//!
//! The roster's orders travel as they always do: issued by the leader, carried by the IC
//! transport (`mush_propagation`, jamming and all), delivered into the recipient's order book
//! (`crate::detachment::sortie::field_orders`). What changes in a field is who executes them.
//! The surface's NPC executor (`crate::detachment::sortie::npc`) hands a delivered order whose
//! unit stands in a field to [`step`], which does three things and nothing else:
//!
//! 1. **The fence and the order reach the kernel** through the DT.1b adapter
//!    (`crate::arena::detachment`): a [`DetachmentControl`] naming the roster's assignment and
//!    term, then a [`DetachmentCommand`] with the order's objective, its `HOLD FIRE`, its
//!    deadline in arena time and its issuer's unit. Both are applied at the head of the next
//!    pulse, so a `HOLD FIRE` binds the doctrine's next shot (`Arena::holds_fire`). No second AI:
//!    the hull is flown by its sealed mind exactly as in the simulator.
//! 2. **The kernel resolves the target in the recipient's own observation.** An `ENGAGE` naming
//!    a contact carries the row key from the leader's own cockpit scope; the adapter re-keys it
//!    into the recipient's classified rows or finds nothing (doc 16 §3.6's observer rule). The
//!    doctrine targets Hostile parties only (CF.0's books, Q-CF-9).
//! 3. **The reply travels back on the same transport.** Once the recipient's own frame shows the
//!    order standing, the NPC answers WILCO; when the order named a contact its frame does not
//!    hold, NEGATIVE (a lost contact is uncertainty, never a guessed lock). Orders a field cannot
//!    execute — a mining run, a campaign contact, a presence row, a contact from another field,
//!    support for a unit not in this field — are NEGATIVE at once and never reach the kernel.
//!
//! **The NPC keeps its seat.** At admission a hired NPC's hull is the mind's
//! (`Controller::Mind`) while the NPC stays bound to its station, so the arena builds the NPC's
//! own frame — what it sees — and the host reads its answer off that, never off the kernel's
//! objective truth.
//!
//! **Open ground** (§4a): doctrine runs only inside a field, so a delivered `ENGAGE` received on
//! the unpromoted surface naming a presence row is its leader's Hostile mark on that flight's
//! party ([`surface_mark`] → [`super::mark_from_order`]), resolved from the recipient's own unit.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, ObjKind, World};
use mush_tactics::detachment::{DeliveredOrder, DetachmentRoster, Directive, Response};
use smol_str::SmolStr;

use crate::arena::detachment::{
    DetachmentAuthority, DetachmentCommand, DetachmentControl, DetachmentObjective,
};
use crate::arena::perception::MindObjective;
use crate::arena::state::ContactId;
use crate::arena_bridge::ArenaRegistry;
use crate::arena_task::MatchId;
use crate::deliberation::order::{Constraints, Objective};
use crate::detachment::sortie::{EngageAt, FieldObjective, FieldPayload};

/// Whether `actor` is a non-player character (a hired NPC, a raider's leader).
#[must_use]
pub(crate) fn is_npc(w: &World, actor: ObjId) -> bool {
    w.get(actor)
        .is_some_and(|object| object.kind == ObjKind::Npc)
}

/// The ordinal of the field running on `site`.
fn ordinal(w: &World, site: &CanonicalId) -> Option<u32> {
    w.module_state::<super::FieldBook>()?
        .hosted(site)
        .map(|hosted| hosted.live.field.ordinal)
}

/// **A contact of the leader's own cockpit, by the designator they typed** (doc 32 Q-CF-11):
/// when `unit`, the leader's own, stands in a running field and the leader's last frame from it
/// holds a row with that designator.
#[must_use]
pub(crate) fn leader_contact(
    w: &World,
    leader: ObjId,
    unit: ObjId,
    word: &str,
) -> Option<EngageAt> {
    let site = super::field_of_unit(w, unit)?;
    let ordinal = ordinal(w, &site)?;
    let cockpit = w.module_state::<ArenaRegistry>()?.last_frame(leader)?;
    if cockpit.frame.unit != unit {
        return None;
    }
    let contact = cockpit
        .frame
        .contacts
        .iter()
        .find(|contact| contact.designator.to_string().eq_ignore_ascii_case(word))?;
    Some(EngageAt::Contact {
        ordinal,
        id: contact.id.0,
        designator: SmolStr::new(contact.designator.to_string()),
    })
}

/// **A presence row in one word** — `NE`, or `NE-near` with a band (Q-CF-15).
#[must_use]
pub(crate) fn row_of_words(word: &str) -> Option<EngageAt> {
    let (bearing, band) = match word.split_once('-') {
        Some((bearing, band)) => (bearing, Some(band)),
        None => (word, None),
    };
    let octant = super::commands::octant(bearing)?;
    let band = match band {
        Some(word) => Some(super::commands::band(word)?),
        None => None,
    };
    Some(EngageAt::Row {
        bearing: SmolStr::new(octant.abbrev()),
        band: band.map(|band| SmolStr::new(band.word())),
    })
}

/// **The flight a row names, seen from `unit`** on `site`'s open ground: the nearest flight of
/// another party that matches, as `(unit, target)`.
fn surface_target(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    unit: ObjId,
    at: &EngageAt,
) -> Option<(ObjId, ObjId)> {
    let EngageAt::Row { bearing, band } = at else {
        return None;
    };
    let octant = super::commands::octant(bearing)?;
    let band = match band {
        Some(word) => Some(super::commands::band(word)?),
        None => None,
    };
    let domain = super::parties::site_domain(ctx.w, site)?;
    let units = super::parties::on_ground(ctx.w, domain);
    let entrants = super::parties::entrants(ctx, site, &units);
    let own = entrants.iter().find(|entrant| entrant.unit == unit)?.key;
    let others: Vec<ObjId> = entrants
        .iter()
        .filter(|entrant| entrant.key != own)
        .map(|entrant| entrant.unit)
        .collect();
    super::commands::resolve_row(ctx, &[unit], &others, octant, band)
}

/// **A delivered `ENGAGE` received on open ground** (§4a): the leader's Hostile mark on the party
/// of the flight its row names, seen from the recipient's own unit. `Ok(false)` when the order
/// names no row (nothing the surface can act on); `Err` when the row finds no flight or the mark
/// is refused — a NEGATIVE either way.
///
/// # Errors
///
/// Why no mark was made, as a sentence.
pub(crate) fn surface_mark(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    unit: ObjId,
    leader: ObjId,
    objective: &FieldObjective,
) -> Result<bool, String> {
    let FieldObjective::Engage {
        contact: None,
        at: Some(at @ EngageAt::Row { .. }),
    } = objective
    else {
        return Ok(false);
    };
    let (from, target) = surface_target(ctx, site, unit, at)
        .ok_or_else(|| "The named flight is not in sight.".to_owned())?;
    super::mark_from_order(ctx, leader, site, (from, target)).map_err(|error| error.to_string())?;
    Ok(true)
}

/// **What [`step`] did** for one unit's delivered order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing to do.
    Idle,
    /// Waiting: the order is offered and the recipient's frame does not yet show it.
    Waiting,
    /// The NPC owes this reply on the transport.
    Reply(Response),
}

/// The orders offered to field tasks (RAM, module state): unit → (task, fence epoch, sequence)
/// and the world second it was offered. A restart forgets it and the next visit offers again;
/// the kernel's fence refuses the duplicate harmlessly.
#[derive(Clone, Debug, Default)]
pub(crate) struct Offered(BTreeMap<ObjId, ((MatchId, u64, u64), i64)>);

/// **How long an offered order may take to stand**, world seconds. The kernel refuses an order
/// it cannot take without a word — its issuer's unit wrecked or gone from the field, the fence
/// moved, the deadline past — so an order the recipient's own frame does not show standing
/// within this window is answered NEGATIVE rather than left unanswered.
pub const ORDER_TAKE_WINDOW_S: i64 = 10;

/// **The fence epoch of a roster assignment**: increasing with the roster's revision, then its
/// command term, so a succession raises it and a duplicate repeats it.
fn epoch(roster: &DetachmentRoster) -> u64 {
    roster
        .revision()
        .saturating_mul(1 << 24)
        .saturating_add(roster.term())
        .max(1)
}

/// The admitted order's identity as the kernel writes it on the hull's order row.
fn identity(roster: &DetachmentRoster, sequence: u64) -> String {
    format!(
        "dt:{:x}:{:x}:{:x}",
        roster.id().0,
        roster.revision(),
        sequence
    )
}

/// **The order as the kernel's objective**, or `Err` for one a field cannot execute.
fn objective(
    payload: &FieldPayload,
    roster: &DetachmentRoster,
    live: &super::record::FieldLive,
    unit: ObjId,
) -> Result<Objective, ()> {
    Ok(match &payload.objective {
        FieldObjective::Hold => Objective::Hold,
        FieldObjective::Advance { to } => Objective::Advance { to: *to },
        FieldObjective::Withdraw => Objective::Withdraw,
        FieldObjective::Screen { bearing_mdeg } => Objective::Screen {
            facing_mdeg: *bearing_mdeg,
        },
        FieldObjective::Support { slot } => {
            let of = roster.member(*slot).map(|member| member.unit).ok_or(())?;
            if of == unit || live.party_of(of).is_none() || live.party_of(of) != live.party_of(unit)
            {
                return Err(());
            }
            Objective::Support { of }
        }
        FieldObjective::Engage {
            contact: None,
            at: None,
        } => Objective::Engage { target_hint: None },
        FieldObjective::Engage {
            contact: None,
            at: Some(EngageAt::Contact { ordinal, id, .. }),
        } if *ordinal == live.field.ordinal => Objective::Engage {
            target_hint: Some(ContactId(*id)),
        },
        // A campaign contact, a presence row or another field's contact names nothing in this
        // field's observation; a mining run is not a field's work.
        FieldObjective::Engage { .. } | FieldObjective::Mine { .. } => return Err(()),
    })
}

/// **The arena deadline of an IC deadline**: the IC milliseconds left, at the world's rate, after
/// the field's own clock now. `None` for a standing order (or a frozen clock).
fn arena_deadline(w: &World, task: MatchId, expires_ic_ms: Option<u64>) -> Option<i64> {
    let end = expires_ic_ms?;
    let now_ic_ms = u64::try_from(w.world_ic_now().at)
        .ok()?
        .saturating_mul(1_000);
    let rate = u64::from(w.world_timeline().current().rate).max(1);
    let left_ms = end.saturating_sub(now_ic_ms) / rate;
    let now_ms = w.module_state::<ArenaRegistry>()?.mirror(task)?.now_ms;
    Some(now_ms.saturating_add(i64::try_from(left_ms).unwrap_or(i64::MAX)))
}

/// **One NPC unit's delivered order, in the field on `site`** (this module's head).
pub(crate) fn step(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    (unit, operator): (ObjId, ObjId),
    roster: &DetachmentRoster,
    delivered: &DeliveredOrder<FieldPayload>,
) -> Step {
    let Some((task, live)) = ctx
        .w
        .module_state::<super::FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| (hosted.task, hosted.live.clone()))
    else {
        return Step::Idle;
    };
    let Some(party) = live.party_of(unit) else {
        return Step::Idle;
    };
    let order = &delivered.order;
    let unanswered = delivered.acknowledgement.is_none();
    let directive = match &order.directive {
        Directive::Cancel => None,
        Directive::Objective(payload) => match objective(payload, roster, &live, unit) {
            Ok(objective) => Some(DetachmentObjective {
                objective,
                constraints: Constraints {
                    hold_fire: payload.hold_fire,
                    ..Constraints::default()
                },
            }),
            Err(()) if unanswered => return Step::Reply(Response::Negative),
            Err(()) => return Step::Idle,
        },
    };
    let fence = epoch(roster);
    let offered = ctx
        .w
        .module_state::<Offered>()
        .and_then(|offered| offered.0.get(&unit).copied());
    let key = (task, fence, order.sequence);
    if offered.is_none_or(|(at, _)| at != key) {
        let authority = DetachmentAuthority {
            detachment: roster.id().0,
            revision: roster.revision(),
            term: roster.term(),
        };
        let control = DetachmentControl {
            unit,
            epoch: fence,
            authority: Some(authority),
        };
        let command = DetachmentCommand {
            authority,
            authority_epoch: fence,
            sequence: order.sequence,
            issuer: order.issuer,
            issuer_unit: order.issuer_unit,
            squad: crate::deliberation::SquadId {
                team: party,
                index: 1,
            },
            directive,
            expires_at_ms: arena_deadline(ctx.w, task, order.expires_at_ms),
        };
        let now = ctx.w.now();
        if crate::arena_bridge::offer_field_detachment(ctx, task, control, command)
            && let Some(book) = ctx.w.module_state_mut::<Offered>()
        {
            book.0.insert(unit, (key, now));
        }
        return Step::Waiting;
    }
    if !unanswered {
        return Step::Idle;
    }
    let Some(directive) = directive else {
        // A cancellation stands once the kernel took it: nothing in the frame to wait for.
        return Step::Reply(Response::Wilco);
    };
    // The recipient's own frame says whether the order stands, and what it holds of the target.
    let standing = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.last_frame(operator))
        .filter(|cockpit| cockpit.frame.unit == unit)
        .and_then(|cockpit| cockpit.frame.order.clone())
        .filter(|row| row.identity.as_deref() == Some(identity(roster, order.sequence).as_str()));
    let Some(row) = standing else {
        let offered_at = offered.map_or(0, |(_, at)| at);
        return if ctx.w.now().saturating_sub(offered_at) >= ORDER_TAKE_WINDOW_S {
            Step::Reply(Response::Negative)
        } else {
            Step::Waiting
        };
    };
    let lost = matches!(
        directive.objective,
        Objective::Engage {
            target_hint: Some(_)
        }
    ) && matches!(row.objective, MindObjective::Engage { target: None });
    Step::Reply(if lost {
        Response::Negative
    } else {
        Response::Wilco
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_one_word_with_an_optional_band() {
        assert_eq!(
            row_of_words("ne"),
            Some(EngageAt::Row {
                bearing: SmolStr::new("NE"),
                band: None
            })
        );
        assert!(matches!(
            row_of_words("W-near"),
            Some(EngageAt::Row { band: Some(_), .. })
        ));
        assert_eq!(row_of_words("CB"), None);
        assert_eq!(row_of_words("NE-sideways"), None);
    }
}
