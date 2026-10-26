//! Private, bounded preparation/worker receipts retained by the actual artifact
//! owner. A number from a request is only a lookup; it cannot mint any ticket.
//! The worker/queue keeps the noncloneable Machine separately until acknowledgement.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use mush_core::{AccountId, CommitToken, ObjId};
use mush_roll::commerce::rating::{
    RatingCompatibility, RatingSubject,
    nonconformance::MeasuredRegistryResult,
    protocol::{TicketId, Transition},
};

use super::{
    BulkError, invalid,
    model::{ArtifactBytes, Plan, RunId, Status, WorkKind},
    owner::Receipt,
};

const MAX_TICKETS: usize = 136; // configured maxima:128 queued +8 workers
const MAX_ENCODED_TICKET_BYTES: u64 = 64 * 1024 * 1024;

/// Built only by the actual Registry preparation worker from its captured
/// subject/policy/schedule, never deserialized from an action parameter.
pub(crate) struct AdmissionData {
    pub(crate) account: AccountId,
    pub(crate) character: ObjId,
    pub(crate) nonce: u64,
    pub(crate) subject: RatingSubject,
    pub(crate) compatibility: RatingCompatibility,
    pub(crate) plan: Plan,
    pub(crate) start: ArtifactBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    pub(super) job: CommitToken,
    pub(super) account: AccountId,
    pub(super) character: ObjId,
    pub(super) run: RunId,
}

pub(super) enum TicketKind {
    Admit(AdmissionData),
    Grant {
        binding: Binding,
        maximum_ms: u64,
    },
    Settle {
        binding: Binding,
        receipt: Receipt,
        artifact: Option<ArtifactBytes>,
        measured: Option<MeasuredRegistryResult>,
    },
    Cancel(Binding),
    /// Created only while boot owns the stopped worker service. Public cancel
    /// never mints this variant or assumes that a running worker has stopped.
    Recover(Binding),
}
pub(super) struct Ticket {
    pub(super) kind: TicketKind,
    bytes: u64,
}
impl Ticket {
    pub(super) fn reconformance(&self, store: &super::owner::BulkStore) -> Result<bool, BulkError> {
        let budget = match &self.kind {
            TicketKind::Admit(data) => data.plan.registry.as_ref(),
            _ => {
                let binding = self.binding().ok_or(BulkError::Missing)?;
                store
                    .run(binding.account, binding.character, binding.run)?
                    .plan
                    .registry
                    .as_ref()
            }
        }
        .ok_or(BulkError::Missing)?;
        Ok(budget.reconformance.is_some())
    }

