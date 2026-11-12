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

/// **The last frame `operator`'s seat was sent from the field `unit` stands in**: its own
/// observation, never the kernel's truth (doc 16 §3.6). `None` outside a running field, before
/// the first frame, and for a frame of another unit or another task.
fn own_frame(w: &World, unit: ObjId, operator: ObjId) -> Option<&crate::arena::step::Frame> {
    let site = super::field_of_unit(w, unit)?;
    let task = w.module_state::<super::FieldBook>()?.hosted(&site)?.task;
    let cockpit = w.module_state::<ArenaRegistry>()?.last_frame(operator)?;
    (cockpit.match_id == task && cockpit.frame.unit == unit).then_some(&cockpit.frame)
}

/// **What a commander's own frame holds** (doc 44 unit MW.14a): the live contacts its party's
/// book holds Hostile, as `operator`'s seat in `unit` last saw them. The compiled commander
/// decides on this and on nothing the kernel knows that its hull has not seen. `None` until its
/// frame has arrived.
#[must_use]
pub(crate) fn sight(
    w: &World,
    unit: ObjId,
    operator: ObjId,
) -> Option<crate::detachment::commander::Sight> {
    let frame = own_frame(w, unit, operator)?;
    let hostiles = frame
        .contacts
        .iter()
        .filter(|contact| {
            contact.relation == crate::arena::party::Relation::Hostile
                && contact.reads_dead != Some(true)
        })
        .count();
    Some(crate::detachment::commander::Sight {
        hostiles: u16::try_from(hostiles).unwrap_or(u16::MAX),
    })
}

