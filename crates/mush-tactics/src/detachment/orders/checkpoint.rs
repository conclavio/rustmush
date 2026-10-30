//! Validated restart of the bounded message working set, never a command permit.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderCheckpoint<P> {
    schema: u16,
    detachment: DetachmentId,
    sequence: u64,
    last_time_ms: Option<u64>,
    slots: BTreeMap<Slot, SlotHistory<P>>,
}
impl<P: Clone> OrderBook<P> {
    #[must_use]
    pub fn checkpoint(&self) -> OrderCheckpoint<P> {
        OrderCheckpoint {
            schema: 1,
            detachment: self.detachment,
            sequence: self.sequence,
            last_time_ms: self.last_time_ms,
            slots: self.slots.clone(),
        }
    }
    /// The durable host also validates its objective type and actual owner
    /// permissions. Historical deliveries survive succession; pending messages
    /// still pass the ordinary current-term delivery checks before replacing them.
    pub fn restore(
        saved: &OrderCheckpoint<P>,
        roster: &DetachmentRoster,
    ) -> Result<Self, OrderError> {
        if saved.schema != 1
            || saved.detachment != roster.id()
            || saved.slots.len() > 6
            || saved.last_time_ms.is_none() && (saved.sequence != 0 || !saved.slots.is_empty())
        {
            return Err(OrderError::InvalidCheckpoint);
        }
        let now = saved.last_time_ms.unwrap_or(0);
        let valid_order = |order: &OrderEnvelope<P>| {
            order.detachment == saved.detachment
                && order.sequence > 0
                && order.sequence <= saved.sequence
                && order.roster_revision == roster.revision()
                && order.command_term > 0
                && order.command_term <= roster.term()
                && roster.member(order.recipient.slot) == Some(&order.recipient)
                && roster.members().iter().any(|m| {
                    m.unit == order.issuer_unit && m.controller.character() == Some(order.issuer)
                })
                && order.issued_at_ms <= now
                && order
                    .expires_at_ms
                    .is_none_or(|end| end > order.issued_at_ms)
        };
        for (slot, history) in &saved.slots {
            if roster.member(*slot).is_none()
                || history.pending.is_none() && history.delivered.is_none()
            {
                return Err(OrderError::InvalidCheckpoint);
            }
            if let Some(pending) = &history.pending
                && (!valid_order(pending) || pending.recipient.slot != *slot)
            {
                return Err(OrderError::InvalidCheckpoint);
            }
            if let Some(delivered) = &history.delivered {
                let order = &delivered.order;
                let observation_time = |at: u64| at >= delivered.delivered_at_ms && at <= now;
                if !valid_order(order)
                    || order.recipient.slot != *slot
                    || delivered.delivered_at_ms < order.issued_at_ms
                    || delivered.delivered_at_ms > now
                    || history
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.sequence <= order.sequence)
                    || delivered.acknowledgement.as_ref().is_some_and(|a| {
                        !observation_time(a.at_ms)
                            || order.recipient.controller.character() != Some(a.by)
                    })
                    || delivered
                        .attempted_at_ms
                        .is_some_and(|at| !observation_time(at))
                    || delivered
                        .completed_at_ms
                        .is_some_and(|at| !observation_time(at))
                    || matches!(order.directive, Directive::Cancel)
                        && (delivered.attempted_at_ms.is_some()
                            || delivered.completed_at_ms.is_some())
                {
                    return Err(OrderError::InvalidCheckpoint);
                }
            }
        }
        Ok(Self {
            detachment: saved.detachment,
            sequence: saved.sequence,
            last_time_ms: saved.last_time_ms,
            slots: saved.slots.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detachment::roster::{Connection, LeadershipPolicy, MemberFacts};
    #[test]
    fn restart_keeps_standing_delivery_and_rejects_forged_recipient_history()
    -> Result<(), Box<dyn std::error::Error>> {
        let slot = Slot::new(1)?;
        let mut roster = DetachmentRoster::new(
            DetachmentId(1),
            1,
            vec![Member {
                slot,
                unit: ObjId(11),
                controller: Controller::Player {
                    character: ObjId(1),
                },
            }],
            None,
            LeadershipPolicy::default(),
        )?;
        roster.reconcile(
            0,
            &[MemberFacts {
                slot,
                capable: true,
                controls_unit: true,
                unit_disabled: false,
                connection: Connection::Connected,
                command_authorized: true,
            }],
        )?;
        let mut book = OrderBook::new(roster.id());
        let issued = book.issue(
            &roster,
            1,
            OrderRequest {
                issuer: ObjId(1),
                roster_revision: 1,
                command_term: roster.term(),
                target: OrderTarget::All,
                directive: Directive::Objective("hold".to_string()),
                expires_at_ms: None,
            },
            |_| true,
        )?;
        book.deliver(&roster, slot, issued.sequence, 2, |_| true)?;
        book.respond(slot, issued.sequence, ObjId(1), Response::Wilco, 3)?;
        let mut saved: OrderCheckpoint<String> =
            serde_json::from_value(serde_json::to_value(book.checkpoint())?)?;
        let restored = OrderBook::restore(&saved, &roster)?;
        check_eq!(restored, book);
        check_eq!(
            restored.standing(&roster, slot, 1000).map(String::as_str),
            Some("hold")
        );
        let delivered = saved
            .slots
            .get_mut(&slot)
            .and_then(|h| h.delivered.as_mut())
            .ok_or("delivered")?;
        delivered.acknowledgement.as_mut().ok_or("ack")?.by = ObjId(2);
        check!(OrderBook::restore(&saved, &roster).is_err());
        Ok(())
    }
}
