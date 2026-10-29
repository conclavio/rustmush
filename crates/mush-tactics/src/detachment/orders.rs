//! DT.1 orders are messages and observations, never physical control inputs.
//!
//! The host validates/bounds the objective vocabulary, resolves communications,
//! supplies one monotonic mission clock and rechecks actual per-unit command
//! grants. Only a real delivery invokes `deliver`; issuing a message is not an
//! acknowledgement. This bounded working set is not the durable mission log:
//! adapters persist emitted facts at their own authoritative commit boundary.

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::Serialize;
use thiserror::Error;

use super::roster::{Controller, DetachmentId, DetachmentRoster, Member, RosterError, Slot};

/// Slots are resolved to their current units when an order is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OrderTarget {
    Slot(Slot),
    All,
}

/// Cancellation travels through the same authority and delivery fences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Directive<P> {
    Objective(P),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRequest<P> {
    pub issuer: ObjId,
    pub roster_revision: u64,
    pub command_term: u64,
    pub target: OrderTarget,
    pub directive: Directive<P>,
    /// None means a standing order, with no arena TTL imposed on it.
    pub expires_at_ms: Option<u64>,
}

/// A projection of an admitted message, not an importable authority token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OrderEnvelope<P> {
    pub detachment: DetachmentId,
    pub sequence: u64,
    pub issuer: ObjId,
    /// The admitted leader's actual unit; observer-scoped objectives must use
    /// this provenance rather than a cached or client-supplied command hull.
    pub issuer_unit: ObjId,
    pub roster_revision: u64,
    pub command_term: u64,
    pub recipient: Member,
    pub directive: Directive<P>,
    pub issued_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IssuedOrder {
    pub sequence: u64,
    pub recipients: Vec<Slot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Response {
    Wilco,
    Negative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Acknowledgement {
    pub by: ObjId,
    pub response: Response,
    pub at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ExecutionObservation {
    Attempted,
    Completed,
}

/// These fields record only what the host actually observed. Completion does
/// not invent an earlier attempt or an acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeliveredOrder<P> {
    pub order: OrderEnvelope<P>,
    pub delivered_at_ms: u64,
    pub acknowledgement: Option<Acknowledgement>,
    pub attempted_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    AlreadyDelivered,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OrderError {
    #[error("the order belongs to another detachment")]
    WrongDetachment,
    #[error(transparent)]
    Roster(#[from] RosterError),
    #[error("the target slot is not in this roster")]
    UnknownSlot,
    #[error("the requested set is empty")]
    EmptyTarget,
    #[error("command authority does not cover the target unit")]
    Unauthorized,
    #[error("the target's assignment has changed")]
    AssignmentChanged,
    #[error("the order has expired")]
    Expired,
    #[error("the order sequence is no longer current")]
    StaleSequence,
    #[error("there is no delivered order to respond to")]
    NotDelivered,
    #[error("the responding character is not this recipient")]
    WrongRecipient,
    #[error("cancellation has no objective to attempt or complete")]
    NoObjective,
    #[error("the mission clock moved backward")]
    TimeReversed,
    #[error("the order sequence is exhausted")]
    SequenceExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlotHistory<P> {
    pending: Option<OrderEnvelope<P>>,
    delivered: Option<DeliveredOrder<P>>,
}

impl<P> Default for SlotHistory<P> {
    fn default() -> Self {
        Self {
            pending: None,
            delivered: None,
        }
    }
}

/// At most one pending and one delivered message per current roster slot.
/// Fields are private and there is deliberately no unchecked deserialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderBook<P> {
    detachment: DetachmentId,
    sequence: u64,
    last_time_ms: Option<u64>,
    slots: BTreeMap<Slot, SlotHistory<P>>,
}

impl<P> OrderBook<P> {
    #[must_use]
    pub fn new(detachment: DetachmentId) -> Self {
        Self {
            detachment,
            sequence: 0,
            last_time_ms: None,
            slots: BTreeMap::new(),
        }
    }

    fn check_time(&self, now_ms: u64) -> Result<(), OrderError> {
        if self.last_time_ms.is_some_and(|last| now_ms < last) {
            Err(OrderError::TimeReversed)
        } else {
            Ok(())
        }
    }

    fn check_roster(&self, roster: &DetachmentRoster, now_ms: u64) -> Result<(), OrderError> {
        if roster.id() != self.detachment {
            return Err(OrderError::WrongDetachment);
        }
        if roster
            .last_reconciled_at_ms()
            .is_some_and(|last| now_ms < last)
        {
            return Err(OrderError::TimeReversed);
        }
        Ok(())
    }

    #[must_use]
    pub fn pending(&self, slot: Slot) -> Option<&OrderEnvelope<P>> {
        self.slots.get(&slot)?.pending.as_ref()
    }

    /// Private-to-recipient working history. A host must apply its observation
    /// and communication policy before showing it to another character.
    #[must_use]
    pub fn delivered(&self, slot: Slot) -> Option<&DeliveredOrder<P>> {
        self.slots.get(&slot)?.delivered.as_ref()
    }

    /// The objective actually delivered to this slot, including across a
    /// leadership change. An acknowledgement alone neither ends nor executes it.
    #[must_use]
    pub fn standing(&self, roster: &DetachmentRoster, slot: Slot, now_ms: u64) -> Option<&P> {
        let delivered = self.delivered(slot)?;
        if roster.id() != self.detachment
            || roster.member(slot) != Some(&delivered.order.recipient)
            || now_ms < delivered.delivered_at_ms
            || delivered.completed_at_ms.is_some()
            || delivered
                .order
                .expires_at_ms
                .is_some_and(|end| now_ms >= end)
        {
            return None;
        }
        match &delivered.order.directive {
            Directive::Objective(payload) => Some(payload),
            Directive::Cancel => None,
        }
    }

    /// Record a real character's response; this has no route to unit controls.
    pub fn respond(
        &mut self,
        slot: Slot,
        sequence: u64,
        character: ObjId,
        response: Response,
        now_ms: u64,
    ) -> Result<(), OrderError> {
        self.check_time(now_ms)?;
        let delivered = self.current_mut(slot, sequence)?;
        let recipient = match delivered.order.recipient.controller {
            Controller::Player { character } | Controller::Npc { character } => Some(character),
            Controller::Automated => None,
        };
        if recipient != Some(character) {
            return Err(OrderError::WrongRecipient);
        }
        delivered.acknowledgement = Some(Acknowledgement {
            by: character,
            response,
            at_ms: now_ms,
        });
        self.last_time_ms = Some(now_ms);
        Ok(())
    }

    /// Called only by a trusted host after an observed event, including actual
    /// player actions. This is not a player-command admission API: the host must
    /// authenticate a reporting character and verify the event first. It does
    /// not ask a player, NPC driver or controller to perform an action.
    pub fn observe_execution(
        &mut self,
        slot: Slot,
        sequence: u64,
        observation: ExecutionObservation,
        now_ms: u64,
    ) -> Result<(), OrderError> {
        self.check_time(now_ms)?;
        let delivered = self.current_mut(slot, sequence)?;
        if matches!(delivered.order.directive, Directive::Cancel) {
            return Err(OrderError::NoObjective);
        }
        match observation {
            ExecutionObservation::Attempted => {
                delivered.attempted_at_ms.get_or_insert(now_ms);
            }
            ExecutionObservation::Completed => {
                delivered.completed_at_ms.get_or_insert(now_ms);
            }
        }
        self.last_time_ms = Some(now_ms);
        Ok(())
    }

    fn current_mut(
        &mut self,
        slot: Slot,
        sequence: u64,
    ) -> Result<&mut DeliveredOrder<P>, OrderError> {
        let delivered = self
            .slots
            .get_mut(&slot)
            .and_then(|history| history.delivered.as_mut())
            .ok_or(OrderError::NotDelivered)?;
        if delivered.order.sequence != sequence {
            return Err(OrderError::StaleSequence);
        }
        Ok(delivered)
    }
}

impl<P: Clone> OrderBook<P> {
    /// Validate the complete target set before recording any message. The grant
    /// resolver must check the issuer/current-leader mission role against each
    /// unit; holding the leadership slot supplies no missing asset permission.
    pub fn issue(
        &mut self,
        roster: &DetachmentRoster,
        now_ms: u64,
        request: OrderRequest<P>,
        mut authorize: impl FnMut(&Member) -> bool,
    ) -> Result<IssuedOrder, OrderError> {
        self.check_time(now_ms)?;
        self.check_roster(roster, now_ms)?;
        roster.validate_command_at(
            request.issuer,
            request.roster_revision,
            request.command_term,
            now_ms,
        )?;
        let issuer_unit = roster
            .leader()
            .and_then(|leader| roster.member(leader.slot))
            .ok_or(OrderError::Unauthorized)?
            .unit;
        if request.expires_at_ms.is_some_and(|end| now_ms >= end) {
            return Err(OrderError::Expired);
        }
        let recipients = match request.target {
            OrderTarget::Slot(slot) => vec![roster.member(slot).ok_or(OrderError::UnknownSlot)?],
            OrderTarget::All => roster.members().iter().collect(),
        };
        if recipients.is_empty() {
            return Err(OrderError::EmptyTarget);
        }
        for recipient in &recipients {
            if !authorize(recipient) {
                return Err(OrderError::Unauthorized);
            }
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(OrderError::SequenceExhausted)?;
        // A changed roster cannot leave an unbounded trail of old slot keys.
        self.slots.retain(|slot, _| roster.member(*slot).is_some());
        let mut targets = Vec::with_capacity(recipients.len());
        for recipient in recipients {
            targets.push(recipient.slot);
            let history = self.slots.entry(recipient.slot).or_default();
            // Reusing a slot must not hand a new recipient the old unit's order.
            if history
                .delivered
                .as_ref()
                .is_some_and(|old| old.order.recipient != *recipient)
            {
                history.delivered = None;
            }
            history.pending = Some(OrderEnvelope {
                detachment: self.detachment,
                sequence,
                issuer: request.issuer,
                issuer_unit,
                roster_revision: request.roster_revision,
                command_term: request.command_term,
                recipient: *recipient,
                directive: request.directive.clone(),
                issued_at_ms: now_ms,
                expires_at_ms: request.expires_at_ms,
            });
        }
        self.sequence = sequence;
        self.last_time_ms = Some(now_ms);
        Ok(IssuedOrder {
            sequence,
            recipients: targets,
        })
    }

    /// Commit transport-confirmed delivery. Delayed obsolete terms/revisions,
    /// revoked grants and reordered messages leave the standing order intact.
    pub fn deliver(
        &mut self,
        roster: &DetachmentRoster,
        slot: Slot,
        sequence: u64,
        now_ms: u64,
        mut authorize: impl FnMut(&Member) -> bool,
    ) -> Result<DeliveryOutcome, OrderError> {
        self.check_time(now_ms)?;
        self.check_roster(roster, now_ms)?;
        let recipient = roster.member(slot).ok_or(OrderError::UnknownSlot)?;
        let history = self.slots.get(&slot).ok_or(OrderError::StaleSequence)?;
        if history.delivered.as_ref().is_some_and(|current| {
            current.order.sequence == sequence && current.order.recipient == *recipient
        }) {
            // A duplicate receipt is not a new admission or an order rewrite.
            return Ok(DeliveryOutcome::AlreadyDelivered);
        }
        let pending = history.pending.as_ref().ok_or(OrderError::StaleSequence)?;
        if pending.sequence != sequence {
            return Err(OrderError::StaleSequence);
        }
        roster.validate_command_at(
            pending.issuer,
            pending.roster_revision,
            pending.command_term,
            now_ms,
        )?;
        if pending.recipient != *recipient {
            return Err(OrderError::AssignmentChanged);
        }
        if pending.expires_at_ms.is_some_and(|end| now_ms >= end) {
            return Err(OrderError::Expired);
        }
        if !authorize(recipient) {
            return Err(OrderError::Unauthorized);
        }
        let delivered = DeliveredOrder {
            order: pending.clone(),
            delivered_at_ms: now_ms,
            acknowledgement: None,
            attempted_at_ms: None,
            completed_at_ms: None,
        };
        let history = self.slots.entry(slot).or_default();
        history.pending = None;
        history.delivered = Some(delivered);
        self.last_time_ms = Some(now_ms);
        Ok(DeliveryOutcome::Delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::super::roster::{Connection, LeadershipPolicy, MemberFacts};
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn fixture() -> Result<(DetachmentRoster, Vec<MemberFacts>), Box<dyn std::error::Error>> {
        let members = vec![
            Member {
                slot: Slot::new(1)?,
                unit: ObjId(101),
                controller: Controller::Player {
                    character: ObjId(1),
                },
            },
            Member {
                slot: Slot::new(2)?,
                unit: ObjId(102),
                controller: Controller::Player {
                    character: ObjId(2),
                },
            },
            Member {
                slot: Slot::new(3)?,
                unit: ObjId(103),
                controller: Controller::Npc {
                    character: ObjId(3),
                },
            },
            Member {
                slot: Slot::new(4)?,
                unit: ObjId(104),
                controller: Controller::Automated,
            },
        ];
        let facts: Vec<_> = members
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
            8,
            members,
            Some(ObjId(3)),
            LeadershipPolicy::default(),
        )?;
        roster.reconcile(0, &facts)?;
        Ok((roster, facts))
    }

    fn request(
        roster: &DetachmentRoster,
        target: OrderTarget,
        payload: &'static str,
    ) -> OrderRequest<&'static str> {
        OrderRequest {
            issuer: roster.leader().map_or(ObjId(0), |leader| leader.character),
            roster_revision: roster.revision(),
            command_term: roster.term(),
            target,
            directive: Directive::Objective(payload),
            expires_at_ms: None,
        }
    }

    #[test]
    fn all_admission_is_atomic_and_delivery_rechecks_grants() -> TestResult {
        let (roster, _) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let before = book.clone();
        check_eq!(
            book.issue(
                &roster,
                1,
                request(&roster, OrderTarget::All, "hold"),
                |member| member.slot.get() != 4
            ),
            Err(OrderError::Unauthorized)
        );
        check_eq!(book, before);
        let issued = book.issue(
            &roster,
            1,
            request(&roster, OrderTarget::All, "hold"),
            |_| true,
        )?;
        check_eq!(issued.recipients.len(), 4);
        let slot = Slot::new(4)?;
        check_eq!(book.standing(&roster, slot, 1), None);
        let before = book.clone();
        check_eq!(
            book.deliver(&roster, slot, issued.sequence, 2, |_| false),
            Err(OrderError::Unauthorized)
        );
        check_eq!(book, before);
        book.deliver(&roster, slot, issued.sequence, 2, |_| true)?;
        check_eq!(book.standing(&roster, slot, 10_000_000), Some(&"hold"));
        Ok(())
    }

    #[test]
    fn succession_preserves_delivered_orders_but_fences_delayed_messages() -> TestResult {
        let (mut roster, mut facts) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(4)?;
        let hold = book.issue(
            &roster,
            1,
            request(&roster, OrderTarget::All, "hold"),
            |_| true,
        )?;
        check_eq!(
            book.pending(slot).map(|order| order.issuer_unit),
            Some(ObjId(101))
        );
        book.deliver(&roster, slot, hold.sequence, 2, |_| true)?;
        let advance = book.issue(
            &roster,
            3,
            request(&roster, OrderTarget::All, "advance"),
            |_| true,
        )?;
        if let Some(first) = facts.first_mut() {
            first.unit_disabled = true;
        }
        roster.reconcile(4, &facts)?;
        check_eq!(book.standing(&roster, slot, 4), Some(&"hold"));
        let before = book.clone();
        check_eq!(
            book.deliver(&roster, slot, advance.sequence, 4, |_| true),
            Err(OrderError::Roster(RosterError::StaleTerm))
        );
        check_eq!(book, before);
        let withdraw = book.issue(
            &roster,
            5,
            request(&roster, OrderTarget::All, "withdraw"),
            |_| true,
        )?;
        check_eq!(
            book.pending(slot).map(|order| order.issuer_unit),
            Some(ObjId(102))
        );
        check_eq!(book.standing(&roster, slot, 5), Some(&"hold"));
        book.deliver(&roster, slot, withdraw.sequence, 6, |_| true)?;
        check_eq!(
            book.deliver(&roster, slot, advance.sequence, 7, |_| true),
            Err(OrderError::StaleSequence)
        );
        check_eq!(book.standing(&roster, slot, 7), Some(&"withdraw"));
        Ok(())
    }

    #[test]
    fn same_term_reordering_and_duplicate_receipts_cannot_reset_acknowledgements() -> TestResult {
        let (roster, _) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(2)?;
        let old = book.issue(
            &roster,
            1,
            request(&roster, OrderTarget::Slot(slot), "hold"),
            |_| true,
        )?;
        let new = book.issue(
            &roster,
            2,
            request(&roster, OrderTarget::Slot(slot), "advance"),
            |_| true,
        )?;
        check_eq!(
            book.deliver(&roster, slot, old.sequence, 3, |_| true),
            Err(OrderError::StaleSequence)
        );
        book.deliver(&roster, slot, new.sequence, 3, |_| true)?;
        book.respond(slot, new.sequence, ObjId(2), Response::Wilco, 4)?;
        let before = book.clone();
        check_eq!(
            book.deliver(&roster, slot, new.sequence, 5, |_| false),
            Ok(DeliveryOutcome::AlreadyDelivered)
        );
        check_eq!(book, before);
        Ok(())
    }

    #[test]
    fn human_responses_are_separate_from_controller_execution() -> TestResult {
        let (roster, _) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let human = Slot::new(2)?;
        let npc = Slot::new(3)?;
        let machine = Slot::new(4)?;
        let issued = book.issue(
            &roster,
            1,
            request(&roster, OrderTarget::All, "advance"),
            |_| true,
        )?;
        for slot in [human, npc, machine] {
            book.deliver(&roster, slot, issued.sequence, 2, |_| true)?;
        }
        check_eq!(
            book.respond(human, issued.sequence, ObjId(3), Response::Wilco, 3),
            Err(OrderError::WrongRecipient)
        );
        book.respond(human, issued.sequence, ObjId(2), Response::Wilco, 3)?;
        book.respond(human, issued.sequence, ObjId(2), Response::Negative, 4)?;
        let receipt = book.delivered(human).ok_or("human receipt missing")?;
        check_eq!(
            receipt.acknowledgement.map(|ack| ack.response),
            Some(Response::Negative)
        );
        check_eq!(receipt.attempted_at_ms, None);
        check_eq!(receipt.completed_at_ms, None);
        check_eq!(book.standing(&roster, human, 5), Some(&"advance"));
        // A separate trusted physical observation may complete a player's
        // objective; neither of the two acknowledgements above did so.
        book.observe_execution(human, issued.sequence, ExecutionObservation::Completed, 5)?;
        check_eq!(book.standing(&roster, human, 5), None);
        book.observe_execution(npc, issued.sequence, ExecutionObservation::Attempted, 5)?;
        book.observe_execution(npc, issued.sequence, ExecutionObservation::Completed, 6)?;
        check_eq!(book.standing(&roster, npc, 6), None);
        book.observe_execution(machine, issued.sequence, ExecutionObservation::Completed, 7)?;
        let receipt = book.delivered(machine).ok_or("machine receipt missing")?;
        check_eq!(receipt.acknowledgement, None);
        check_eq!(receipt.attempted_at_ms, None);
        check_eq!(receipt.completed_at_ms, Some(7));
        Ok(())
    }

    #[test]
    fn expiry_and_cancellation_have_explicit_delivery_semantics() -> TestResult {
        let (roster, _) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(4)?;
        let standing = book.issue(
            &roster,
            1,
            request(&roster, OrderTarget::Slot(slot), "hold"),
            |_| true,
        )?;
        book.deliver(&roster, slot, standing.sequence, 2, |_| true)?;
        let mut timed = request(&roster, OrderTarget::Slot(slot), "advance");
        timed.expires_at_ms = Some(10);
        let expired = book.issue(&roster, 3, timed.clone(), |_| true)?;
        check_eq!(
            book.deliver(&roster, slot, expired.sequence, 10, |_| true),
            Err(OrderError::Expired)
        );
        check_eq!(book.standing(&roster, slot, 10), Some(&"hold"));
        timed.expires_at_ms = Some(20);
        let timed = book.issue(&roster, 10, timed, |_| true)?;
        book.deliver(&roster, slot, timed.sequence, 11, |_| true)?;
        check_eq!(book.standing(&roster, slot, 19), Some(&"advance"));
        check_eq!(book.standing(&roster, slot, 20), None);
        let next = book.issue(
            &roster,
            20,
            request(&roster, OrderTarget::Slot(slot), "hold"),
            |_| true,
        )?;
        book.deliver(&roster, slot, next.sequence, 21, |_| true)?;
        let mut cancel = request(&roster, OrderTarget::Slot(slot), "unused");
        cancel.directive = Directive::Cancel;
        let cancel = book.issue(&roster, 22, cancel, |_| true)?;
        check_eq!(book.standing(&roster, slot, 22), Some(&"hold"));
        book.deliver(&roster, slot, cancel.sequence, 23, |_| true)?;
        check_eq!(book.standing(&roster, slot, 23), None);
        check_eq!(
            book.observe_execution(slot, cancel.sequence, ExecutionObservation::Attempted, 24),
            Err(OrderError::NoObjective)
        );
        Ok(())
    }

    #[test]
    fn wrong_roster_revisions_assignments_and_backward_time_are_refused() -> TestResult {
        let (roster, facts) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(4)?;
        let order = book.issue(
            &roster,
            10,
            request(&roster, OrderTarget::All, "hold"),
            |_| true,
        )?;
        let before = book.clone();
        check_eq!(
            book.deliver(&roster, slot, order.sequence, 9, |_| true),
            Err(OrderError::TimeReversed)
        );
        check_eq!(book, before);
        let mut other = DetachmentRoster::new(
            DetachmentId(8),
            roster.revision(),
            roster.members().to_vec(),
            Some(ObjId(3)),
            LeadershipPolicy::default(),
        )?;
        other.reconcile(10, &facts)?;
        check_eq!(
            book.deliver(&other, slot, order.sequence, 10, |_| true),
            Err(OrderError::WrongDetachment)
        );
        let mut revised = DetachmentRoster::new(
            roster.id(),
            roster.revision() + 1,
            roster.members().to_vec(),
            Some(ObjId(3)),
            LeadershipPolicy::default(),
        )?;
        revised.reconcile(10, &facts)?;
        check_eq!(
            book.deliver(&revised, slot, order.sequence, 10, |_| true),
            Err(OrderError::Roster(RosterError::StaleRevision))
        );
        book.deliver(&roster, slot, order.sequence, 10, |_| true)?;
        let mut members = roster.members().to_vec();
        if let Some(member) = members.iter_mut().find(|member| member.slot == slot) {
            member.unit = ObjId(999);
        }
        let mut replacement = DetachmentRoster::new(
            roster.id(),
            roster.revision() + 1,
            members,
            Some(ObjId(3)),
            LeadershipPolicy::default(),
        )?;
        replacement.reconcile(10, &facts)?;
        check_eq!(book.standing(&replacement, slot, 10), None);
        check_eq!(book.standing(&other, slot, 10), None);
        let mut invalid = request(&roster, OrderTarget::All, "advance");
        invalid.issuer = ObjId(2);
        check_eq!(
            book.issue(&roster, 11, invalid, |_| true),
            Err(OrderError::Roster(RosterError::NotLeader))
        );
        Ok(())
    }

    #[test]
    fn an_order_cannot_predate_the_roster_observation_admitting_it() -> TestResult {
        let (mut roster, facts) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(4)?;
        roster.reconcile(100, &facts)?;
        check_eq!(
            book.issue(
                &roster,
                99,
                request(&roster, OrderTarget::All, "hold"),
                |_| true
            ),
            Err(OrderError::TimeReversed)
        );
        let issued = book.issue(
            &roster,
            100,
            request(&roster, OrderTarget::All, "hold"),
            |_| true,
        )?;
        roster.reconcile(200, &facts)?;
        let before = book.clone();
        check_eq!(
            book.deliver(&roster, slot, issued.sequence, 199, |_| true),
            Err(OrderError::TimeReversed)
        );
        check_eq!(book, before);
        book.deliver(&roster, slot, issued.sequence, 200, |_| true)?;
        Ok(())
    }

    #[test]
    fn expired_linkdead_grace_requires_refresh_before_order_admission() -> TestResult {
        let (mut roster, mut facts) = fixture()?;
        let mut book = OrderBook::new(roster.id());
        let slot = Slot::new(4)?;
        if let Some(leader) = facts.first_mut() {
            leader.connection = Connection::Linkdead { grace_until_ms: 20 };
        }
        roster.reconcile(10, &facts)?;
        let issued = book.issue(
            &roster,
            11,
            request(&roster, OrderTarget::All, "hold"),
            |_| true,
        )?;
        let before = book.clone();
        check_eq!(
            book.issue(
                &roster,
                20,
                request(&roster, OrderTarget::All, "advance"),
                |_| true
            ),
            Err(OrderError::Roster(RosterError::RefreshRequired))
        );
        check_eq!(
            book.deliver(&roster, slot, issued.sequence, 20, |_| true),
            Err(OrderError::Roster(RosterError::RefreshRequired))
        );
        check_eq!(book, before);
        roster.reconcile(20, &facts)?;
        check_eq!(
            book.deliver(&roster, slot, issued.sequence, 20, |_| true),
            Err(OrderError::Roster(RosterError::StaleTerm))
        );
        Ok(())
    }
}