    pub(super) fn transition(&self) -> Transition {
        match &self.kind {
            TicketKind::Admit(_) => Transition::Admit,
            TicketKind::Grant { .. } => Transition::Grant,
            TicketKind::Settle { .. } => Transition::Settle,
            TicketKind::Cancel(_) => Transition::Cancel,
            TicketKind::Recover(_) => Transition::Recover,
        }
    }
    pub(super) fn character(&self) -> ObjId {
        match &self.kind {
            TicketKind::Admit(data) => data.character,
            TicketKind::Grant { binding, .. } | TicketKind::Settle { binding, .. } => {
                binding.character
            }
            TicketKind::Cancel(binding) | TicketKind::Recover(binding) => binding.character,
        }
    }
    pub(super) fn account(&self) -> AccountId {
        match &self.kind {
            TicketKind::Admit(data) => data.account,
            TicketKind::Grant { binding, .. } | TicketKind::Settle { binding, .. } => {
                binding.account
            }
            TicketKind::Cancel(binding) | TicketKind::Recover(binding) => binding.account,
        }
    }
    pub(super) fn binding(&self) -> Option<Binding> {
        match &self.kind {
            TicketKind::Admit(_) => None,
            TicketKind::Grant { binding, .. } | TicketKind::Settle { binding, .. } => {
                Some(*binding)
            }
            TicketKind::Cancel(binding) | TicketKind::Recover(binding) => Some(*binding),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Acknowledged {
    pub(super) ticket: TicketId,
    pub(super) operation: CommitToken,
    pub(super) job: CommitToken,
    pub(super) transition: Transition,
}

#[derive(Clone, Default)]
pub(super) struct Tickets {
    next: u64,
    bytes: u64,
    rows: BTreeMap<u64, Arc<Ticket>>,
    acknowledged: VecDeque<Acknowledged>,
}
impl Tickets {
    pub(super) fn pristine(&self) -> bool {
        self.next == 0 && self.bytes == 0 && self.rows.is_empty() && self.acknowledged.is_empty()
    }
    fn insert(&mut self, kind: TicketKind, bytes: u64) -> Result<TicketId, BulkError> {
        let id = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("Registry ticket exhausted"))?;
        let retained = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("Registry ticket bytes"))?;
        if self.rows.len() + self.acknowledged.len() >= MAX_TICKETS
            || retained > MAX_ENCODED_TICKET_BYTES
        {
            return Err(BulkError::QueueFull);
        }
        let ticket = TicketId::new(id).map_err(|_| invalid("Registry ticket identity"))?;
        self.rows.insert(id, Arc::new(Ticket { kind, bytes }));
        self.next = id;
        self.bytes = retained;
        Ok(ticket)
    }
    pub(super) fn admit(&mut self, data: AdmissionData) -> Result<TicketId, BulkError> {
        data.plan.validate()?;
        let budget = data.plan.registry.as_ref().ok_or(BulkError::Missing)?;
        if data.plan.kind != WorkKind::Registry
            || data.character.0 == 0
            || data.nonce == 0
            || match &budget.reconformance {
                None => data.subject.revision().revision().key.character != data.character,
                Some(_) => data.character != mush_core::GOD || !budget.measured_rejection,
            }
            || data
                .subject
                .digest()
                .map_err(|_| invalid("Registry ticket subject"))?
                != budget.subject
            || data.compatibility.digest() != budget.compatibility
            || data.plan.start != *data.start.descriptor()
        {
            return Err(invalid(
                "Registry prepared ticket does not bind its real inputs",
            ));
        }
        let plan_bytes = serde_json::to_vec(&data.plan)
            .map_err(|_| invalid("Registry plan bytes"))?
            .len() as u64;
        let bytes = data
            .start
            .descriptor()
            .bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(plan_bytes))
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| invalid("Registry ticket byte bound"))?;
        self.insert(TicketKind::Admit(data), bytes)
    }
    pub(super) fn grant(
        &mut self,
        binding: Binding,
        maximum_ms: u64,
    ) -> Result<TicketId, BulkError> {
        if maximum_ms == 0 {
            return Err(invalid("Registry grant duration"));
        }
        self.insert(
            TicketKind::Grant {
                binding,
                maximum_ms,
            },
            4096,
        )
    }
    /// Called on the off-world receipt preparation path. It validates the exact
    /// raw worker bytes once before any bounded world publication can name them.
    #[cfg(test)]
    pub(super) fn settle(
        &mut self,
        binding: Binding,
        receipt: Receipt,
        artifact: Option<ArtifactBytes>,
        measured: Option<MeasuredRegistryResult>,
    ) -> Result<TicketId, BulkError> {
        self.insert_settlement(PreparedSettlement::prepare(
            binding, receipt, artifact, measured,
        )?)
        .map_err(|(error, _)| error)
    }
    // Queue refusal returns the exact measured completion, never a new receipt.
    #[allow(clippy::result_large_err)]
    pub(super) fn insert_settlement(
        &mut self,
        value: PreparedSettlement,
    ) -> Result<TicketId, (BulkError, PreparedSettlement)> {
        if self.rows.len() + self.acknowledged.len() >= MAX_TICKETS
            || self.next == u64::MAX
            || self
                .bytes
                .checked_add(value.bytes)
                .is_none_or(|n| n > MAX_ENCODED_TICKET_BYTES)
        {
            return Err((BulkError::QueueFull, value));
        }
        // The same exact checks as insert precede the move; this function is
        // single-owner synchronous and cannot fail after consuming the value.
        let id = self.next + 1;
        let ticket = match TicketId::new(id) {
            Ok(id) => id,
            Err(_) => return Err((invalid("Registry ticket identity"), value)),
        };
        self.rows.insert(
            id,
            Arc::new(Ticket {
                kind: value.kind,
                bytes: value.bytes,
            }),
        );
        self.next = id;
        self.bytes += value.bytes;
        Ok(ticket)
    }

    pub(super) fn cancel(&mut self, binding: Binding) -> Result<TicketId, BulkError> {
        self.insert(TicketKind::Cancel(binding), 4096)
    }
    pub(super) fn recover_stopped(
        &mut self,
        stopped: &super::registry_recovery::StoppedBinding,
    ) -> Result<TicketId, BulkError> {
        self.insert(TicketKind::Recover(stopped.binding()), 4096)
    }
    pub(super) fn selector(
        &self,
        actor: ObjId,
        transition: Transition,
        reconformance: bool,
        store: &super::owner::BulkStore,
    ) -> Result<Option<mush_roll::commerce::rating::protocol::RegistrySelector>, BulkError> {
        use mush_roll::commerce::rating::protocol::RegistrySelector;
        let mut selected = None;
        for (id, ticket) in &self.rows {
            if ticket.character() == actor
                && ticket.transition() == transition
                && ticket.reconformance(store)? == reconformance
            {
                selected = Some((id, ticket));
                break;
            }
        }
        let Some((id, ticket)) = selected else {
            return Ok(None);
        };
        let id = TicketId::new(*id).map_err(|_| invalid("Registry ticket identity"))?;
        Ok(Some(match &ticket.kind {
            TicketKind::Admit(data) if reconformance => RegistrySelector::reconformance(
                id,
                data.plan
                    .registry
                    .as_ref()
                    .and_then(|b| b.reconformance.as_ref())
                    .ok_or(BulkError::Missing)?
                    .filed(),
            ),
            TicketKind::Admit(data) => {
                RegistrySelector::admission(id, data.subject.revision().revision())
                    .map_err(|_| invalid("Registry selector"))?
            }
            _ => RegistrySelector::existing(id, ticket.binding().ok_or(BulkError::Missing)?.job),
        }))
    }
    pub(super) fn get(&self, id: TicketId) -> Result<Arc<Ticket>, BulkError> {
        self.rows.get(&id.get()).cloned().ok_or(BulkError::Missing)
    }
    pub(super) fn check(&self, id: TicketId, expected: &Arc<Ticket>) -> Result<(), BulkError> {
        if self
            .rows
            .get(&id.get())
            .is_some_and(|actual| Arc::ptr_eq(actual, expected))
        {
            Ok(())
        } else {
            Err(BulkError::Stale)
        }
    }
    /// The service calls this only after consuming the exact observed
    /// admission's NoIntent proof. Grant/settle/cancel/recovery custody has no
    /// equivalent discard: those tickets belong to an already admitted job.
    pub(super) fn discard_unadmitted(
        &mut self,
        id: TicketId,
        character: ObjId,
        account: AccountId,
    ) -> Result<(), BulkError> {
        let ticket = self.rows.get(&id.get()).ok_or(BulkError::Missing)?;
        if !matches!(ticket.kind, TicketKind::Admit(_))
            || ticket.character() != character
            || ticket.account() != account
            || self.acknowledged.iter().any(|ack| ack.ticket == id)
        {
            return Err(BulkError::Stale);
        }
        let retained = self
            .bytes
            .checked_sub(ticket.bytes)
            .ok_or_else(|| invalid("Registry ticket byte accounting"))?;
        self.rows.remove(&id.get());
        self.bytes = retained;
        Ok(())
    }
    /// Bind has checked exact Arc identity and capacity. No authority lookup,
    /// encoding or fallible operation remains in this owner image installation.
    pub(super) fn acknowledge_checked(&mut self, expected: &Arc<Ticket>, ack: Acknowledged) {
        self.rows.remove(&ack.ticket.get());
        self.bytes -= expected.bytes;
        self.acknowledged.push_back(ack);
    }
    pub(super) fn peek_acknowledged(&self) -> Option<Acknowledged> {
        self.acknowledged.front().copied()
    }
    pub(super) fn take_acknowledged(&mut self) -> Option<Acknowledged> {
        self.acknowledged.pop_front()
    }
}

