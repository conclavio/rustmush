//! Actual private worker/owner custody. Ticket numbers are lookups; no command
//! or decoded result can create admission data, a lease or a measured receipt.
use super::{
    BulkError, invalid,
    model::{ArtifactBytes, Plan, RunId, Status, WorkKind},
    owner::Receipt,
};
use mush_core::{AccountId, CommitToken, ObjId};
use mush_roll::commerce::conduct::{
    ConductSubject,
    protocol::{TicketId, Transition},
    records::ConductMeasurement,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

const MAX_TICKETS: usize = 136;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) struct AdmissionData {
    pub(crate) account: AccountId,
    pub(crate) character: ObjId,
    pub(crate) nonce: u64,
    pub(crate) subject: ConductSubject,
    pub(crate) plan: Plan,
    pub(crate) start: crate::minds::registry::conduct::history::PreparedStart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    pub(super) job: CommitToken,
    pub(super) account: AccountId,
    pub(super) character: ObjId,
    pub(super) run: RunId,
}
pub(super) enum TicketKind {
    Admit(Box<AdmissionData>),
    Grant { binding: Binding, maximum_ms: u64 },
    Settle(Box<SettlementData>),
    Cancel(Binding),
    Recover(Binding),
}
pub(super) struct SettlementData {
    pub(super) binding: Binding,
    pub(super) receipt: Receipt,
    pub(super) artifact: Option<ArtifactBytes>,
    pub(super) measurement: Option<ConductMeasurement>,
}
pub(super) struct Ticket {
    pub(super) kind: TicketKind,
    bytes: u64,
}
impl Ticket {
    pub(super) fn key(&self, id: TicketId) -> String {
        match &self.kind {
            TicketKind::Admit(data) => format!(
                "conduct/{}/{}/{}",
                data.account.canonical_hash().to_hex(),
                data.nonce,
                id.get()
            ),
            _ => format!(
                "conduct/{}/{}/{}",
                self.binding().map(|b| b.job.to_hex()).unwrap_or_default(),
                self.transition().action_id(),
                id.get()
            ),
        }
    }
    pub(super) fn transition(&self) -> Transition {
        match &self.kind {
            TicketKind::Admit(_) => Transition::Admit,
            TicketKind::Grant { .. } => Transition::Grant,
            TicketKind::Settle(_) => Transition::Settle,
            TicketKind::Cancel(_) => Transition::Cancel,
            TicketKind::Recover(_) => Transition::Recover,
        }
    }
    pub(super) fn binding(&self) -> Option<Binding> {
        match &self.kind {
            TicketKind::Admit(_) => None,
            TicketKind::Grant { binding, .. } => Some(*binding),
            TicketKind::Settle(data) => Some(data.binding),
            TicketKind::Cancel(b) | TicketKind::Recover(b) => Some(*b),
        }
    }
    pub(super) fn account(&self) -> AccountId {
        match &self.kind {
            TicketKind::Admit(a) => a.account,
            TicketKind::Settle(data) => data.binding.account,
            TicketKind::Grant { binding, .. }
            | TicketKind::Cancel(binding)
            | TicketKind::Recover(binding) => binding.account,
        }
    }
    pub(super) fn character(&self) -> ObjId {
        match &self.kind {
            TicketKind::Admit(a) => a.character,
            TicketKind::Settle(data) => data.binding.character,
            TicketKind::Grant { binding, .. }
            | TicketKind::Cancel(binding)
            | TicketKind::Recover(binding) => binding.character,
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
    pub(super) fn selector(
        &self,
        transition: Transition,
    ) -> Result<Option<mush_roll::commerce::conduct::protocol::ConductSelector>, BulkError> {
        let mut found = self.rows.iter().filter(|(_, ticket)| {
            ticket.character() == mush_core::GOD && ticket.transition() == transition
        });
        let Some((id, ticket)) = found.next() else {
            return Ok(None);
        };
        if found.next().is_some() {
            return Err(BulkError::Stale);
        }
        let id = TicketId::new(*id).map_err(|_| BulkError::Stale)?;
        Ok(Some(match &ticket.kind {
            TicketKind::Admit(data) => {
                mush_roll::commerce::conduct::protocol::ConductSelector::admission(
                    id,
                    data.subject.clone(),
                )
            }
            _ => mush_roll::commerce::conduct::protocol::ConductSelector::existing(
                id,
                ticket.binding().ok_or(BulkError::Stale)?.job,
            ),
        }))
    }
    fn room(&self, bytes: u64) -> Result<(), BulkError> {
        if self.next == u64::MAX
            || self.rows.len() + self.acknowledged.len() >= MAX_TICKETS
            || self.bytes.checked_add(bytes).is_none_or(|n| n > MAX_BYTES)
        {
            Err(BulkError::QueueFull)
        } else {
            Ok(())
        }
    }
    fn insert(&mut self, kind: TicketKind, bytes: u64) -> Result<TicketId, BulkError> {
        self.room(bytes)?;
        let id = TicketId::new(self.next + 1).map_err(|_| invalid("Conduct ticket identity"))?;
        self.rows.insert(id.get(), Arc::new(Ticket { kind, bytes }));
        self.next = id.get();
        self.bytes += bytes;
        Ok(id)
    }
    pub(super) fn admit(&mut self, data: AdmissionData) -> Result<TicketId, BulkError> {
        data.plan.validate()?;
        data.subject
            .validate()
            .map_err(|_| invalid("Conduct subject"))?;
        let budget = data.plan.conduct.as_ref().ok_or(BulkError::Missing)?;
        if data.plan.kind != WorkKind::Conduct
            || data.character != mush_core::GOD
            || data.nonce == 0
            || data
                .subject
                .worker_digest()
                .map_err(|_| invalid("Conduct subject"))?
                != budget.subject
            || data.plan.start != *data.start.bytes().descriptor()
        {
            return Err(invalid("Conduct actual admission differs"));
        }
        let size = data
            .start
            .bytes()
            .descriptor()
            .bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(32 * 1024))
            .ok_or_else(|| invalid("Conduct ticket bytes"))?;
        self.insert(TicketKind::Admit(Box::new(data)), size)
    }
    pub(super) fn grant(
        &mut self,
        binding: Binding,
        maximum_ms: u64,
    ) -> Result<TicketId, BulkError> {
        if maximum_ms == 0 {
            return Err(invalid("Conduct grant duration"));
        }
        self.insert(
            TicketKind::Grant {
                binding,
                maximum_ms,
            },
            4096,
        )
    }
    pub(super) fn cancel(&mut self, binding: Binding) -> Result<TicketId, BulkError> {
        self.insert(TicketKind::Cancel(binding), 4096)
    }
    /// Only boot's stopped-owner recovery path is allowed to supply this sealed
    /// value. There is no equivalent service command or wire constructor.
    pub(super) fn recover(&mut self, stopped: &StoppedBinding) -> Result<TicketId, BulkError> {
        self.insert(TicketKind::Recover(stopped.0), 4096)
    }
    #[allow(clippy::result_large_err)]
    pub(super) fn insert_settlement(
        &mut self,
        value: PreparedSettlement,
    ) -> Result<TicketId, (BulkError, PreparedSettlement)> {
        if let Err(error) = self.room(value.bytes) {
            return Err((error, value));
        }
        let id = match TicketId::new(self.next + 1) {
            Ok(id) => id,
            Err(_) => return Err((invalid("Conduct ticket identity"), value)),
        };
        self.rows.insert(
            id.get(),
            Arc::new(Ticket {
                kind: value.kind,
                bytes: value.bytes,
            }),
        );
        self.next = id.get();
        self.bytes += value.bytes;
        Ok(id)
    }
    pub(super) fn get(&self, id: TicketId) -> Result<Arc<Ticket>, BulkError> {
        self.rows.get(&id.get()).cloned().ok_or(BulkError::Missing)
    }
    pub(super) fn check(&self, id: TicketId, ticket: &Arc<Ticket>) -> Result<(), BulkError> {
        if self
            .rows
            .get(&id.get())
            .is_some_and(|a| Arc::ptr_eq(a, ticket))
        {
            Ok(())
        } else {
            Err(BulkError::Stale)
        }
    }
    pub(super) fn acknowledge_checked(&mut self, ticket: &Arc<Ticket>, ack: Acknowledged) {
        self.rows.remove(&ack.ticket.get());
        self.bytes -= ticket.bytes;
        self.acknowledged.push_back(ack);
    }
    pub(super) fn peek_acknowledged(&self) -> Option<Acknowledged> {
        self.acknowledged.front().copied()
    }
    pub(super) fn take_acknowledged(&mut self) -> Option<Acknowledged> {
        self.acknowledged.pop_front()
    }
}
/// Its constructor lives beside the real stopped service, added when that
/// service is wired. Holding a plain Binding never authorizes a recovery.
pub(super) struct StoppedBinding(Binding);
impl StoppedBinding {
    pub(super) fn capture(
        _: &super::world::ColdHydration,
        account: AccountId,
        run: &super::model::Run,
    ) -> Result<Self, BulkError> {
        if run.plan.kind != WorkKind::Conduct
            || run.status.terminal()
            || run.character != mush_core::GOD
        {
            return Err(BulkError::Stale);
        }
        Ok(Self(Binding {
            job: run.conduct_job.ok_or(BulkError::Missing)?,
            account,
            character: run.character,
            run: run.id,
        }))
    }
    pub(super) fn binding(&self) -> Binding {
        self.0
    }
}

pub(super) struct PreparedSettlement {
    kind: TicketKind,
    bytes: u64,
}
impl PreparedSettlement {
    /// Called only on the actual executor thread. The measurement comes from
    /// that private Machine's completed native run, never a JSON decoder.
    #[allow(clippy::result_large_err)]
    pub(super) fn from_worker(
        binding: Binding,
        receipt: Receipt,
        artifact: Option<ArtifactBytes>,
        measurement: Option<ConductMeasurement>,
    ) -> Result<Self, (BulkError, Option<ArtifactBytes>)> {
        let bytes = match Self::validate(binding, &receipt, &artifact, &measurement) {
            Ok(n) => n,
            Err(e) => return Err((e, artifact)),
        };
        Ok(Self {
            kind: TicketKind::Settle(Box::new(SettlementData {
                binding,
                receipt,
                artifact,
                measurement,
            })),
            bytes,
        })
    }
    fn validate(
        binding: Binding,
        receipt: &Receipt,
        artifact: &Option<ArtifactBytes>,
        measurement: &Option<ConductMeasurement>,
    ) -> Result<u64, BulkError> {
        if receipt.account() != binding.account
            || receipt.run() != binding.run
            || receipt.lease.kind() != WorkKind::Conduct
            || receipt.registry.is_some()
        {
            return Err(invalid("Conduct receipt ownership"));
        }
        match (artifact, measurement, receipt.terminal) {
            (Some(bytes), Some(measurement), Some(Status::Complete)) => {
                measurement
                    .validate()
                    .map_err(|_| invalid("Conduct actual measured result"))?;
                if receipt.result.as_ref() != Some(bytes.descriptor())
                    || bytes.descriptor().bytes > 256 * 1024
                    || receipt
                        .conduct
                        .is_none_or(|p| p.ending != super::conduct_budget::ConductEnding::Complete)
                {
                    return Err(invalid("Conduct actual artifact differs"));
                }
                bytes
                    .descriptor()
                    .bytes
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(4096))
                    .ok_or_else(|| invalid("Conduct ticket bytes"))
            }
            (None, None, terminal)
                if matches!(terminal, None | Some(Status::Failed | Status::Cancelled))
                    && receipt.result.is_none() =>
            {
                Ok(4096)
            }
            _ => Err(invalid("Conduct worker completion is incomplete")),
        }
    }
}
