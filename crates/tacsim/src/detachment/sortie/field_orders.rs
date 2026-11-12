//! IC orders retain source history; Propagation owns their actual transport.
use super::model::OperationId;
use mush_core::{ContactId, ObjId};
use mush_tactics::detachment::{DetachmentRoster, Directive, OrderBook, OrderCheckpoint, Slot};
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
        }
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
        {
            return Err("Invalid field message history.".into());
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
    if !lapsing(world, read, h, &roster, orders, &book, now).is_empty() {
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
}