/// **Whether `unit` is lost in the field it stands in** (doc 44 unit MW.14a): its operator's own
/// frame shows it a wreck, or the field's kernel has named it destroyed
/// ([`super::boarding::Destroyed`], which the ledger's chunks fill a little later). The world's
/// unit record is checked out and stale while the field runs, so the roster's command facts
/// read this beside it: a commander whose hull is lost stops leading at once, not at the
/// field's close. `false` outside a running field.
#[must_use]
pub(crate) fn lost(w: &World, unit: ObjId, operator: ObjId) -> bool {
    if own_frame(w, unit, operator).is_some_and(|frame| frame.wreck) {
        return true;
    }
    super::field_of_unit(w, unit)
        .and_then(|site| {
            let task = w.module_state::<super::FieldBook>()?.hosted(&site)?.task;
            w.module_state::<super::boarding::Destroyed>()?.0.get(&task)
        })
        .is_some_and(|units| units.contains(&unit))
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

/// **The delivered orders a damaged relay is holding back** (doc 40 cut FR.5, Q-40-5; RAM,
/// module state): recipient unit → (task, fence epoch, sequence) and the world second the order
/// was first seen under a damaged relay. A restart forgets it and the delay runs again from the
/// next visit, which is a later order, never an earlier one.
#[derive(Clone, Debug, Default)]
pub(crate) struct RelayLate(BTreeMap<ObjId, ((MatchId, u64, u64), i64)>);

/// **The relay word of the carrier of `party`** (doc 40 cut FR.5, Q-40-5): the first unit of the
/// party whose record carries a relay part (only the Halcyon does, `halcyon.json`), with its
/// part's condition.
///
/// **A seam, named**: the word is read off the carrier's **world** record — the condition it
/// brought into the field, so a relay an earlier field jammed holds the party's orders until the
/// bay clears it. A part the kernel jams or damages **inside** this field changes the arena's
/// copy of the record, which reaches the world at the carrier's check-in; until the host is told
/// the kernel's live word (a mirror of the carrier's part conditions, fed by the `hit` line's
/// part op), a mid-field relay hit takes effect from the next field. Swap the read here for that
/// mirror and nothing else moves.
fn carrier_relay(
    w: &mush_core::World,
    live: &super::record::FieldLive,
    party: crate::arena::state::PartyId,
) -> Option<(ObjId, mush_tactics::field_ops::ComponentCondition)> {
    live.units
        .iter()
        .filter(|row| row.party == party)
        .find_map(|row| {
            let record = mush_tactics::module::unit_of(w, row.unit)?;
            crate::arena::part_effects::relay_condition(record).map(|word| (row.unit, word))
        })
}

/// **The damaged-relay delay of `carrier`**, wall milliseconds: its unit's component effects
/// (its own table, the resolution pack's, else the compiled
/// [`crate::arena::resolution::COMPONENT_EFFECTS`]).
fn relay_delay_ms(w: &mush_core::World, carrier: ObjId) -> u32 {
    let effects = w
        .module_state::<crate::TacsimStore>()
        .zip(mush_tactics::module::unit_of(w, carrier))
        .map_or(
            crate::arena::resolution::COMPONENT_EFFECTS,
            |(store, record)| crate::match_::unit_profile(store.definitions(), record).effects(),
        );
    effects.relay.damaged_delay_ms
}

/// **Whether a delivered order may be offered to the kernel now** (doc 40 cut FR.5, Q-40-5): a
/// carrier relay that reads jammed holds every order of its party until the field ends or the
/// part is unjammed; one that reads damaged lets each order through its delay after it was
/// first seen here ([`RelayLate`]); otherwise, and for a party with no relay, at once.
fn relay_passes(
    ctx: &mut Ctx<'_>,
    live: &super::record::FieldLive,
    party: crate::arena::state::PartyId,
    unit: ObjId,
    key: (MatchId, u64, u64),
) -> bool {
    use mush_tactics::field_ops::ComponentCondition;
    let Some((carrier, word)) = carrier_relay(ctx.w, live, party) else {
        return true;
    };
    match word {
        ComponentCondition::Jammed => false,
        ComponentCondition::Damaged => {
            let delay_ms = i64::from(relay_delay_ms(ctx.w, carrier));
            let now = ctx.w.now();
            let Some(book) = ctx.w.module_state_mut::<RelayLate>() else {
                return true;
            };
            let since = match book.0.get(&unit) {
                Some((at, since)) if *at == key => *since,
                _ => {
                    book.0.insert(unit, (key, now));
                    now
                }
            };
            now.saturating_sub(since).saturating_mul(1_000) >= delay_ms
        }
        _ => true,
    }
}

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
        // Doc 40 cut FR.5 (Q-40-5): the party's carrier relay decides whether the order may
        // reach the kernel yet.
        if !relay_passes(ctx, &live, party, unit, key) {
            return Step::Waiting;
        }
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

    /// A roster of four NPC-flown hulls, units 101 to 104 in slots 1 to 4, and a field that
    /// holds 101 and 102 in party A and 103 in party B (104 is not in it).
    fn roster_and_field() -> (DetachmentRoster, super::super::record::FieldLive) {
        use mush_tactics::detachment::{Controller, DetachmentId, LeadershipPolicy, Member, Slot};
        let members = (1..=4)
            .map(|n: u16| Member {
                slot: Slot::new(n).expect("a slot"),
                unit: ObjId(100 + u32::from(n)),
                controller: Controller::Npc {
                    character: ObjId(10 + u32::from(n)),
                },
            })
            .collect();
        let roster = DetachmentRoster::new(
            DetachmentId(7),
            1,
            members,
            Some(ObjId(11)),
            LeadershipPolicy::default(),
        )
        .expect("a roster");
        let (a, b) = (
            crate::arena::PartyId::nth(0).expect("A"),
            crate::arena::PartyId::nth(1).expect("B"),
        );
        let row = |unit: u32, party| super::super::record::FieldUnit {
            unit: ObjId(unit),
            party,
            pilot: None,
            epoch: None,
            aboard: Vec::new(),
            arrived: false,
            away: None,
        };
        let live = super::super::record::FieldLive {
            format: super::super::record::FIELD_LIVE_FORMAT,
            field: crate::surface::site_record::FieldId {
                site: CanonicalId::new("ahuna_port").expect("a site"),
                ordinal: 1,
            },
            opened_ic: 0,
            segment: 1,
            at_ms: 0,
            parties: Vec::new(),
            units: vec![row(101, a), row(102, a), row(103, b)],
            homes: Vec::new(),
            fire: None,
            owed_exits: Vec::new(),
            held: Vec::new(),
            tally: Vec::new(),
            carried: Vec::new(),
        };
        (roster, live)
    }

    fn payload(objective: FieldObjective) -> FieldPayload {
        FieldPayload {
            objective,
            hold_fire: false,
        }
    }

    /// **Doc 36 cut GC.0 (Q-36-42): SCREEN inside a field** reaches the kernel as a screen
    /// facing the ordered bearing.
    #[test]
    fn gc0_a_screen_in_a_field_holds_its_bearing() {
        let (roster, live) = roster_and_field();
        assert_eq!(
            objective(
                &payload(FieldObjective::Screen {
                    bearing_mdeg: 90_000
                }),
                &roster,
                &live,
                ObjId(101)
            ),
            Ok(Objective::Screen {
                facing_mdeg: 90_000
            })
        );
    }

    /// **Doc 36 cut GC.0 (Q-36-42): SUPPORT inside a field** binds to the slot's unit when it is
    /// another unit of the recipient's own party in the same field, and is NEGATIVE otherwise.
    #[test]
    fn gc0_a_support_in_a_field_binds_to_its_partys_unit_only() {
        let (roster, live) = roster_and_field();
        let support = |slot: u16, unit: u32| {
            objective(
                &payload(FieldObjective::Support {
                    slot: mush_tactics::detachment::Slot::new(slot).expect("a slot"),
                }),
                &roster,
                &live,
                ObjId(unit),
            )
        };
        assert_eq!(
            support(2, 101),
            Ok(Objective::Support { of: ObjId(102) }),
            "the escort's charge in its own party"
        );
        assert_eq!(support(1, 101), Err(()), "never itself");
        assert_eq!(support(3, 101), Err(()), "never another party's unit");
        assert_eq!(support(4, 101), Err(()), "never a unit outside the field");
    }
}