/// Created on the actual executor thread, retained unchanged across publication
/// refusal. It contains the real receipt/raw result and no synthetic evidence.
pub(super) struct PreparedSettlement {
    kind: TicketKind,
    bytes: u64,
}
impl PreparedSettlement {
    // Refusal returns ownership of the exact worker bytes for bounded repair.
    #[allow(clippy::result_large_err)]
    pub(super) fn from_worker(
        binding: Binding,
        receipt: Receipt,
        artifact: Option<ArtifactBytes>,
    ) -> Result<Self, (BulkError, Option<ArtifactBytes>)> {
        let measured = match artifact
            .as_ref()
            .map(|bytes| {
                serde_json::from_slice::<MeasuredRegistryResult>(bytes.bytes())
                    .map_err(|_| invalid("Registry measured artifact codec"))
            })
            .transpose()
        {
            Ok(measured) => measured,
            Err(error) => return Err((error, artifact)),
        };
        let bytes = match Self::validate(binding, &receipt, &artifact, &measured) {
            Ok(bytes) => bytes,
            Err(error) => return Err((error, artifact)),
        };
        Ok(Self {
            kind: TicketKind::Settle {
                binding,
                receipt,
                artifact,
                measured,
            },
            bytes,
        })
    }
    #[cfg(test)]
    fn prepare(
        binding: Binding,
        receipt: Receipt,
        artifact: Option<ArtifactBytes>,
        measured: Option<MeasuredRegistryResult>,
    ) -> Result<Self, BulkError> {
        let bytes = Self::validate(binding, &receipt, &artifact, &measured)?;
        Ok(Self {
            kind: TicketKind::Settle {
                binding,
                receipt,
                artifact,
                measured,
            },
            bytes,
        })
    }
    fn validate(
        binding: Binding,
        receipt: &Receipt,
        artifact: &Option<ArtifactBytes>,
        measured: &Option<MeasuredRegistryResult>,
    ) -> Result<u64, BulkError> {
        if receipt.account() != binding.account
            || receipt.run() != binding.run
            || receipt.lease.kind() != WorkKind::Registry
        {
            return Err(invalid("Registry receipt ticket ownership"));
        }
        let bytes = match (artifact, measured, receipt.terminal) {
            (Some(bytes), Some(measured), Some(Status::Complete)) => {
                let measured_identity = measured
                    .artifact()
                    .map_err(|_| invalid("Registry measured artifact"))?;
                if receipt.result.as_ref() != Some(bytes.descriptor())
                    || measured_identity.sha256.as_bytes() != &bytes.descriptor().sha256
                    || measured_identity.bytes != bytes.descriptor().bytes
                {
                    return Err(invalid("Registry exact measured receipt bytes"));
                }
                bytes
                    .descriptor()
                    .bytes
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(4096))
                    .ok_or_else(|| invalid("Registry receipt ticket bytes"))?
            }
            (None, None, terminal)
                if terminal != Some(Status::Complete) && receipt.result.is_none() =>
            {
                4096
            }
            _ => return Err(invalid("Registry receipt ticket completeness")),
        };
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    fn binding() -> Result<Binding, Box<dyn std::error::Error>> {
        Ok(Binding {
            job: CommitToken::from_bytes([1; 16]),
            account: AccountId::mint("ticket-bound", 1)?,
            character: ObjId(7),
            run: RunId(1),
        })
    }
    #[test]
    fn bounded_ticket_and_unread_ack_capacity_refuse_without_consuming_an_id() -> TestResult {
        let mut tickets = Tickets::default();
        let binding = binding()?;
        let first = tickets.cancel(binding)?;
        let captured = tickets.get(first)?;
        for _ in 1..MAX_TICKETS {
            tickets.cancel(binding)?;
        }
        let previous = (tickets.next, tickets.bytes, tickets.rows.len());
        check_eq!(tickets.cancel(binding), Err(BulkError::QueueFull));
        check_eq!((tickets.next, tickets.bytes, tickets.rows.len()), previous);
        tickets.check(first, &captured)?;
        tickets.acknowledge_checked(
            &captured,
            Acknowledged {
                ticket: first,
                operation: CommitToken::from_bytes([2; 16]),
                job: binding.job,
                transition: Transition::Cancel,
            },
        );
        check_eq!(
            tickets.cancel(binding),
            Err(BulkError::QueueFull),
            "unread ack retains the slot"
        );
        tickets.take_acknowledged().ok_or("ack")?;
        let next = tickets.cancel(binding)?;
        check_eq!(next.get(), u64::try_from(MAX_TICKETS)? + 1);
        check!(tickets.check(first, &captured).is_err());
        Ok(())
    }
    #[test]
    fn encoded_ticket_budget_refuses_before_row_or_counter_mutation() -> TestResult {
        let mut tickets = Tickets::default();
        let binding = binding()?;
        tickets.insert(TicketKind::Cancel(binding), MAX_ENCODED_TICKET_BYTES)?;
        let previous = (tickets.next, tickets.bytes, tickets.rows.len());
        check_eq!(tickets.cancel(binding), Err(BulkError::QueueFull));
        check_eq!((tickets.next, tickets.bytes, tickets.rows.len()), previous);
        Ok(())
    }
}
