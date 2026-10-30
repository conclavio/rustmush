//! IC orders retain source history; Propagation owns their actual transport.
use super::model::OperationId;
use mush_core::{ContactId, ObjId};
use mush_tactics::detachment::{DetachmentRoster, Directive, OrderBook, OrderCheckpoint, Slot};
use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPayload {
    pub objective: FieldObjective,
    pub hold_fire: bool,
}
impl FieldPayload {
    pub fn validate(&self, roster: &DetachmentRoster) -> Result<(), String> {
        match self.objective {
            FieldObjective::Advance { to }
                if to.0 >= crate::surface::ground::SITE_CELLS
                    || to.1 >= crate::surface::ground::SITE_CELLS =>
            {
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
        }
    }
    pub fn validate(&self, roster: &DetachmentRoster) -> Result<(), String> {
        let book = OrderBook::restore(&self.book, roster).map_err(|e| e.to_string())?;
        if self.schema != 1
            || self.outbound.len() > 6
            || self.received.len() > 6
            || self.replies.len() > 6
            || self.flights.len() > 6
        {
            return Err("Invalid field message history.".into());
        }
        for m in roster.members() {
            for order in book
                .pending(m.slot)
                .into_iter()
                .chain(book.delivered(m.slot).map(|d| &d.order))
            {
                if let Directive::Objective(payload) = &order.directive {
                    payload.validate(roster)?;
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

pub(super) fn needs_poll(world: &mush_core::World, op: &super::model::OperationRecord) -> bool {
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
