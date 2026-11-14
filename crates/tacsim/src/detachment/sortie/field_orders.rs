//! IC orders retain source history; Propagation owns their actual transport.
use super::model::OperationId;
use mush_core::{ContactId, ObjId};
use mush_tactics::detachment::{
    DeliveredOrder, DetachmentRoster, Directive, OrderBook, OrderCheckpoint, OrderEnvelope,
    Response, Slot,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Act {
    Issue {
        revision: u64,
        term: u64,
        target: mush_tactics::detachment::OrderTarget,
        directive: Directive<FieldPayload>,
        expires_at_ms: Option<u64>,
    },
    Poll,
    Respond {
        slot: Slot,
        sequence: u64,
        response: mush_tactics::detachment::Response,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandRight {
    pub issuer: ObjId,
    pub unit: ObjId,
    pub allowed: bool,
    pub role_grant: Option<mush_org::unit_authority::UnitGrantId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub operation: OperationId,
    pub facts: super::field::OwnerFacts,
    pub rights: Vec<CommandRight>,
    pub act: Act,
}

pub(super) fn coordinates(
    op: &super::model::OperationRecord,
    act: &Act,
    actor: ObjId,
) -> Result<Vec<(ObjId, ObjId)>, String> {
    match act {
        Act::Issue { .. } => op
            .roster
            .plan
            .slots
            .iter()
            .map(|s| {
                s.unit
                    .map(|u| (u, actor))
                    .ok_or_else(|| "Unit unavailable.".into())
            })
            .collect::<Result<Vec<_>, String>>(),
        Act::Poll => {
            let Some(saved) = &op.command else {
                return Ok(Vec::new());
            };
            let roster = DetachmentRoster::restore(saved).map_err(|e| e.to_string())?;
            let Some(orders) = &op.orders else {
                return Ok(Vec::new());
            };
            let book = OrderBook::restore(&orders.book, &roster).map_err(|e| e.to_string())?;
            Ok(roster
                .members()
                .iter()
                .filter_map(|m| book.pending(m.slot).map(|p| (m.unit, p.issuer)))
                .collect())
        }
        Act::Respond { .. } => Ok(Vec::new()),
    }
}
fn observed_rights(
    coords: &[(ObjId, ObjId)],
    response: mush_org::unit_authority::reader::UnitAuthorityResponse,
) -> Result<Vec<CommandRight>, String> {
    let mush_org::unit_authority::reader::UnitAuthorityResponse::MissionCommands(allowed) =
        response
    else {
        return Err("Command rights unavailable.".into());
    };
    if coords.len() != allowed.len() {
        return Err("Command rights incomplete.".into());
    }
    Ok(coords
        .iter()
        .zip(allowed)
        .map(|((unit, issuer), right)| CommandRight {
            unit: *unit,
            issuer: *issuer,
            allowed: right.allowed,
            role_grant: right.role_grant,
        })
        .collect())
}
pub(super) fn request(
    ctx: &mush_core::Ctx<'_>,
    h: &super::runtime::SortieHandles,
    op: &super::model::OperationRecord,
    actor: ObjId,
    act: Act,
) -> Result<Request, String> {
    let facts = super::field::read_owner_facts(ctx, h, op)?;
    let coords = coordinates(op, &act, actor)?;
    let rights = if coords.is_empty() {
        Vec::new()
    } else {
        let observed = ctx
            .read_owner(
                super::field::rights(ctx.w)?,
                mush_org::unit_authority::reader::UnitAuthorityRequest::MissionCommands {
                    crew: coords.clone(),
                    mission: facts
                        .mission
                        .clone()
                        .ok_or("Mission command unavailable.")?,
                },
            )
            .map_err(|e| e.to_string())?;
        observed_rights(&coords, observed)?
    };
    Ok(Request {
        operation: op.id,
        facts,
        rights,
        act,
    })
}
pub(super) fn verify(
    ctx: &mut mush_core::PrepareCtx<'_>,
    txn: &mut mush_core::ActionTxn<'_>,
    h: &super::runtime::SortieHandles,
    actor: ObjId,
    request: &Request,
) -> Result<(), mush_core::ActionError> {
    use super::runtime::error;
    super::field::verify_owner_facts(ctx, txn, h, request.operation, &request.facts)?;
    let op = super::prepare::operation(&mut &mut *txn, h, request.operation).map_err(error)?;
    let coords = coordinates(&op, &request.act, actor).map_err(error)?;
    if coords.is_empty() {
        if !request.rights.is_empty() {
            return Err(error("Unexpected command rights."));
        }
        return Ok(());
    }
    let handle = super::field::rights(ctx.world().map_err(error)?)
        .map_err(error)?
        .clone();
    let observed = ctx
        .read_owner_preimage(
            txn,
            &handle,
            mush_org::unit_authority::reader::UnitAuthorityRequest::MissionCommands {
                crew: coords.clone(),
                mission: request
                    .facts
                    .mission
                    .clone()
                    .ok_or_else(|| error("Mission command unavailable."))?,
            },
        )
        .map_err(error)?;
    if observed_rights(&coords, observed).map_err(error)? != request.rights {
        return Err(error("Command rights changed."));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldObjective {
    Hold,
    Advance {
        to: (u16, u16),
    },
    Engage {
        contact: Option<ContactId>,
        /// What the leader named besides a campaign contact (doc 32 cut CF.6): a presence row
        /// on open ground, or a contact of the leader's own field observation. Absent on every
        /// order written before CF.6.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<EngageAt>,
    },
    Withdraw,
    Screen {
        bearing_mdeg: u32,
    },
    Support {
        slot: Slot,
    },
    /// One bounded run, followed by physical recovery and unload.
    Mine {
        family: Option<crate::surface::geology::OreFamily>,
    },
    /// **The standing word of a player's unit** (doc 45 unit AP.4a; Q-45-26): the leader sets
    /// the posture a roster member's unit holds when its player is not at its controls. It is
    /// no objective of the unit's: the host writes it to the unit's own record when the order
    /// is delivered (`super::npc`, `posture_order`) and answers WILCO for the unit. Only a
    /// player's slot takes one; a hired crew is not flown for an absent player (Q-45-13).
    Posture {
        posture: crate::posture::Posture,
    },
    /// **Concentrate on one contact** (doc 44 unit MW.14c; the focus-fire order). The leader
    /// names a contact of their own observation inside a field, as an `ENGAGE` does
    /// ([`EngageAt::Contact`]), and every hull ordered that holds that contact itself engages
    /// it. It differs from an `ENGAGE` in three ways (`crate::field::orders::concentrate`): it
    /// may go to the whole roster and reach whoever can take it
    /// ([`mush_tactics::detachment::OrderTarget::Able`]); a hull that does not hold the contact
    /// answers NEGATIVE and keeps the order it had; and once the contact is destroyed or lost
    /// to the hull, the hull goes back to the order it had before ([`Concentration`]).
    Concentrate {
        /// The field's ordinal on its site.
        ordinal: u32,
        /// The row key in the leader's scope.
        id: u64,
        /// The contact's designator in the leader's scope when issued: an identified
        /// contact's is its own callsign, which every hull that has identified it reads too.
        designator: SmolStr,
    },
}
/// **What an `ENGAGE` names** besides a campaign contact (doc 32 cut CF.6; Q-CF-7, Q-CF-15,
/// §4a). Each is resolved by the recipient in its own observation when the order arrives, never
/// handed over as a lock: a name the recipient cannot resolve is a lost contact, and the reply
/// is NEGATIVE.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngageAt {
    /// A flight on the presence picture, by what the leader read (`NE`, `NE-near`): on open
    /// ground the recipient finds the nearest flight that matches from its own unit, and the
    /// order acts as the leader's Hostile mark on that flight's party (§4a).
    Row {
        /// The octant's abbreviation.
        bearing: SmolStr,
        /// The band's word, when the leader gave one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        band: Option<SmolStr>,
    },
    /// A contact in the leader's own observation inside a field (its designator when issued,
    /// its row key in the leader's scope): the kernel re-keys it into the recipient's own
    /// classified observations, or finds nothing there.
    Contact {
        /// The field's ordinal on its site.
        ordinal: u32,
        /// The row key in the leader's scope.
        id: u64,
        /// The designator the leader typed, for the report.
        designator: SmolStr,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPayload {
    pub objective: FieldObjective,
    pub hold_fire: bool,
}
impl FieldPayload {
    /// `side`: the field site's cells along one side (doc 32 cut S.0, §18), which an `ADVANCE`
    /// stays inside — the site's own at issue, the widest site's
    /// ([`crate::surface::ground::MAX_SITE_CELLS`]) for a stored order.
    pub fn validate(&self, roster: &DetachmentRoster, side: u16) -> Result<(), String> {
        match self.objective {
            FieldObjective::Advance { to } if to.0 >= side || to.1 >= side => {
                Err("Destination lies outside this field site.".into())
            }
            FieldObjective::Screen { bearing_mdeg } if bearing_mdeg >= 360_000 => {
                Err("Bearing must be below 360 degrees.".into())
            }
            FieldObjective::Support { slot } if roster.member(slot).is_none() => {
                Err("Support needs an accepted slot.".into())
            }
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyReceipt {
    pub sequence: u64,
    pub envelope: u64,
}

/// Causal coordinates of a message, never an execution permit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldOrderFlight {
    pub sequence: u64,
    pub flight: super::model::FlightKey,
    pub site: mush_core::CanonicalId,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldOrderFlights {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<FieldOrderFlight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<FieldOrderFlight>,
}
/// **A delivered order that lapsed with its leader's absence** (doc 45 unit AP.8; Q-45-13;
/// `crate::field::orders`). Written once, by the poll that finds the order's issuer gone, and
/// kept while that order is the slot's delivered one: the issuer's return does not clear it, and
/// the next order delivered to the slot does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lapse {
    /// The delivered order that lapsed.
    pub sequence: u64,
    /// The sequence reserved for the first half of the stand the recipient takes in its place
    /// (it holds): above every order issued before the lapse and below every order issued after
    /// it.
    pub stand: u64,
    /// The sequence reserved for the stand's second half (it answers a hit): above `stand`.
    pub answer: u64,
    /// When, IC milliseconds.
    pub at_ms: u64,
}
/// **What a hull goes back to after a `CONCENTRATE`** (doc 44 unit MW.14c). Written once, by
/// the poll that delivers the order, and kept while that order is the slot's delivered one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Concentration {
    /// The delivered `CONCENTRATE`.
    pub sequence: u64,
    /// The sequence reserved for the hull's return to its earlier order: above the
    /// `CONCENTRATE`'s own and below every order issued to the slot after it.
    pub resume: u64,
    /// The order that stood on the slot when the `CONCENTRATE` arrived, as it was admitted;
    /// through an earlier `CONCENTRATE`, the order that one replaced. `None` when nothing
    /// stood (no order, or one that had ended): the hull then goes back to no order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Box<OrderEnvelope<FieldPayload>>>,
    /// Set once the order in `before` lapsed with its leader's absence (doc 45 unit AP.8;
    /// Q-45-13) while this `CONCENTRATE` stood over it: `before` is then the `HOLD` that
    /// order's stand begins with, and this is the sequence reserved for it, above `resume`. A
    /// hull that had already gone back to the lapsed order is given the `HOLD` under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stand: Option<u64>,
}

impl Concentration {
    /// **Turn the order this goes back to into the `HOLD` a lapsed order leaves** (doc 45 unit
    /// AP.8), under the lapsed order's own `HOLD FIRE`, to be taken under `stand`. Whether
    /// anything changed: nothing does when there is no order to go back to, or it is done.
    pub(super) fn stand_down(&mut self, stand: u64) -> bool {
        if self.stand.is_some() {
            return false;
        }
        let Some(before) = self.before.as_deref_mut() else {
            return false;
        };
        let hold_fire = matches!(
            &before.directive,
            Directive::Objective(payload) if payload.hold_fire
        );
        before.directive = Directive::Objective(FieldPayload {
            objective: FieldObjective::Hold,
            hold_fire,
        });
        self.stand = Some(stand);
        true
    }
}

/// **One order an NPC leader gave** (doc 44 unit MW.14c), kept so the roster's organiser and
/// its players can read what their NPC commander ordered (`+sortie/orders`), and so the
/// commander itself can read what it last ordered and when. The transport
/// keeps a message's reference and not its words, and the book keeps one order a slot, so an
/// NPC's superseded orders would otherwise be nowhere a player can read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Given {
    /// The order's sequence in the roster's book.
    pub sequence: u64,
    /// The slot it went to.
    pub slot: Slot,
    /// The NPC who gave it.
    pub issuer: ObjId,
    /// What it said.
    pub directive: Directive<FieldPayload>,
    /// When it was admitted, IC milliseconds: what the commander's dwell is read from
    /// (`super::commander`), so the dwell survives a restart with the history.
    pub at_ms: u64,
}

/// **How many of an NPC leader's orders the roster's history keeps** (doc 44 unit MW.14c): the
/// last sixteen, as many as `+sortie/orders` shows of a player's own.
pub const GIVEN_KEPT: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldOrders {
    pub schema: u16,
    pub book: OrderCheckpoint<FieldPayload>,
    /// Pending outbound references; a source message alone is never delivered.
    pub outbound: BTreeMap<Slot, u64>,
    pub received: BTreeMap<Slot, u64>,
    pub replies: BTreeMap<Slot, ReplyReceipt>,
    /// Old messages without these coordinates remain advisory. A coordinate
    /// is never silently rebound to a subsequent departure at the same site.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flights: BTreeMap<Slot, FieldOrderFlights>,
    /// The delivered orders that lapsed with their leader's absence (doc 45 unit AP.8). Absent
    /// on every history written before AP.8.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lapsed: BTreeMap<Slot, Lapse>,
    /// What each hull under a delivered `CONCENTRATE` goes back to (doc 44 unit MW.14c).
    /// Absent on every history written before MW.14c.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub concentrating: BTreeMap<Slot, Concentration>,
    /// The last [`GIVEN_KEPT`] orders an NPC leader gave, oldest first (doc 44 unit MW.14c).
    /// Absent on every history written before MW.14c.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub given: Vec<Given>,
}
/// Whether `directive` is a `CONCENTRATE` (doc 44 unit MW.14c).
#[must_use]
pub fn concentrates(directive: &Directive<FieldPayload>) -> bool {
    matches!(
        directive,
        Directive::Objective(FieldPayload {
            objective: FieldObjective::Concentrate { .. },
            ..
        })
    )
}
/// Whether `directive` is a `POSTURE` (doc 45 unit AP.4a).
#[must_use]
pub(super) const fn sets_posture(directive: &Directive<FieldPayload>) -> bool {
    matches!(
        directive,
        Directive::Objective(FieldPayload {
            objective: FieldObjective::Posture { .. },
            ..
        })
    )
}
/// **Whether slot `member` may take `directive` now** (the issue and the delivery read this
/// one rule): a member the roster counts available, or, for a `POSTURE` alone, a player's
/// unit a mind flies because its player has gone (doc 47 unit CO.3, R-47-4;
/// `crate::field::orders::minded_for`). The order then goes to the unit's mind. Any other
/// order to a gone member's unit is refused as before.
pub(super) fn takes(
    world: &mush_core::World,
    roster: &DetachmentRoster,
    member: &mush_tactics::detachment::Member,
    directive: &Directive<FieldPayload>,
) -> bool {
    roster.member_available(member.slot)
        || (sets_posture(directive)
            && match member.controller {
                mush_tactics::detachment::Controller::Player { character } => {
                    crate::field::orders::minded_for(world, member.unit, character)
                }
                _ => false,
            })
}
pub(super) fn message_key(id: OperationId, sequence: u64, slot: Slot) -> String {
    format!(
        "dt:{}:{}:{}:{}:{}",
        id.organizer.0,
        id.draft.0,
        id.activation,
        sequence,
        slot.get()
    )
}
pub(super) fn response_word(response: mush_tactics::detachment::Response) -> &'static str {
    match response {
        mush_tactics::detachment::Response::Wilco => "wilco",
        mush_tactics::detachment::Response::Negative => "negative",
    }
}
impl FieldOrders {
    pub fn new(roster: &DetachmentRoster) -> Self {
        Self {
            schema: 1,
            book: OrderBook::new(roster.id()).checkpoint(),
            outbound: BTreeMap::new(),
            received: BTreeMap::new(),
            replies: BTreeMap::new(),
            flights: BTreeMap::new(),
            lapsed: BTreeMap::new(),
            concentrating: BTreeMap::new(),
            given: Vec::new(),
        }
    }
    /// **What `slot` goes back to after its delivered `CONCENTRATE` `sequence`** (doc 44 unit
    /// MW.14c), when that order is the one recorded.
    #[must_use]
    pub fn concentration_of(&self, slot: Slot, sequence: u64) -> Option<&Concentration> {
        self.concentrating
            .get(&slot)
            .filter(|record| record.sequence == sequence)
    }
    /// Drop every concentration whose order is no longer its slot's delivered one.
    pub(super) fn prune_concentrating(&mut self, book: &OrderBook<FieldPayload>) {
        self.concentrating.retain(|slot, record| {
            book.delivered(*slot)
                .is_some_and(|delivered| delivered.order.sequence == record.sequence)
        });
    }
    /// **The order a slot stands on, seen through a `CONCENTRATE`** (doc 44 unit MW.14c): the
    /// delivered order, or, when that is a `CONCENTRATE`, the order the hull goes back to after
    /// it. `None` when nothing was delivered, or a `CONCENTRATE` replaced nothing.
    #[must_use]
    pub fn underlying<'a>(
        &'a self,
        book: &'a OrderBook<FieldPayload>,
        slot: Slot,
    ) -> Option<&'a OrderEnvelope<FieldPayload>> {
        let delivered = book.delivered(slot)?;
        if concentrates(&delivered.order.directive) {
            self.concentration_of(slot, delivered.order.sequence)?
                .before
                .as_deref()
        } else {
            Some(&delivered.order)
        }
    }
    /// **Record a `CONCENTRATE` just delivered to `slot`** (doc 44 unit MW.14c), in the poll
    /// that delivered it: what stood there before (`previous`, the slot's delivered order until
    /// this delivery), and the sequence its hull returns to that under. An order of any other
    /// kind clears the slot's record.
    ///
    /// What stood before is the earlier order while it still binds: one that had completed,
    /// run out, been answered NEGATIVE or been a cancellation is nothing to go back to. An
    /// order that had lapsed with its leader's absence (doc 45 unit AP.8) is gone back to as
    /// the stand it left: a `HOLD`, under the lapsed order's own `HOLD FIRE`. Through an
    /// earlier `CONCENTRATE`, what that one recorded.
    pub(super) fn note_delivery(
        &mut self,
        book: &mut OrderBook<FieldPayload>,
        slot: Slot,
        previous: Option<DeliveredOrder<FieldPayload>>,
        now_ms: u64,
    ) -> Result<(), String> {
        let Some(sequence) = book
            .delivered(slot)
            .filter(|delivered| concentrates(&delivered.order.directive))
            .map(|delivered| delivered.order.sequence)
        else {
            self.concentrating.remove(&slot);
            return Ok(());
        };
        let before = previous.and_then(|previous| {
            if concentrates(&previous.order.directive) {
                return self
                    .concentration_of(slot, previous.order.sequence)
                    .and_then(|record| record.before.clone());
            }
            let ended = previous.completed_at_ms.is_some()
                || previous
                    .order
                    .expires_at_ms
                    .is_some_and(|end| now_ms >= end)
                || previous
                    .acknowledgement
                    .is_some_and(|answer| answer.response == Response::Negative);
            let mut order = previous.order;
            let lapsed = self.lapse_of(slot, order.sequence).is_some();
            match &order.directive {
                Directive::Objective(payload)
                    if !ended && !matches!(payload.objective, FieldObjective::Posture { .. }) =>
                {
                    if lapsed {
                        order.directive = Directive::Objective(FieldPayload {
                            objective: FieldObjective::Hold,
                            hold_fire: payload.hold_fire,
                        });
                    }
                    Some(Box::new(order))
                }
                _ => None,
            }
        });
        let resume = book.reserve_sequence(now_ms).map_err(|e| e.to_string())?;
        self.concentrating.insert(
            slot,
            Concentration {
                sequence,
                resume,
                before,
                stand: None,
            },
        );
        Ok(())
    }
    /// **Keep an order an NPC leader just gave** (doc 44 unit MW.14c), dropping the oldest
    /// past [`GIVEN_KEPT`].
    pub(super) fn note_given(
        &mut self,
        (sequence, slot): (u64, Slot),
        issuer: ObjId,
        directive: &Directive<FieldPayload>,
        at_ms: u64,
    ) {
        self.given.push(Given {
            sequence,
            slot,
            issuer,
            directive: directive.clone(),
            at_ms,
        });
        let over = self.given.len().saturating_sub(GIVEN_KEPT);
        self.given.drain(..over);
    }
    /// **The lapse standing on `slot`'s delivered order**, if that order lapsed (doc 45 unit
    /// AP.8).
    #[must_use]
    pub fn lapse_of(&self, slot: Slot, sequence: u64) -> Option<Lapse> {
        self.lapsed
            .get(&slot)
            .copied()
            .filter(|lapse| lapse.sequence == sequence)
    }
    /// Drop every lapse whose order is no longer its slot's delivered one.
    pub(super) fn prune_lapsed(&mut self, book: &OrderBook<FieldPayload>) {
        self.lapsed.retain(|slot, lapse| {
            book.delivered(*slot)
                .is_some_and(|delivered| delivered.order.sequence == lapse.sequence)
        });
    }
    pub fn validate(&self, roster: &DetachmentRoster) -> Result<(), String> {
        let book = OrderBook::restore(&self.book, roster).map_err(|e| e.to_string())?;
        if self.schema != 1
            || self.outbound.len() > 6
            || self.received.len() > 6
            || self.replies.len() > 6
            || self.flights.len() > 6
            || self.lapsed.len() > 6
            || self.concentrating.len() > 6
            || self.given.len() > GIVEN_KEPT
        {
            return Err("Invalid field message history.".into());
        }
        if self.concentrating.iter().any(|(slot, record)| {
            record.resume <= record.sequence
                || record.resume > book.sequence()
                || record
                    .stand
                    .is_some_and(|stand| stand <= record.resume || stand > book.sequence())
                || book.delivered(*slot).is_none_or(|delivered| {
                    delivered.order.sequence != record.sequence
                        || !concentrates(&delivered.order.directive)
                })
                || record.before.as_deref().is_some_and(|before| {
                    before.sequence >= record.sequence
                        || match &before.directive {
                            Directive::Objective(payload) => payload
                                .validate(roster, crate::surface::ground::MAX_SITE_CELLS)
                                .is_err(),
                            Directive::Cancel => true,
                        }
                })
        }) {
            return Err("A concentration differs from its delivered order.".into());
        }
        if self.given.iter().any(|given| {
            given.sequence == 0 || given.sequence > book.sequence() || given.issuer.0 == 0
        }) || self.given.windows(2).any(|pair| {
            (pair[0].sequence, pair[0].slot) >= (pair[1].sequence, pair[1].slot)
                || pair[0].at_ms > pair[1].at_ms
        }) {
            return Err("An NPC leader's order history is out of order.".into());
        }
        if self.lapsed.iter().any(|(slot, lapse)| {
            lapse.stand <= lapse.sequence
                || lapse.answer <= lapse.stand
                || lapse.answer > book.sequence()
                || book.delivered(*slot).is_none_or(|delivered| {
                    delivered.order.sequence != lapse.sequence
                        || lapse.at_ms < delivered.delivered_at_ms
                })
        }) {
            return Err("A lapse differs from its delivered order.".into());
        }
        for m in roster.members() {
            for order in book
                .pending(m.slot)
                .into_iter()
                .chain(book.delivered(m.slot).map(|d| &d.order))
            {
                if let Directive::Objective(payload) = &order.directive {
                    payload.validate(roster, crate::surface::ground::MAX_SITE_CELLS)?;
                }
            }
        }
        if self
            .outbound
            .iter()
            .any(|(s, id)| *id == 0 || book.pending(*s).is_none())
            || self
                .received
                .iter()
                .any(|(s, id)| *id == 0 || book.delivered(*s).is_none())
            || self.replies.iter().any(|(s, r)| {
                r.envelope == 0
                    || book.delivered(*s).is_none_or(|d| {
                        d.order.sequence != r.sequence || d.acknowledgement.is_none()
                    })
            })
        {
            return Err("Field transport reference differs from its source history.".into());
        }
        for (slot, flights) in &self.flights {
            let member = roster
                .member(*slot)
                .ok_or("Field flight has no accepted slot.")?;
            for (context, order) in [
                (&flights.pending, book.pending(*slot)),
                (&flights.standing, book.delivered(*slot).map(|d| &d.order)),
            ] {
                if let Some(context) = context
                    && (context.sequence == 0
                        || context.flight.ordinal == 0
                        || context.flight.unit != member.unit
                        || order.is_none_or(|o| o.sequence != context.sequence))
                {
                    return Err("Field message flight differs from its source order.".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn forget_pending_flight(&mut self, slot: Slot) {
        if let Some(context) = self.flights.get_mut(&slot) {
            context.pending = None;
        }
        self.flights
            .retain(|_, c| c.pending.is_some() || c.standing.is_some());
    }
    pub(super) fn attach(
        &mut self,
        id: OperationId,
        roster: &DetachmentRoster,
        envelopes: &[mush_propagation::Envelope],
    ) -> Result<(), String> {
        use mush_propagation::record::DispatchKind;
        let book = OrderBook::restore(&self.book, roster).map_err(|e| e.to_string())?;
        for e in envelopes {
            let reference = e
                .dispatch_ref()
                .ok_or("Field transport reference unavailable.")?;
            let mut attached = false;
            for m in roster.members() {
                if e.kind() == DispatchKind::Detachment
                    && let Some(order) = book.pending(m.slot)
                    && reference.key == message_key(id, order.sequence, m.slot)
                    && e.issuer() == order.issuer
                    && m.controller.character() == Some(e.recipient())
                {
                    self.outbound.insert(m.slot, e.id());
                    attached = true;
                    break;
                }
                if e.kind() == DispatchKind::DetachmentReply
                    && let Some(delivered) = book.delivered(m.slot)
                    && delivered.acknowledgement.as_ref().is_some_and(|a| {
                        reference.key
                            == format!(
                                "{}:reply:{}",
                                message_key(id, delivered.order.sequence, m.slot),
                                response_word(a.response)
                            )
                    })
                    && e.recipient() == delivered.order.issuer
                    && delivered
                        .acknowledgement
                        .as_ref()
                        .is_some_and(|a| a.by == e.issuer())
                {
                    self.replies.insert(
                        m.slot,
                        ReplyReceipt {
                            sequence: delivered.order.sequence,
                            envelope: e.id(),
                        },
                    );
                    attached = true;
                    break;
                }
            }
            if !attached {
                return Err("Transport does not match the admitted field message.".into());
            }
        }
        self.validate(roster)
    }
}

/// **The delivered orders that lapse now and are not yet recorded as lapsed** (doc 45 unit
/// AP.8; `crate::field::orders::lapses`): the slot, the order's sequence and its issuer.
///
/// Only an order a hired NPC is carrying out on its current field flight lapses: it was
/// delivered on the flight the recipient's unit is on now (an order outlives its flight in the
/// book, and an old one binds nothing), the recipient did not answer it NEGATIVE, and it has
/// neither completed nor run out its own deadline. A player recipient's order is advice the
/// player flies by hand, and is left as it reads.
pub(super) fn lapsing(
    world: &mush_core::World,
    read: &mut impl super::runtime::Read,
    h: &super::runtime::SortieHandles,
    roster: &DetachmentRoster,
    orders: &FieldOrders,
    book: &OrderBook<FieldPayload>,
    now_ms: u64,
) -> Vec<(Slot, u64, ObjId)> {
    use mush_tactics::detachment::{Controller, Response};
    roster
        .members()
        .iter()
        .filter(|member| matches!(member.controller, Controller::Npc { .. }))
        .filter_map(|member| {
            let delivered = book.delivered(member.slot)?;
            let order = &delivered.order;
            if delivered.completed_at_ms.is_some()
                || order.expires_at_ms.is_some_and(|end| now_ms >= end)
                || delivered
                    .acknowledgement
                    .is_some_and(|answer| answer.response == Response::Negative)
                || orders.lapse_of(member.slot, order.sequence).is_some()
                || !crate::field::orders::lapses(world, order)
            {
                return None;
            }
            let context = orders
                .flights
                .get(&member.slot)
                .and_then(|flights| flights.standing.as_ref())
                .filter(|context| context.sequence == order.sequence)?;
            let (_, flight) = super::prepare::current(read, h, member.unit).ok()?;
            (flight.key == context.flight
                && flight.site == context.site
                && flight.phase == super::model::Phase::Field)
                .then_some((member.slot, order.sequence, order.issuer))
        })
        .collect()
}

/// **The slots whose `CONCENTRATE` stands over an order that lapses now** (doc 44 unit
/// MW.14c; doc 45 unit AP.8): the order the hull goes back to presses, a player gave it, and
/// the field has that player gone (`crate::field::orders::lapses`). The `CONCENTRATE`'s own
/// lapse is [`lapsing`]'s; this is the order under it, which [`lapsing`] cannot see.
pub(super) fn under_lapsing(world: &mush_core::World, orders: &FieldOrders) -> Vec<Slot> {
    orders
        .concentrating
        .iter()
        .filter(|(_, record)| {
            record.stand.is_none()
                && record
                    .before
                    .as_deref()
                    .is_some_and(|before| crate::field::orders::lapses(world, before))
        })
        .map(|(slot, _)| *slot)
        .collect()
}

/// **Record every lapse that is due** (doc 45 unit AP.8; Q-45-13), in whichever transaction
/// saves the roster: an order's poll, an issue, a reply or a refresh. The lead passes and the
/// order lapses in one write, so no successor ever leads over an order that is still pressing
/// for a leader who is gone. Each lapse reserves the two sequences its recipient's stand takes
/// (`crate::field::orders::stand`), above every order issued so far. A lapse whose order has
/// been replaced is dropped first. What was recorded, for the transaction's observations.
pub(super) fn lapse_due(
    world: &mush_core::World,
    (read, h): (
        &mut impl super::runtime::Read,
        &super::runtime::SortieHandles,
    ),
    roster: &DetachmentRoster,
    orders: &mut FieldOrders,
    book: &mut OrderBook<FieldPayload>,
    now_ms: u64,
) -> Result<Vec<serde_json::Value>, String> {
    orders.prune_lapsed(book);
    orders.prune_concentrating(book);
    let mut recorded = Vec::new();
    for (slot, sequence, issuer) in lapsing(world, read, h, roster, orders, book, now_ms) {
        let stand = book.reserve_sequence(now_ms).map_err(|e| e.to_string())?;
        let answer = book.reserve_sequence(now_ms).map_err(|e| e.to_string())?;
        orders.lapsed.insert(
            slot,
            Lapse {
                sequence,
                stand,
                answer,
                at_ms: now_ms,
            },
        );
        recorded.push(serde_json::json!({
            "slot": slot, "lapsed": sequence, "issuer": issuer, "stand": stand, "answer": answer,
        }));
    }
    // Doc 44 unit MW.14c: the order a hull goes back to after a CONCENTRATE lapses under it,
    // by the same rule. It becomes the HOLD a lapsed order leaves, under a sequence of its own.
    for slot in under_lapsing(world, orders) {
        let stand = book.reserve_sequence(now_ms).map_err(|e| e.to_string())?;
        if let Some(record) = orders.concentrating.get_mut(&slot)
            && record.stand_down(stand)
        {
            recorded.push(serde_json::json!({
                "slot": slot, "under": record.sequence, "stand": stand,
            }));
        }
    }
    Ok(recorded)
}

/// **A lapse a write records for the first time** (doc 45 unit AP.8): what its submission
/// tells.
pub(super) struct NewLapse {
    /// The recipient's slot.
    pub slot: Slot,
    /// The lapsed order's sequence.
    pub sequence: u64,
    /// The leader who gave it.
    pub issuer: ObjId,
    /// The order, as that leader typed it.
    pub directive: Directive<FieldPayload>,
}

/// **The lapses `after` records that `before` did not** (doc 45 unit AP.8), each with the
/// order as its leader typed it and who gave it: what a submission tells
/// (`super::runtime::enqueue`).
pub(super) fn new_lapses(
    before: Option<&FieldOrders>,
    after: &super::model::OperationRecord,
) -> Vec<NewLapse> {
    let (Some(saved), Some(orders)) = (&after.command, &after.orders) else {
        return Vec::new();
    };
    let Ok(roster) = DetachmentRoster::restore(saved) else {
        return Vec::new();
    };
    let Ok(book) = OrderBook::restore(&orders.book, &roster) else {
        return Vec::new();
    };
    orders
        .lapsed
        .iter()
        .filter(|(slot, lapse)| {
            before.is_none_or(|before| before.lapse_of(**slot, lapse.sequence).is_none())
        })
        .filter_map(|(slot, lapse)| {
            let order = &book.delivered(*slot)?.order;
            (order.sequence == lapse.sequence).then(|| NewLapse {
                slot: *slot,
                sequence: lapse.sequence,
                issuer: order.issuer,
                directive: order.directive.clone(),
            })
        })
        .collect()
}

pub(super) fn needs_poll(
    world: &mush_core::World,
    read: &mut impl super::runtime::Read,
    h: &super::runtime::SortieHandles,
    op: &super::model::OperationRecord,
) -> bool {
    let (Some(saved), Some(orders)) = (&op.command, &op.orders) else {
        return false;
    };
    let Ok(roster) = DetachmentRoster::restore(saved) else {
        return false;
    };
    let Ok(book) = OrderBook::restore(&orders.book, &roster) else {
        return false;
    };
    let now = u64::try_from(world.world_ic_now().at)
        .unwrap_or(0)
        .saturating_mul(1000);
    // Doc 45 unit AP.8: an order whose leader is gone is the poll's to lapse.
    if !lapsing(world, read, h, &roster, orders, &book, now).is_empty()
        || !under_lapsing(world, orders).is_empty()
    {
        return true;
    }
    roster.members().iter().any(|m| {
        book.pending(m.slot).is_some_and(|p| {
            p.command_term != roster.term()
                || roster.leader().is_none_or(|l| l.character != p.issuer)
                || p.expires_at_ms.is_some_and(|end| now >= end)
                || orders
                    .outbound
                    .get(&m.slot)
                    .and_then(|id| {
                        world
                            .module_state::<mush_propagation::PropagationStore>()?
                            .envelope(*id)
                    })
                    .is_none_or(|e| {
                        matches!(
                            e.state(),
                            mush_propagation::EnvelopeState::Delivered
                                | mush_propagation::EnvelopeState::Acknowledged
                                | mush_propagation::EnvelopeState::Expired
                                | mush_propagation::EnvelopeState::Superseded
                        )
                    })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **An `ADVANCE` follows the site's side** (doc 32 cut S.0, §18): `(300, 300)` is on a 512
    /// site and off a 256 one; a stored order is held only to the widest site.
    #[test]
    fn s0_advance_follows_the_site_side() {
        let roster = DetachmentRoster::new(
            mush_tactics::detachment::DetachmentId(7),
            1,
            vec![mush_tactics::detachment::Member {
                slot: Slot::new(1).expect("a slot"),
                unit: ObjId(101),
                controller: mush_tactics::detachment::Controller::Automated,
            }],
            None,
            mush_tactics::detachment::LeadershipPolicy::default(),
        )
        .expect("a roster");
        let advance = |to| FieldPayload {
            objective: FieldObjective::Advance { to },
            hold_fire: false,
        };
        assert_eq!(advance((300, 300)).validate(&roster, 512), Ok(()));
        assert!(advance((300, 300)).validate(&roster, 256).is_err());
        assert_eq!(advance((255, 255)).validate(&roster, 256), Ok(()));
        assert!(advance((256, 0)).validate(&roster, 256).is_err());
        assert!(
            advance((512, 0))
                .validate(&roster, crate::surface::ground::MAX_SITE_CELLS)
                .is_err()
        );
    }

    /// A roster of two hired hulls, units 101 and 102 in slots 1 and 2, led by the NPC in
    /// slot 1 (the plan's commander), reconciled at time nought.
    fn led_roster() -> DetachmentRoster {
        use mush_tactics::detachment::roster::{Connection, MemberFacts};
        use mush_tactics::detachment::{Controller, DetachmentId, LeadershipPolicy, Member};
        let members: Vec<Member> = (1..=2u16)
            .map(|n| Member {
                slot: Slot::new(n).expect("a slot"),
                unit: ObjId(100 + u32::from(n)),
                controller: Controller::Npc {
                    character: ObjId(10 + u32::from(n)),
                },
            })
            .collect();
        let facts: Vec<MemberFacts> = members
            .iter()
            .map(|member| MemberFacts {
                slot: member.slot,
                capable: true,
                controls_unit: true,
                unit_disabled: false,
                connection: Connection::Connected,
                command_authorized: true,
            })
            .collect();
        let mut roster = DetachmentRoster::new(
            DetachmentId(7),
            1,
            members,
            Some(ObjId(11)),
            LeadershipPolicy::default(),
        )
        .expect("a roster");
        roster.reconcile(0, &facts).expect("reconciled");
        roster
    }

    fn payload(objective: FieldObjective) -> Directive<FieldPayload> {
        Directive::Objective(FieldPayload {
            objective,
            hold_fire: false,
        })
    }

    fn concentrate_on(id: u64) -> Directive<FieldPayload> {
        payload(FieldObjective::Concentrate {
            ordinal: 1,
            id,
            designator: SmolStr::new("BK"),
        })
    }

    /// Issue `directive` to slot 2 as the roster's leader and deliver it, as a poll does:
    /// the delivery noted in the history. The order's sequence.
    fn deliver(
        roster: &DetachmentRoster,
        orders: &mut FieldOrders,
        book: &mut OrderBook<FieldPayload>,
        directive: Directive<FieldPayload>,
        now_ms: u64,
    ) -> u64 {
        use mush_tactics::detachment::{OrderRequest, OrderTarget};
        let slot = Slot::new(2).expect("a slot");
        let issued = book
            .issue(
                roster,
                now_ms,
                OrderRequest {
                    issuer: ObjId(11),
                    roster_revision: roster.revision(),
                    command_term: roster.term(),
                    target: OrderTarget::Slot(slot),
                    directive,
                    expires_at_ms: None,
                },
                |_| true,
            )
            .expect("issued");
        let previous = book.delivered(slot).cloned();
        book.deliver(roster, slot, issued.sequence, now_ms, |_| true)
            .expect("delivered");
        orders
            .note_delivery(book, slot, previous, now_ms)
            .expect("noted");
        orders.prune_lapsed(book);
        orders.prune_concentrating(book);
        orders.book = book.checkpoint();
        orders.validate(roster).expect("a valid history");
        issued.sequence
    }

    /// **Doc 44 unit MW.14c: a history written before the unit still loads.** The text is a
    /// roster's message history as it was stored before `concentrating` and `given` existed;
    /// it reads with neither, and a history that holds neither is stored without their keys.
    #[test]
    fn mw14c_a_history_written_before_the_unit_loads() {
        let roster = led_roster();
        let empty = FieldOrders::new(&roster);
        let stored = serde_json::to_string(&empty).expect("encode");
        assert!(
            !stored.contains("concentrating") && !stored.contains("given"),
            "{stored}"
        );
        let old: FieldOrders = serde_json::from_str(&stored).expect("an older history loads");
        assert!(old.concentrating.is_empty() && old.given.is_empty());
        old.validate(&roster).expect("and is valid");
    }

    /// **Doc 44 unit MW.14c: a delivered `CONCENTRATE` records what its hull goes back to.**
    /// The order that stood; through a second `CONCENTRATE`, the order the first replaced; an
    /// order the hull had refused is nothing to go back to; and any other order delivered
    /// after it clears the record. The return's sequence sits above the `CONCENTRATE` and
    /// below the next order.
    #[test]
    fn mw14c_a_delivered_concentrate_records_what_it_replaced() {
        use mush_tactics::detachment::Response;
        let roster = led_roster();
        let slot = Slot::new(2).expect("a slot");
        let mut orders = FieldOrders::new(&roster);
        let mut book = OrderBook::restore(&orders.book, &roster).expect("a book");

        let hold = deliver(
            &roster,
            &mut orders,
            &mut book,
            payload(FieldObjective::Hold),
            1,
        );
        assert!(orders.concentrating.is_empty(), "a HOLD records nothing");
        assert_eq!(
            orders.underlying(&book, slot).map(|order| order.sequence),
            Some(hold)
        );

        let first = deliver(&roster, &mut orders, &mut book, concentrate_on(70), 2);
        let record = orders
            .concentration_of(slot, first)
            .expect("recorded")
            .clone();
        assert_eq!(
            record.before.as_deref().map(|order| order.sequence),
            Some(hold)
        );
        assert!(record.resume > first && record.resume <= book.sequence());
        assert_eq!(
            orders.underlying(&book, slot).map(|order| &order.directive),
            Some(&payload(FieldObjective::Hold)),
            "the slot stands on its HOLD, seen through the CONCENTRATE"
        );

        // A second CONCENTRATE goes back to what the first replaced, not to the first.
        let second = deliver(&roster, &mut orders, &mut book, concentrate_on(71), 3);
        assert!(second > record.resume, "the next order is above the return");
        let chained = orders.concentration_of(slot, second).expect("recorded");
        assert_eq!(
            chained.before.as_deref().map(|order| order.sequence),
            Some(hold)
        );
        assert!(orders.concentration_of(slot, first).is_none());
        // The stored history round-trips with its record.
        let text = serde_json::to_string(&orders).expect("encode");
        let again: FieldOrders = serde_json::from_str(&text).expect("decode");
        assert_eq!(again, orders);
        again.validate(&roster).expect("valid");

        // Any other order clears it.
        let advance = deliver(
            &roster,
            &mut orders,
            &mut book,
            payload(FieldObjective::Advance { to: (4, 4) }),
            4,
        );
        assert!(orders.concentrating.is_empty());

        // An order the hull refused is nothing to go back to.
        book.respond(slot, advance, ObjId(12), Response::Negative, 5)
            .expect("answered");
        let third = deliver(&roster, &mut orders, &mut book, concentrate_on(72), 6);
        let record = orders.concentration_of(slot, third).expect("recorded");
        assert!(record.before.is_none(), "{record:?}");
        assert!(orders.underlying(&book, slot).is_none());
    }

    /// **Doc 44 unit MW.14c: a `CONCENTRATE` over an order that lapsed goes back to the stand
    /// that order left**, a `HOLD` under the lapsed order's own `HOLD FIRE` (doc 45 unit AP.8),
    /// and never to the attack that lapsed.
    #[test]
    fn mw14c_a_concentrate_over_a_lapsed_order_goes_back_to_a_hold() {
        let roster = led_roster();
        let slot = Slot::new(2).expect("a slot");
        let mut orders = FieldOrders::new(&roster);
        let mut book = OrderBook::restore(&orders.book, &roster).expect("a book");
        let engage = deliver(
            &roster,
            &mut orders,
            &mut book,
            Directive::Objective(FieldPayload {
                objective: FieldObjective::Engage {
                    contact: None,
                    at: None,
                },
                hold_fire: true,
            }),
            1,
        );
        let stand = book.reserve_sequence(2).expect("reserved");
        let answer = book.reserve_sequence(2).expect("reserved");
        orders.lapsed.insert(
            slot,
            Lapse {
                sequence: engage,
                stand,
                answer,
                at_ms: 2,
            },
        );
        let concentrate = deliver(&roster, &mut orders, &mut book, concentrate_on(70), 3);
        let record = orders
            .concentration_of(slot, concentrate)
            .expect("recorded");
        assert_eq!(
            record.before.as_deref().map(|order| &order.directive),
            Some(&Directive::Objective(FieldPayload {
                objective: FieldObjective::Hold,
                hold_fire: true,
            }))
        );
    }

    /// **Doc 44 unit MW.14c: the order under a `CONCENTRATE` can lapse too.** Once its leader
    /// is gone the hull is not to go back to the attack that leader started: the record's
    /// earlier order becomes a `HOLD`, under that order's own `HOLD FIRE`, with a sequence of
    /// its own above the return's; it is done once; and the history is still valid.
    #[test]
    fn mw14c_the_order_under_a_concentrate_stands_down_to_a_hold() {
        let roster = led_roster();
        let slot = Slot::new(2).expect("a slot");
        let mut orders = FieldOrders::new(&roster);
        let mut book = OrderBook::restore(&orders.book, &roster).expect("a book");
        let advance = deliver(
            &roster,
            &mut orders,
            &mut book,
            Directive::Objective(FieldPayload {
                objective: FieldObjective::Advance { to: (4, 4) },
                hold_fire: true,
            }),
            1,
        );
        let concentrate = deliver(&roster, &mut orders, &mut book, concentrate_on(70), 2);
        let resume = orders
            .concentration_of(slot, concentrate)
            .expect("recorded")
            .resume;
        let stand = book.reserve_sequence(3).expect("reserved");
        let record = orders.concentrating.get_mut(&slot).expect("recorded");
        assert!(record.stand_down(stand));
        assert!(!record.stand_down(stand + 1), "once");
        assert_eq!(record.stand, Some(stand));
        assert!(stand > resume);
        let before = record.before.as_deref().expect("an order to go back to");
        assert_eq!(
            before.sequence, advance,
            "the same order's place in the book"
        );
        assert_eq!(
            before.directive,
            Directive::Objective(FieldPayload {
                objective: FieldObjective::Hold,
                hold_fire: true,
            })
        );
        orders.book = book.checkpoint();
        orders.validate(&roster).expect("valid");
        let text = serde_json::to_string(&orders).expect("encode");
        assert_eq!(
            serde_json::from_str::<FieldOrders>(&text).expect("decode"),
            orders
        );
        // Nothing to go back to: nothing to stand down.
        let mut bare = Concentration {
            sequence: 9,
            resume: 10,
            before: None,
            stand: None,
        };
        assert!(!bare.stand_down(11) && bare.stand.is_none());
    }

    /// **Doc 44 unit MW.14c: an NPC leader's orders are kept, the last sixteen**, oldest
    /// first.
    #[test]
    fn mw14c_an_npc_leaders_last_orders_are_kept() {
        let roster = led_roster();
        let mut orders = FieldOrders::new(&roster);
        let mut book = OrderBook::restore(&orders.book, &roster).expect("a book");
        let slot = Slot::new(2).expect("a slot");
        for n in 0..(GIVEN_KEPT as u64 + 4) {
            let directive = payload(FieldObjective::Advance {
                to: (u16::try_from(n).expect("small"), 4),
            });
            let sequence = deliver(&roster, &mut orders, &mut book, directive.clone(), n + 1);
            orders.note_given((sequence, slot), ObjId(11), &directive, n + 1);
            orders.validate(&roster).expect("valid");
        }
        assert_eq!(orders.given.len(), GIVEN_KEPT);
        assert_eq!(orders.given.first().map(|given| given.sequence), Some(5));
        assert_eq!(
            orders.given.last().map(|given| given.sequence),
            Some(GIVEN_KEPT as u64 + 4)
        );
        // A history whose kept orders are out of order is refused.
        orders.given.swap(0, 1);
        assert!(orders.validate(&roster).is_err());
    }
}
