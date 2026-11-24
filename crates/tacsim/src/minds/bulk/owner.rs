//! Per-account durable journal. Every mutation is fully serialized before
//! publication. A worker lease is released only by apply_checked, after the
//! world publisher has checked/committed this change and its artifact/ref ops.
use super::conduct_budget::{ConductCase, ConductCursor, ConductEnding, ConductProgress};
use super::registry_budget::{RegistryCase, RegistryCursor, RegistryEnding, RegistryProgress};
use super::{BulkError, invalid, model::*};
use mush_core::{AccountId, CommitToken, ObjId};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
#[path = "conduct_quota_tests.rs"]
mod conduct_quota_tests;
#[cfg(test)]
#[path = "registry_quota_tests.rs"]
mod registry_quota_tests;

#[derive(Clone, Debug)]
pub(crate) struct Lease {
    account: AccountId,
    run: RunId,
    sequence: u64,
    utc_day: i64,
    allowance: u64,
    units: u64,
    start: [u8; 32],
    kind: WorkKind,
    registry: Option<(RegistryCursor, RegistryCase)>,
    conduct: Option<(ConductCursor, ConductCase)>,
}
impl Lease {
    #[cfg(test)]
    pub(super) fn diagnostic_sequence(&self) -> u64 {
        self.sequence
    }
    pub(super) fn conduct(&self) -> Option<&(ConductCursor, ConductCase)> {
        self.conduct.as_ref()
    }
    pub(super) fn unchanged_conduct(&self) -> Option<ConductProgress> {
        self.conduct.as_ref().map(|(cursor, _)| ConductProgress {
            cursor: *cursor,
            ending: ConductEnding::Continue,
        })
    }
    pub(super) fn registry(&self) -> Option<&(RegistryCursor, RegistryCase)> {
        self.registry.as_ref()
    }
    pub(super) fn unchanged_registry(&self) -> Option<RegistryProgress> {
        self.registry.as_ref().map(|(cursor, _)| RegistryProgress {
            cursor: *cursor,
            ending: RegistryEnding::Continue,
        })
    }
    pub fn run(&self) -> RunId {
        self.run
    }
    pub(super) fn start(&self) -> [u8; 32] {
        self.start
    }
    pub(super) fn kind(&self) -> WorkKind {
        self.kind
    }
    pub(super) fn account(&self) -> AccountId {
        self.account
    }
    pub fn units(&self) -> u64 {
        self.units
    }
    pub fn maximum_sim_ms(&self) -> u64 {
        self.allowance / self.units
    }
    pub(super) fn utc_day(&self) -> i64 {
        self.utc_day
    }
}
/// A result from one exact lease, not a caller-supplied run/account identifier.
/// Production constructors are restricted to the executor; the action adapter
/// cannot mint an uncharged worker continuation.
#[derive(Clone)]
pub(crate) struct Receipt {
    pub(super) registry: Option<RegistryProgress>,
    pub(super) conduct: Option<ConductProgress>,
    pub(super) lease: Lease,
    pub(super) actual_unit_ms: u64,
    pub(super) result: Option<Artifact>,
    pub(super) terminal: Option<Status>,
}
impl Receipt {
    pub fn account(&self) -> AccountId {
        self.lease.account
    }
    pub fn run(&self) -> RunId {
        self.lease.run
    }
}
const MAX_INLINE_TERMINAL_REGISTRY: usize = 16;
const MAX_INLINE_TERMINAL_CONDUCT: usize = 4;

pub(crate) struct Prepared {
    account: Account,
    expected: u64,
    encoded: Vec<u8>,
    evicted: Vec<Run>,
    archived_conduct: Vec<(AccountId, Run)>,
    lease: Option<Lease>,
    registry_changed: bool,
    conduct_changed: bool,
}
impl Prepared {
    pub(super) fn changes_conduct(&self) -> bool {
        self.conduct_changed
    }
    pub(super) fn archived_conduct(&self) -> &[(AccountId, Run)] {
        &self.archived_conduct
    }
    pub(super) fn conduct_run(&self, job: CommitToken) -> Option<RunId> {
        self.account
            .runs
            .values()
            .find(|row| row.conduct_job == Some(job))
            .map(|row| row.id)
    }
    pub(super) fn changes_registry(&self) -> bool {
        self.registry_changed
    }
    pub fn account(&self) -> AccountId {
        self.account.id
    }
    #[cfg(test)]
    pub fn bytes(&self) -> &[u8] {
        &self.encoded
    }
    /// Release each evicted run's exact ProvingRun reference and delete its
    /// artifacts in the same prepared world publication as this document.
    pub fn evicted(&self) -> &[Run] {
        &self.evicted
    }
    pub fn document(&self) -> Result<serde_json::Value, BulkError> {
        serde_json::from_slice(&self.encoded).map_err(|e| invalid(e.to_string()))
    }
    pub fn resulting_run(&self, id: RunId) -> Option<&Run> {
        self.account.runs.get(&id)
    }
    pub(super) fn registry_run(&self, job: CommitToken) -> Option<RunId> {
        self.account
            .runs
            .values()
            .find(|row| row.registry_job == Some(job))
            .map(|row| row.id)
    }
}
#[derive(Clone)]
pub(crate) struct BulkStore {
    cfg: BulkConfig,
    generation: u64,
    accounts: BTreeMap<AccountId, Account>,
    /// Permanent terminal Conduct runs archived into per-run KV documents
    /// (`tacsim:conduct:run:<account_hash>:<run_id>`) so `Account` documents
    /// never grow beyond `MAX_ACCOUNT_DOCUMENT`.
    archived_conduct: BTreeMap<(AccountId, RunId), Run>,
    quarantined: BTreeSet<AccountId>,
    unreadable_quota_key: bool,
    // Volatile post-ack handoff only. The durable flight remains authoritative;
    // boot discards this queue and interrupts/reconciles that exact paid flight.
    registry_handoffs: BTreeMap<CommitToken, Lease>,
    conduct_handoffs: BTreeMap<CommitToken, Lease>,
}
impl BulkStore {
    pub fn new(cfg: BulkConfig) -> Result<Self, BulkError> {
        cfg.validate()?;
        Ok(Self {
            cfg,
            generation: 0,
            accounts: BTreeMap::new(),
            archived_conduct: BTreeMap::new(),
            quarantined: BTreeSet::new(),
            unreadable_quota_key: false,
            registry_handoffs: BTreeMap::new(),
            conduct_handoffs: BTreeMap::new(),
        })
    }
    pub fn account_ids(&self) -> Vec<AccountId> {
        self.accounts.keys().copied().collect()
    }
    pub(super) fn fresh_for_hydration(&self) -> bool {
        self.generation == 0
            && self.accounts.is_empty()
            && self.archived_conduct.is_empty()
            && self.quarantined.is_empty()
            && !self.unreadable_quota_key
            && self.registry_handoffs.is_empty()
            && self.conduct_handoffs.is_empty()
    }
    /// At most one active row per validated account. These are exact persisted
    /// bindings, not worker receipts or a reconstruction from a public job id.
    pub(super) fn stopped_registry_bindings(
        &self,
    ) -> impl Iterator<Item = super::registry_tickets::Binding> + '_ {
        self.accounts.values().filter_map(|account| {
            let run = account.runs.get(&account.active?)?;
            Some(super::registry_tickets::Binding {
                job: run.registry_job?,
                account: account.id,
                character: run.character,
                run: run.id,
            })
        })
    }
    /// One synchronous pass over the already hydrated account set. Only the
    /// nonterminal Registry projection is retained (at most 136 entries); raw
    /// account identities stay within this private quota owner.
    pub(super) fn audit_registry_boot(
        &self,
        executions: &[mush_roll::commerce::rating::execution::RatingExecution],
    ) -> Result<(), BulkError> {
        use mush_roll::commerce::rating::protocol::MAX_STOPPED_REGISTRY_JOBS;
        if self.unreadable_quota_key || !self.quarantined.is_empty() {
            return Err(BulkError::Quarantined);
        }
        if executions.len() > MAX_STOPPED_REGISTRY_JOBS {
            return Err(invalid("Registry boot typed inventory exceeds its bound"));
        }
        let mut actual = BTreeMap::new();
        for binding in self.stopped_registry_bindings() {
            if actual.len() == MAX_STOPPED_REGISTRY_JOBS {
                return Err(invalid("Registry boot private inventory exceeds its bound"));
            }
            if actual.insert(binding.job, binding).is_some() {
                return Err(invalid("Registry boot duplicate private job"));
            }
        }
        if actual.len() != executions.len() {
            return Err(invalid("Registry boot inventories differ"));
        }
        for execution in executions {
            if execution.observation().phase.terminal() {
                return Err(invalid("Registry boot requires nonterminal typed work"));
            }
            let binding = actual
                .remove(&execution.job())
                .ok_or_else(|| invalid("Registry boot typed job has no unique private row"))?;
            super::registry_projection::require_before(
                binding.account,
                binding.job,
                self.account_run(binding.account, binding.run)?,
                execution.admission(),
                execution.observation(),
            )?;
        }
        if !actual.is_empty() {
            return Err(invalid("Registry boot unmatched private work"));
        }
        Ok(())
    }

    /// Refusal-only detection among successfully decoded accounts. This does
    /// not certify completeness when an unrelated account is quarantined.
    pub(super) fn has_known_conduct(&self) -> bool {
        !self.archived_conduct.is_empty()
            || self
                .accounts
                .values()
                .any(|a| a.runs.values().any(|r| r.plan.kind == WorkKind::Conduct))
    }
    /// Complete permanent Conduct inventory, including terminal and archived
    /// rows. A missing account cannot be treated as an empty inventory after
    /// quarantine.
    pub(super) fn conduct_runs(
        &self,
    ) -> Result<impl Iterator<Item = (AccountId, &Run)>, BulkError> {
        if self.unreadable_quota_key || !self.quarantined.is_empty() {
            return Err(BulkError::Quarantined);
        }
        Ok(self
            .accounts
            .values()
            .flat_map(|account| {
                account
                    .runs
                    .values()
                    .filter(|run| run.plan.kind == WorkKind::Conduct)
                    .map(move |run| (account.id, run))
            })
            .chain(
                self.archived_conduct
                    .iter()
                    .map(|((account, _), run)| (*account, run)),
            ))
    }

    pub(super) fn active_count(&self) -> usize {
        self.accounts
            .values()
            .filter(|a| a.active.is_some())
            .count()
    }
    pub(super) fn active(&self, id: AccountId) -> Option<RunId> {
        self.accounts.get(&id).and_then(|a| a.active)
    }
    pub(super) fn account_run(&self, id: AccountId, run: RunId) -> Result<&Run, BulkError> {
        let account = self.account(id)?;
        account
            .runs
            .get(&run)
            .or_else(|| self.archived_conduct.get(&(id, run)))
            .ok_or(BulkError::Missing)
    }
    pub(super) fn archived_registry_runs(
        &self,
        account: AccountId,
        character: ObjId,
    ) -> Result<Vec<&ArchivedRegistryRun>, BulkError> {
        Ok(self
            .account(account)?
            .archived_registry
            .iter()
            .filter(|entry| entry.character == character)
            .collect())
    }
    pub(super) fn load_archived_conduct(
        &mut self,
        id: AccountId,
        run_id: RunId,
        bytes: &[u8],
    ) -> Result<(), BulkError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        let parsed = if bytes.len() <= MAX_CONDUCT_ARCHIVE_DOCUMENT {
            serde_json::from_slice::<ArchivedConductRun>(bytes).ok()
        } else {
            None
        };
        let valid = parsed.filter(|archived| {
            archived.account == id
                && archived.run.id == run_id
                && archived.validate().is_ok()
                && !self.quarantined.contains(&id)
                && self.accounts.get(&id).is_some_and(|account| {
                    !account.runs.contains_key(&run_id)
                        && run_id.0 < account.next_run
                        && archived.run.nonce <= account.request_watermark
                })
                && !self.archived_conduct.contains_key(&(id, run_id))
                && self.archived_conduct.len() < mush_roll::commerce::conduct::MAX_TRIALS
                && archived.run.conduct_job.is_some_and(|job| {
                    !self
                        .archived_conduct
                        .values()
                        .any(|row| row.conduct_job == Some(job))
                        && !self
                            .accounts
                            .values()
                            .flat_map(|a| a.runs.values())
                            .any(|row| row.conduct_job == Some(job))
                })
        });
        if let Some(archived) = valid {
            self.archived_conduct.insert((id, run_id), *archived.run);
            Ok(())
        } else {
            self.unreadable_quota_key = true;
            self.quarantined.insert(id);
            Err(BulkError::Quarantined)
        }
    }
    pub fn next_nonce(&self, id: AccountId) -> Result<u64, BulkError> {
        match self.account(id) {
            Ok(account) => account
                .request_watermark
                .checked_add(1)
                .ok_or_else(|| invalid("request sequence exhausted")),
            Err(BulkError::Missing) => Ok(1),
            Err(error) => Err(error),
        }
    }
    /// A malformed account key has no AccountId to place in `quarantined`.
    /// Retain that fact for the final Registry owner audit instead of treating
    /// a failed key decode as an absent/healthy document.
    pub(super) fn quarantine_unreadable_key(&mut self) {
        self.unreadable_quota_key = true;
    }

    /// The loader is given the authoritative account from the document key.
    /// Malformed/oversized/mismatched records keep that key quarantined.
    pub fn load(&mut self, id: AccountId, bytes: &[u8]) -> Result<(), BulkError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        let parsed = if bytes.len() <= MAX_ACCOUNT_DOCUMENT {
            serde_json::from_slice::<Account>(bytes).ok()
        } else {
            None
        };
        match parsed.filter(|a| a.id == id && a.validate(&self.cfg).is_ok()) {
            Some(account)
                if account
                    .registry_document_ceiling()
                    .is_ok_and(|n| n.is_none_or(|n| n <= MAX_ACCOUNT_DOCUMENT)) =>
            {
                self.accounts.insert(id, account);
                self.quarantined.remove(&id);
                Ok(())
            }
            Some(_) | None => {
                self.accounts.remove(&id);
                self.quarantined.insert(id);
                Err(BulkError::Quarantined)
            }
        }
    }
    fn account(&self, id: AccountId) -> Result<&Account, BulkError> {
        if self.quarantined.contains(&id) {
            Err(BulkError::Quarantined)
        } else {
            self.accounts.get(&id).ok_or(BulkError::Missing)
        }
    }
    fn clone_account(&self, id: AccountId, now: i64) -> Result<Account, BulkError> {
        if self.quarantined.contains(&id) {
            return Err(BulkError::Quarantined);
        }
        let mut account = self
            .accounts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Account::empty(id, utc_day(now)));
        let day = utc_day(now);
        if day < account.utc_day {
            return Err(invalid("UTC clock moved backwards"));
        }
        if day > account.utc_day {
            // A previously granted quantum must settle before advancing the
            // day. Its charge belongs to the day on which the worker started.
            if account.runs.values().any(|r| r.flight.is_some()) {
                return Err(BulkError::Busy);
            }
            account.utc_day = day;
            account.spent_unit_ms = 0;
            account.reserved_unit_ms = 0;
            if let Some(run) = account.active.and_then(|id| account.runs.get(&id)) {
                account.reserved_unit_ms = run
                    .plan
                    .maximum_unit_ms()
                    .saturating_sub(run.charged_unit_ms)
                    .min(self.cfg.daily_ms());
            }
        }
        Ok(account)
    }
    fn prepared(
        &self,
        account: Account,
        evicted: Vec<Run>,
        lease: Option<Lease>,
    ) -> Result<Prepared, BulkError> {
        self.prepared_with_archive(account, evicted, Vec::new(), lease)
    }
    fn prepared_with_archive(
        &self,
        account: Account,
        evicted: Vec<Run>,
        archived_conduct: Vec<(AccountId, Run)>,
        lease: Option<Lease>,
    ) -> Result<Prepared, BulkError> {
        account.validate(&self.cfg)?;
        let encoded = serde_json::to_vec(&account).map_err(|e| invalid(e.to_string()))?;
        if encoded.len() > MAX_ACCOUNT_DOCUMENT
            || account
                .registry_document_ceiling()?
                .is_some_and(|n| n > MAX_ACCOUNT_DOCUMENT)
        {
            return Err(invalid("account document bound"));
        }
        for (archived_account, run) in &archived_conduct {
            let doc = ArchivedConductRun {
                format: CONDUCT_ARCHIVE_FORMAT,
                account: *archived_account,
                run: Box::new(run.clone()),
            };
            doc.validate()?;
            let archive_bytes = serde_json::to_vec(&doc).map_err(|e| invalid(e.to_string()))?;
            if *archived_account != account.id || archive_bytes.len() > MAX_CONDUCT_ARCHIVE_DOCUMENT
            {
                return Err(invalid("archived Conduct document bound"));
            }
        }
        self.generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        let registry_rows = |account: &Account| {
            account
                .runs
                .iter()
                .filter(|(_, run)| run.plan.kind == WorkKind::Registry)
                .map(|(id, run)| (*id, run.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let before_account = self.accounts.get(&account.id);
        let before = before_account.map(registry_rows).unwrap_or_default();
        let before_archived_registry = before_account
            .map(|a| a.archived_registry.as_slice())
            .unwrap_or(&[]);
        let registry_changed = before != registry_rows(&account)
            || before_archived_registry != account.archived_registry.as_slice();
        let conduct_rows = |account: &Account| {
            account
                .runs
                .iter()
                .filter(|(_, run)| run.plan.kind == WorkKind::Conduct)
                .map(|(id, run)| (*id, run.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let conduct_changed = !archived_conduct.is_empty()
            || before_account.map(conduct_rows).unwrap_or_default() != conduct_rows(&account);
        Ok(Prepared {
            conduct_changed,
            registry_changed,
            account,
            expected: self.generation,
            encoded,
            evicted,
            archived_conduct,
            lease,
        })
    }
    pub fn check(&self, change: &Prepared) -> Result<(), BulkError> {
        if change.expected != self.generation {
            return Err(BulkError::Stale);
        }
        if self.quarantined.contains(&change.account.id) {
            return Err(BulkError::Quarantined);
        }
        Ok(())
    }
    pub(super) fn check_registry(
        &self,
        change: &Prepared,
        operation: CommitToken,
    ) -> Result<(), BulkError> {
        self.check(change)?;
        if !change.changes_registry() || change.changes_conduct() || !change.evicted.is_empty() {
            // Permanent Registry roots and old M6 retention have separate
            // explicit purge protocols; admission cannot silently evict either.
            return Err(invalid("Registry change requires exact retained rows"));
        }
        if change.lease.is_some()
            && (self.registry_handoffs.contains_key(&operation)
                || self.registry_handoffs.len() >= self.cfg.proving_workers)
        {
            return Err(BulkError::Busy);
        }
        Ok(())
    }
    pub(super) fn apply_registry_checked(&mut self, change: Prepared, operation: CommitToken) {
        if let Some(lease) = self.apply_checked(change) {
            self.registry_handoffs.insert(operation, lease);
        }
    }
    pub(super) fn check_conduct(
        &self,
        change: &Prepared,
        operation: CommitToken,
    ) -> Result<(), BulkError> {
        self.check(change)?;
        if !change.changes_conduct() || change.changes_registry() || !change.evicted.is_empty() {
            return Err(invalid("Conduct change requires exact retained rows"));
        }
        if self
            .archived_conduct
            .len()
            .saturating_add(change.archived_conduct.len())
            > mush_roll::commerce::conduct::MAX_TRIALS
            || change
                .archived_conduct
                .iter()
                .any(|(account, run)| self.archived_conduct.contains_key(&(*account, run.id)))
        {
            return Err(invalid("Conduct archive capacity or identity conflict"));
        }
        if change.lease.is_some()
            && (self.conduct_handoffs.contains_key(&operation)
                || self.conduct_handoffs.len() >= self.cfg.proving_workers)
        {
            return Err(BulkError::Busy);
        }
        Ok(())
    }
    pub(super) fn apply_conduct_checked(&mut self, change: Prepared, operation: CommitToken) {
        if let Some(lease) = self.apply_checked(change) {
            self.conduct_handoffs.insert(operation, lease);
        }
    }
    pub(super) fn take_conduct_lease(&mut self, operation: CommitToken) -> Option<Lease> {
        self.conduct_handoffs.remove(&operation)
    }
    pub(super) fn take_registry_lease(&mut self, operation: CommitToken) -> Option<Lease> {
        self.registry_handoffs.remove(&operation)
    }
    /// Call only after check and after every other participating owner has
    /// checked. No fallible work may follow the publisher's first mutation.
    pub fn apply_checked(&mut self, change: Prepared) -> Option<Lease> {
        self.generation = self.generation.saturating_add(1);
        for (account, run) in change.archived_conduct {
            self.archived_conduct.insert((account, run.id), run);
        }
        self.accounts.insert(change.account.id, change.account);
        change.lease
    }
    pub fn run(&self, account: AccountId, character: ObjId, id: RunId) -> Result<&Run, BulkError> {
        let row = self.account_run(account, id)?;
        if row.character == character {
            Ok(row)
        } else {
            Err(BulkError::Missing)
        }
    }
    pub fn runs(&self, account: AccountId, character: ObjId) -> Result<Vec<&Run>, BulkError> {
        Ok(self
            .account(account)?
            .runs
            .values()
            .filter(|r| r.character == character)
            .collect())
    }
    fn trim(
        &self,
        account: &mut Account,
        character: ObjId,
        extra_bytes: u64,
        extra_count: usize,
    ) -> Result<Vec<Run>, BulkError> {
        let mut removed = Vec::new();
        loop {
            // Proving retention counts and evicts strictly `WorkKind::Proving`
            // rows. Registry and Conduct have separate archival retention and
            // must never consume or deadlock Proving slots.
            let rows: Vec<_> = account
                .runs
                .values()
                .filter(|r| r.character == character && r.plan.kind == WorkKind::Proving)
                .collect();
            let count = rows.len() + extra_count;
            let bytes = rows
                .iter()
                .fold(extra_bytes, |n, r| n.saturating_add(r.retained_bytes()));
            if count <= self.cfg.completed_per_character && bytes <= self.cfg.bytes_per_character {
                return Ok(removed);
            }
            let oldest = rows
                .iter()
                .filter(|r| r.status.terminal() && r.pins == 0)
                .min_by_key(|r| (r.finished_ms, r.id))
                .map(|r| r.id)
                .ok_or(BulkError::Retention)?;
            if let Some(run) = account.runs.remove(&oldest) {
                removed.push(run)
            }
        }
    }
    fn archive_terminal_registry(account: &mut Account) -> Result<(), BulkError> {
        loop {
            let inline_terminal = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Registry && r.status.terminal() && r.pins == 0)
                .count();
            let over_ceiling = account
                .registry_document_ceiling()?
                .is_some_and(|n| n > MAX_ACCOUNT_DOCUMENT);
            if inline_terminal <= MAX_INLINE_TERMINAL_REGISTRY && !over_ceiling {
                return Ok(());
            }
            let Some(oldest) = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Registry && r.status.terminal() && r.pins == 0)
                .min_by_key(|r| (r.finished_ms, r.id))
                .map(|r| r.id)
            else {
                return Ok(());
            };
            if let Some(row) = account.runs.remove(&oldest)
                && let (Some(job), Some(finished_ms)) = (row.registry_job, row.finished_ms)
            {
                if account.archived_registry.len() >= MAX_ARCHIVED_REGISTRY {
                    account.archived_registry.remove(0);
                }
                account.archived_registry.push(ArchivedRegistryRun {
                    run: row.id,
                    character: row.character,
                    job,
                    finished_ms,
                });
            }
        }
    }
    fn archive_terminal_conduct(account: &mut Account) -> Result<Vec<(AccountId, Run)>, BulkError> {
        let mut archived = Vec::new();
        loop {
            let inline_terminal = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Conduct && r.status.terminal())
                .count();
            let over_ceiling = account
                .registry_document_ceiling()?
                .is_some_and(|n| n > MAX_ACCOUNT_DOCUMENT);
            if inline_terminal <= MAX_INLINE_TERMINAL_CONDUCT && !over_ceiling {
                return Ok(archived);
            }
            let Some(oldest) = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Conduct && r.status.terminal())
                .min_by_key(|r| (r.finished_ms, r.id))
                .map(|r| r.id)
            else {
                return Ok(archived);
            };
            if let Some(row) = account.runs.remove(&oldest) {
                archived.push((account.id, row));
            }
        }
    }
    /// Read-only admission preflight mirroring [`Self::prepare_admit_inner`] before
    /// submitting work to the shared `mind-bulk-prepare` thread. Rejects quarantined
    /// accounts, full queues, character caps, exhausted daily compute quota, and
    /// pinned proving retention saturation synchronously on the command path.
    pub(super) fn preflight_admit(
        &self,
        id: AccountId,
        character: ObjId,
        kind: WorkKind,
        initial_charge_ms: u64,
        min_proving_bytes: u64,
        now: i64,
    ) -> Result<(), BulkError> {
        let mut account = self.clone_account(id, now)?;
        if account.active.is_some() {
            return Err(BulkError::Busy);
        }
        if self
            .accounts
            .values()
            .filter(|a| a.active.is_some())
            .count()
            >= self.cfg.queued + self.cfg.proving_workers
        {
            return Err(BulkError::QueueFull);
        }
        if self
            .accounts
            .values()
            .flat_map(|a| a.runs.values())
            .filter(|r| r.status == Status::Queued)
            .count()
            >= self.cfg.queued
        {
            return Err(BulkError::QueueFull);
        }
        if account
            .runs
            .values()
            .map(|r| r.character)
            .collect::<BTreeSet<_>>()
            .len()
            >= MAX_CHARACTERS
            && !account.runs.values().any(|r| r.character == character)
        {
            return Err(BulkError::Retention);
        }
        let available = self.cfg.daily_ms().saturating_sub(account.spent_unit_ms);
        if available < initial_charge_ms {
            return Err(BulkError::Quota);
        }
        if kind == WorkKind::Proving {
            let old_benches = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Bench && r.status.terminal())
                .map(|r| r.id)
                .collect::<Vec<_>>();
            for old in old_benches {
                account.runs.remove(&old);
            }
            let _ = self.trim(&mut account, character, min_proving_bytes, 1)?;
        }
        Ok(())
    }
    pub fn prepare_admit(
        &self,
        id: AccountId,
        character: ObjId,
        nonce: u64,
        plan: Plan,
        now: i64,
    ) -> Result<(RunId, Prepared), BulkError> {
        if matches!(plan.kind, WorkKind::Registry | WorkKind::Conduct) {
            return Err(invalid(
                "Registry admission requires the registered producer",
            ));
        }
        self.prepare_admit_inner(id, character, nonce, plan, None, None, now)
    }
    pub(super) fn prepare_registry_admit(
        &self,
        id: AccountId,
        character: ObjId,
        nonce: u64,
        plan: Plan,
        job: CommitToken,
        now: i64,
    ) -> Result<(RunId, Prepared), BulkError> {
        if plan.kind != WorkKind::Registry {
            return Err(invalid("Registry admission kind"));
        }
        self.prepare_admit_inner(id, character, nonce, plan, Some(job), None, now)
    }
    pub(super) fn prepare_conduct_admit(
        &self,
        id: AccountId,
        character: ObjId,
        nonce: u64,
        plan: Plan,
        job: CommitToken,
        now: i64,
    ) -> Result<(RunId, Prepared), BulkError> {
        if plan.kind != WorkKind::Conduct {
            return Err(invalid("Conduct admission kind"));
        }
        self.prepare_admit_inner(id, character, nonce, plan, None, Some(job), now)
    }
    #[allow(clippy::too_many_arguments)] // exact immutable admission plus actual job identity
    fn prepare_admit_inner(
        &self,
        id: AccountId,
        character: ObjId,
        nonce: u64,
        plan: Plan,
        registry_job: Option<CommitToken>,
        conduct_job: Option<CommitToken>,
        now: i64,
    ) -> Result<(RunId, Prepared), BulkError> {
        plan.validate()?;
        if nonce == 0 {
            return Err(invalid("zero request nonce"));
        }
        let mut account = self.clone_account(id, now)?;
        if nonce <= account.request_watermark {
            return Err(BulkError::Consumed);
        }
        if account.active.is_some() {
            return Err(BulkError::Busy);
        }
        if self
            .accounts
            .values()
            .filter(|a| a.active.is_some())
            .count()
            >= self.cfg.queued + self.cfg.proving_workers
        {
            return Err(BulkError::QueueFull);
        }
        if self
            .accounts
            .values()
            .flat_map(|a| a.runs.values())
            .filter(|r| r.status == Status::Queued)
            .count()
            >= self.cfg.queued
        {
            return Err(BulkError::QueueFull);
        }
        if account
            .runs
            .values()
            .map(|r| r.character)
            .collect::<BTreeSet<_>>()
            .len()
            >= MAX_CHARACTERS
            && !account.runs.values().any(|r| r.character == character)
        {
            return Err(BulkError::Retention);
        }
        let available = self.cfg.daily_ms().saturating_sub(account.spent_unit_ms);
        let (initial_units, initial_native) = plan.initial_charge()?;
        if available < initial_units * u64::from(initial_native) {
            return Err(BulkError::Quota);
        }
        let mut evicted = Vec::new();
        if matches!(plan.kind, WorkKind::Proving | WorkKind::Bench) {
            let old_benches = account
                .runs
                .values()
                .filter(|r| r.plan.kind == WorkKind::Bench && r.status.terminal())
                .map(|r| r.id)
                .collect::<Vec<_>>();
            evicted.extend(
                old_benches
                    .into_iter()
                    .filter_map(|id| account.runs.remove(&id)),
            );
            if plan.kind == WorkKind::Proving {
                evicted.extend(self.trim(
                    &mut account,
                    character,
                    plan.start.bytes + plan.result_reservation_bytes,
                    1,
                )?);
            }
        }
        let kind = plan.kind;
        let run = RunId(account.next_run);
        account.next_run = account
            .next_run
            .checked_add(1)
            .ok_or_else(|| invalid("run allocator exhausted"))?;
        account.request_watermark = nonce;
        account.reserved_unit_ms = plan.maximum_unit_ms().min(available);
        account.active = Some(run);
        account.runs.insert(
            run,
            Run {
                registry_job,
                conduct_job,
                conduct: plan.conduct.as_ref().map(|_| ConductCursor::default()),
                registry: plan.registry.as_ref().map(|_| RegistryCursor::default()),
                id: run,
                character,
                nonce,
                created_ms: now,
                finished_ms: None,
                plan,
                status: Status::Queued,
                charged_unit_ms: 0,
                next_sequence: 1,
                flight: None,
                result: None,
                pins: 0,
            },
        );
        let archived_conduct = match kind {
            WorkKind::Registry => {
                Self::archive_terminal_registry(&mut account)?;
                Vec::new()
            }
            WorkKind::Conduct => Self::archive_terminal_conduct(&mut account)?,
            WorkKind::Proving | WorkKind::Bench => Vec::new(),
        };
        Ok((
            run,
            self.prepared_with_archive(account, evicted, archived_conduct, None)?,
        ))
    }
    pub(super) fn workers_busy(&self) -> bool {
        self.accounts
            .values()
            .flat_map(|a| a.runs.values())
            .filter(|r| r.flight.is_some())
            .count()
            >= self.cfg.proving_workers
    }
    /// Read-only eligibility check mirroring [`Self::prepare_grant`] without
    /// cloning or mutating account state. Used by the shared bulk drain loop so
    /// ungrantable entries rotate to the tail instead of stalling global turns.
    pub(super) fn can_grant(
        &self,
        id: AccountId,
        run: RunId,
        now: i64,
        sim_ms: u64,
    ) -> Result<(), BulkError> {
        if self.workers_busy() {
            return Err(BulkError::Busy);
        }
        let account = self.account(id)?;
        let day = utc_day(now);
        if day < account.utc_day {
            return Err(invalid("UTC clock moved backwards"));
        }
        if account.active != Some(run) {
            return Err(BulkError::Missing);
        }
        let row = account.runs.get(&run).ok_or(BulkError::Missing)?;
        if day > account.utc_day && account.runs.values().any(|r| r.flight.is_some()) {
            return Err(BulkError::Busy);
        }
        let (units, native, case_remaining) = row.charge()?;
        if sim_ms == 0 || sim_ms > u64::from(native) * LEASE_STEPS {
            return Err(invalid("quantum exceeds its frozen work-kind bound"));
        }
        if row.flight.is_some() || row.status == Status::CancelRequested {
            return Err(BulkError::Busy);
        }
        let (spent_unit_ms, reserved_unit_ms) = if day > account.utc_day {
            (
                0_u64,
                row.plan
                    .maximum_unit_ms()
                    .saturating_sub(row.charged_unit_ms)
                    .min(self.cfg.daily_ms()),
            )
        } else {
            (account.spent_unit_ms, account.reserved_unit_ms)
        };
        let available = reserved_unit_ms
            .min(self.cfg.daily_ms().saturating_sub(spent_unit_ms))
            .min(
                row.plan
                    .maximum_unit_ms()
                    .saturating_sub(row.charged_unit_ms),
            )
            / units;
        let sim_ms = sim_ms.min(available).min(case_remaining);
        if sim_ms < u64::from(native) {
            return Err(BulkError::Quota);
        }
        let cost = sim_ms
            .checked_mul(units)
            .ok_or_else(|| invalid("cost overflow"))?;
        if cost > reserved_unit_ms
            || spent_unit_ms.saturating_add(cost) > self.cfg.daily_ms()
            || cost
                > row
                    .plan
                    .maximum_unit_ms()
                    .saturating_sub(row.charged_unit_ms)
        {
            return Err(BulkError::Quota);
        }
        Ok(())
    }
    pub fn prepare_grant(
        &self,
        id: AccountId,
        run: RunId,
        now: i64,
        sim_ms: u64,
    ) -> Result<Prepared, BulkError> {
        if self
            .accounts
            .values()
            .flat_map(|a| a.runs.values())
            .filter(|r| r.flight.is_some())
            .count()
            >= self.cfg.proving_workers
        {
            return Err(BulkError::Busy);
        }
        let mut account = self.clone_account(id, now)?;
        if account.active != Some(run) {
            return Err(BulkError::Missing);
        }
        let row = account.runs.get_mut(&run).ok_or(BulkError::Missing)?;
        let (units, native, case_remaining) = row.charge()?;
        if sim_ms == 0 || sim_ms > u64::from(native) * LEASE_STEPS {
            return Err(invalid("quantum exceeds its frozen work-kind bound"));
        }
        if row.flight.is_some() || row.status == Status::CancelRequested {
            return Err(BulkError::Busy);
        }
        // The last grant can be shorter than the normal lease. Only complete
        // captured native steps execute; zero-time phases reconcile to zero.
        let available = account
            .reserved_unit_ms
            .min(self.cfg.daily_ms().saturating_sub(account.spent_unit_ms))
            .min(
                row.plan
                    .maximum_unit_ms()
                    .saturating_sub(row.charged_unit_ms),
            )
            / units;
        let sim_ms = sim_ms.min(available).min(case_remaining);
        if sim_ms < u64::from(native) {
            return Err(BulkError::Quota);
        }
        let cost = sim_ms
            .checked_mul(units)
            .ok_or_else(|| invalid("cost overflow"))?;
        if cost > account.reserved_unit_ms
            || account.spent_unit_ms.saturating_add(cost) > self.cfg.daily_ms()
            || cost
                > row
                    .plan
                    .maximum_unit_ms()
                    .saturating_sub(row.charged_unit_ms)
        {
            return Err(BulkError::Quota);
        }
        let sequence = row.next_sequence;
        row.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| invalid("step sequence exhausted"))?;
        account.reserved_unit_ms -= cost;
        account.spent_unit_ms += cost;
        row.charged_unit_ms += cost;
        row.flight = Some(Flight {
            registry: row.registry,
            conduct: row.conduct,
            sequence,
            utc_day: account.utc_day,
            allowance_unit_ms: cost,
        });
        row.status = Status::Running;
        let lease = Lease {
            account: id,
            run,
            sequence,
            utc_day: account.utc_day,
            allowance: cost,
            units,
            start: row.plan.start.sha256,
            kind: row.plan.kind,
            conduct: match (&row.plan.conduct, row.conduct) {
                (Some(budget), Some(cursor)) => Some((cursor, budget.case(cursor)?.clone())),
                (None, None) => None,
                _ => return Err(invalid("Conduct lease/cursor mismatch")),
            },
            registry: match (&row.plan.registry, row.registry) {
                (Some(budget), Some(cursor)) => Some((cursor, budget.case(cursor)?.clone())),
                (None, None) => None,
                _ => return Err(invalid("Registry lease/cursor mismatch")),
            },
        };
        self.prepared(account, Vec::new(), Some(lease))
    }
    pub fn prepare_finish(&self, receipt: Receipt, now: i64) -> Result<Prepared, BulkError> {
        let lease = receipt.lease;
        let mut account = self.account(lease.account)?.clone();
        if account.active != Some(lease.run) {
            return Err(BulkError::Stale);
        }
        let row = account.runs.get_mut(&lease.run).ok_or(BulkError::Missing)?;
        let expected = Flight {
            registry: lease.registry.as_ref().map(|(cursor, _)| *cursor),
            conduct: lease.conduct.as_ref().map(|(cursor, _)| *cursor),
            sequence: lease.sequence,
            utc_day: lease.utc_day,
            allowance_unit_ms: lease.allowance,
        };
        if row.flight.as_ref() != Some(&expected) || account.utc_day != lease.utc_day {
            return Err(BulkError::Stale);
        }
        let (units, _, _) = row.charge()?;
        if units != lease.units
            || lease.kind != row.plan.kind
            || lease.start != row.plan.start.sha256
        {
            return Err(BulkError::Stale);
        }
        if matches!(row.plan.kind, WorkKind::Registry | WorkKind::Conduct)
            && receipt.result.is_some()
            && receipt.terminal != Some(Status::Complete)
        {
            return Err(invalid("Registry result before complete certification"));
        }
        if receipt.actual_unit_ms > lease.allowance {
            return Err(invalid("worker exceeded its grant"));
        }
        // Validate the complete exact post-cursor before changing even the
        // staged refund. A failed/unknown worker charges its bounded lease and
        // terminates without inventing known simulated progress.
        let next_cursor = match (
            &row.plan.registry,
            row.registry,
            &lease.registry,
            receipt.registry,
        ) {
            (Some(budget), Some(cursor), Some((captured, case)), Some(progress)) => {
                if cursor != *captured
                    || budget.case(cursor)? != case
                    || !receipt.actual_unit_ms.is_multiple_of(units)
                {
                    return Err(BulkError::Stale);
                }
                if matches!(
                    progress.ending,
                    RegistryEnding::Certified | RegistryEnding::Rejected
                ) != (receipt.terminal == Some(Status::Complete))
                {
                    return Err(invalid("Registry certification/terminal mismatch"));
                }
                let actual =
                    budget.advance(cursor, receipt.actual_unit_ms / units, progress.ending)?;
                if actual != progress.cursor {
                    return Err(invalid("Registry post-cursor mismatch"));
                }
                Some(actual)
            }
            (Some(_), Some(cursor), Some(_), None)
                if matches!(
                    receipt.terminal,
                    Some(Status::Failed | Status::Cancelled | Status::Interrupted)
                ) && receipt.actual_unit_ms == lease.allowance =>
            {
                Some(cursor)
            }
            (None, None, None, None) => None,
            _ => return Err(invalid("Registry receipt/cursor mismatch")),
        };
        let conduct_cursor = match (
            &row.plan.conduct,
            row.conduct,
            &lease.conduct,
            receipt.conduct,
        ) {
            (Some(budget), Some(cursor), Some((captured, case)), Some(progress)) => {
                if cursor != *captured
                    || budget.case(cursor)? != case
                    || !receipt.actual_unit_ms.is_multiple_of(units)
                {
                    return Err(BulkError::Stale);
                }
                if (progress.ending == ConductEnding::Complete)
                    != (receipt.terminal == Some(Status::Complete))
                {
                    return Err(invalid("Conduct completion/terminal mismatch"));
                }
                let actual =
                    budget.advance(cursor, receipt.actual_unit_ms / units, progress.ending)?;
                if actual != progress.cursor {
                    return Err(invalid("Conduct post-cursor mismatch"));
                }
                Some(actual)
            }
            (Some(_), Some(cursor), Some(_), None)
                if matches!(
                    receipt.terminal,
                    Some(Status::Failed | Status::Cancelled | Status::Interrupted)
                ) && receipt.actual_unit_ms == lease.allowance =>
            {
                Some(cursor)
            }
            (None, None, None, None) => None,
            _ => return Err(invalid("Conduct receipt/cursor mismatch")),
        };
        let refund = lease.allowance - receipt.actual_unit_ms;
        account.spent_unit_ms -= refund;
        account.reserved_unit_ms += refund;
        row.charged_unit_ms -= refund;
        row.flight = None;
        row.registry = next_cursor;
        row.conduct = conduct_cursor;
        if let Some(result) = receipt.result {
            result.validate()?;
            if result.bytes > row.plan.result_reservation_bytes {
                return Err(invalid("result exceeded reservation"));
            }
            // A Registry cancellation wins over a returned certificate. Keep
            // the exact measured charge/cursor, but no successful artifact may
            // survive under a Cancelled execution or reach typed publication.
            if !matches!(row.plan.kind, WorkKind::Registry | WorkKind::Conduct)
                || row.status != Status::CancelRequested
            {
                row.result = Some(result);
            }
        }
        let terminal = if row.status == Status::CancelRequested {
            Some(Status::Cancelled)
        } else {
            receipt.terminal
        };
        if let Some(status) = terminal {
            if !status.terminal()
                || (status == Status::Complete
                    && row.plan.kind != WorkKind::Bench
                    && row.result.is_none())
            {
                return Err(invalid("terminal result state"));
            }
            row.status = status;
            row.finished_ms = Some(now);
            account.active = None;
            account.reserved_unit_ms = 0;
        } else {
            row.status = Status::Running
        }
        self.prepared(account, Vec::new(), None)
    }
    pub fn prepare_cancel(
        &self,
        id: AccountId,
        character: ObjId,
        run: RunId,
        now: i64,
    ) -> Result<Prepared, BulkError> {
        self.run(id, character, run)?;
        let mut account = self.account(id)?.clone();
        let row = account.runs.get_mut(&run).ok_or(BulkError::Missing)?;
        if row.status.terminal() {
            return Err(BulkError::Consumed);
        }
        if row.flight.is_some() {
            row.status = Status::CancelRequested
        } else {
            row.status = Status::Cancelled;
            row.finished_ms = Some(now);
            account.active = None;
            account.reserved_unit_ms = 0
        }
        self.prepared(account, Vec::new(), None)
    }
    /// Persist before enabling admission after boot. Never infer zero work for
    /// an in-flight lease: at most one bounded quantum is conservatively charged.
    pub fn prepare_recover(&self, id: AccountId, now: i64) -> Result<Prepared, BulkError> {
        let mut account = self.account(id)?.clone();
        for row in account.runs.values_mut() {
            row.pins = 0;
            if !row.status.terminal() {
                row.flight = None;
                row.status = Status::Interrupted;
                row.finished_ms = Some(now)
            }
        }
        account.active = None;
        account.reserved_unit_ms = 0;
        self.prepared(account, Vec::new(), None)
    }
    /// Conduct recovery changes only its actual active row. Historical Registry
    /// or unrelated pinned rows in the shared account remain byte-identical.
    pub(super) fn prepare_conduct_recover(
        &self,
        binding: super::conduct_tickets::Binding,
        now: i64,
    ) -> Result<Prepared, BulkError> {
        let before = self.run(binding.account, binding.character, binding.run)?;
        if before.plan.kind != WorkKind::Conduct
            || before.conduct_job != Some(binding.job)
            || before.status.terminal()
        {
            return Err(BulkError::Stale);
        }
        let mut account = self.account(binding.account)?.clone();
        if account.active != Some(binding.run) {
            return Err(BulkError::Stale);
        }
        let row = account
            .runs
            .get_mut(&binding.run)
            .ok_or(BulkError::Missing)?;
        row.flight = None;
        row.status = Status::Interrupted;
        row.finished_ms = Some(now);
        account.active = None;
        account.reserved_unit_ms = 0;
        self.prepared(account, Vec::new(), None)
    }
    /// Legacy proving/bench recovery may share an account with Registry history,
    /// but never changes a Registry row or its active reservation. Those owners
    /// recover together only through the registered typed Recover transaction.
    pub(super) fn prepare_legacy_recover(
        &self,
        id: AccountId,
        now: i64,
    ) -> Result<Prepared, BulkError> {
        let mut account = self.account(id)?.clone();
        for row in account
            .runs
            .values_mut()
            .filter(|row| !matches!(row.plan.kind, WorkKind::Registry | WorkKind::Conduct))
        {
            row.pins = 0;
            if !row.status.terminal() {
                row.flight = None;
                row.status = Status::Interrupted;
                row.finished_ms = Some(now);
                account.active = None;
                account.reserved_unit_ms = 0;
            }
        }
        self.prepared(account, Vec::new(), None)
    }
    pub fn prepare_pin(
        &self,
        id: AccountId,
        character: ObjId,
        run: RunId,
        pin: bool,
    ) -> Result<Prepared, BulkError> {
        self.run(id, character, run)?;
        let mut account = self.account(id)?.clone();
        let row = account.runs.get_mut(&run).ok_or(BulkError::Missing)?;
        if !row.status.terminal() {
            return Err(BulkError::Busy);
        }
        row.pins = u8::from(pin);
        self.prepared(account, Vec::new(), None)
    }
    pub fn prepare_forget(
        &self,
        id: AccountId,
        character: ObjId,
        run: RunId,
    ) -> Result<Prepared, BulkError> {
        let row = self.run(id, character, run)?;
        if !row.status.terminal()
            || row.pins != 0
            || matches!(row.plan.kind, WorkKind::Registry | WorkKind::Conduct)
        {
            return Err(BulkError::Retention);
        }
        let mut account = self.account(id)?.clone();
        let removed = account.runs.remove(&run).ok_or(BulkError::Missing)?;
        self.prepared(account, vec![removed], None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(name: &str) -> AccountId {
        AccountId::mint(name, 1).expect("account")
    }
    fn plan() -> Plan {
        Plan {
            registry: None,
            conduct: None,
            kind: WorkKind::Proving,
            scenario: Scenario::Duel,
            opponent: Opponent::Mirror,
            per_side: 1,
            unit_count: 2,
            games: 10,
            seeds: (0..5).collect(),
            limit_ms: 60_000,
            native_substep_ms: 250,
            start: Artifact {
                sha256: [1; 32],
                bytes: 100,
            },
            result_reservation_bytes: 1000,
        }
    }
    fn store() -> BulkStore {
        BulkStore::new(BulkConfig::default()).expect("config")
    }
    fn publish(store: &mut BulkStore, p: Prepared) -> Option<Lease> {
        store.check(&p).expect("current");
        store.apply_checked(p)
    }
    fn admit(
        store: &mut BulkStore,
        account: AccountId,
        actor: ObjId,
        nonce: u64,
        now: i64,
    ) -> RunId {
        let (run, p) = store
            .prepare_admit(account, actor, nonce, plan(), now)
            .expect("admit");
        publish(store, p);
        run
    }
    fn complete(store: &mut BulkStore, account: AccountId, run: RunId, now: i64) -> Vec<u8> {
        let p = store.prepare_grant(account, run, now, 250).expect("grant");
        let lease = publish(store, p).expect("lease");
        let p = store
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 200,
                    result: Some(Artifact {
                        sha256: [2; 32],
                        bytes: 200,
                    }),
                    terminal: Some(Status::Complete),
                },
                now + 1,
            )
            .expect("result");
        let bytes = p.bytes().to_vec();
        publish(store, p);
        bytes
    }
    #[test]
    fn quota_charges_all_units_and_refunds_only_known_unused_time() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        let p = s.prepare_grant(a, r, 0, 250).expect("grant");
        let lease = publish(&mut s, p).expect("lease");
        assert_eq!(s.account(a).expect("account").spent_unit_ms, 500);
        let p = s
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 400,
                    result: None,
                    terminal: None,
                },
                1,
            )
            .expect("settle");
        publish(&mut s, p);
        assert_eq!(s.account(a).expect("account").spent_unit_ms, 400);
        assert_eq!(s.run(a, ObjId(1), r).expect("run").charged_unit_ms, 400)
    }
    #[test]
    fn legacy_hydration_preserves_existing_interruption_and_pin_cleanup() {
        let mut owner = store();
        let account = id("legacy-recovery");
        let completed = admit(&mut owner, account, ObjId(1), 1, 0);
        complete(&mut owner, account, completed, 1);
        let pinned = owner
            .prepare_pin(account, ObjId(1), completed, true)
            .expect("pin completed run");
        publish(&mut owner, pinned);
        let active = admit(&mut owner, account, ObjId(1), 2, 2);
        let grant = owner.prepare_grant(account, active, 3, 250).expect("grant");
        let _ = publish(&mut owner, grant).expect("paid flight");
        let before = owner.account(account).expect("account").clone();
        let old = owner
            .prepare_recover(account, 10)
            .expect("existing recovery");
        let legacy = owner
            .prepare_legacy_recover(account, 10)
            .expect("legacy recovery");
        assert_eq!(
            legacy.document().expect("document"),
            old.document().expect("document")
        );
        assert!(!legacy.changes_registry());
        assert_eq!(owner.account(account).expect("account"), &before);
        publish(&mut owner, legacy);
        let after = owner.account(account).expect("account");
        assert_eq!(after.spent_unit_ms, before.spent_unit_ms);
        assert_eq!(after.reserved_unit_ms, 0);
        assert!(after.active.is_none());
        assert_eq!(after.runs.get(&completed).expect("completed").pins, 0);
        let active = after.runs.get(&active).expect("interrupted");
        assert_eq!(active.status, Status::Interrupted);
        assert!(active.flight.is_none());
    }

    #[test]
    fn crash_keeps_prepaid_cost_and_does_not_rerun_completed_nonce() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        let p = s.prepare_grant(a, r, 0, 250).expect("grant");
        let bytes = p.bytes().to_vec();
        publish(&mut s, p);
        let mut boot = store();
        boot.load(a, &bytes).expect("load");
        let p = boot.prepare_recover(a, 100).expect("recover");
        publish(&mut boot, p);
        assert_eq!(boot.account(a).expect("account").spent_unit_ms, 500);
        assert_eq!(
            boot.run(a, ObjId(1), r).expect("run").status,
            Status::Interrupted
        );
        assert_eq!(
            boot.run(a, ObjId(1), r).expect("same retained run").nonce,
            1
        );
        assert!(matches!(
            boot.prepare_admit(a, ObjId(1), 1, plan(), 200),
            Err(BulkError::Consumed)
        ))
    }
    #[test]
    fn completed_roundtrip_is_idempotent_and_owner_scoped() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        let bytes = complete(&mut s, a, r, 0);
        let mut boot = store();
        boot.load(a, &bytes).expect("roundtrip");
        assert_eq!(
            boot.run(a, ObjId(1), r)
                .expect("run")
                .result
                .as_ref()
                .expect("report")
                .bytes,
            200
        );
        assert_eq!(boot.account(a).expect("account").spent_unit_ms, 200);
        assert!(matches!(boot.run(a, ObjId(2), r), Err(BulkError::Missing)));
        assert!(matches!(
            boot.run(id("b"), ObjId(1), r),
            Err(BulkError::Missing)
        ))
    }
    #[test]
    fn cancellation_waits_for_inflight_receipt_and_cannot_admit_twice() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        let p = s.prepare_grant(a, r, 0, 250).expect("grant");
        let lease = publish(&mut s, p).expect("lease");
        let p = s.prepare_cancel(a, ObjId(1), r, 1).expect("cancel");
        publish(&mut s, p);
        assert!(matches!(
            s.prepare_admit(a, ObjId(1), 2, plan(), 2),
            Err(BulkError::Busy)
        ));
        let p = s
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 300,
                    result: None,
                    terminal: None,
                },
                3,
            )
            .expect("cancel done");
        publish(&mut s, p);
        assert_eq!(
            s.run(a, ObjId(1), r).expect("run").status,
            Status::Cancelled
        );
        assert_eq!(s.account(a).expect("account").spent_unit_ms, 300);
        assert_eq!(s.account(a).expect("account").reserved_unit_ms, 0)
    }
    #[test]
    fn stale_and_duplicate_completions_cannot_change_usage() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        let p = s.prepare_grant(a, r, 0, 250).expect("grant");
        let lease = publish(&mut s, p).expect("lease");
        let duplicate = lease.clone();
        let p = s
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 400,
                    result: None,
                    terminal: None,
                },
                1,
            )
            .expect("complete");
        publish(&mut s, p);
        assert!(matches!(
            s.prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease: duplicate,
                    actual_unit_ms: 0,
                    result: None,
                    terminal: None
                },
                2
            ),
            Err(BulkError::Stale)
        ));
        assert_eq!(s.account(a).expect("account").spent_unit_ms, 400)
    }
    #[test]
    fn midnight_renews_allowance_without_resetting_active_run_or_charging_yesterday_twice() {
        let mut s = store();
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, DAY_MS - 1);
        let p = s.prepare_grant(a, r, DAY_MS - 1, 250).expect("grant");
        let lease = publish(&mut s, p).expect("lease");
        assert!(matches!(
            s.prepare_grant(a, r, DAY_MS, 250),
            Err(BulkError::Busy)
        ));
        let p = s
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 500,
                    result: None,
                    terminal: None,
                },
                DAY_MS,
            )
            .expect("old grant");
        publish(&mut s, p);
        let p = s.prepare_grant(a, r, DAY_MS, 250).expect("new day");
        publish(&mut s, p);
        let a = s.account(a).expect("account");
        assert_eq!(a.utc_day, 1);
        assert_eq!(a.spent_unit_ms, 500);
        assert_eq!(a.runs[&r].charged_unit_ms, 1000)
    }
    #[test]
    fn quota_exhaustion_refuses_and_next_day_recovers() {
        let cfg = BulkConfig {
            daily_unit_minutes: 1,
            ..Default::default()
        };
        let mut s = BulkStore::new(cfg).expect("store");
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        for _ in 0..120 {
            let p = s.prepare_grant(a, r, 0, 250).expect("grant");
            let lease = publish(&mut s, p).expect("lease");
            let p = s
                .prepare_finish(
                    Receipt {
                        registry: None,
                        conduct: None,
                        lease,
                        actual_unit_ms: 500,
                        result: None,
                        terminal: None,
                    },
                    0,
                )
                .expect("step");
            publish(&mut s, p);
        }
        assert!(matches!(
            s.prepare_grant(a, r, 0, 250),
            Err(BulkError::Quota)
        ));
        assert!(s.prepare_grant(a, r, DAY_MS, 250).is_ok())
    }
    #[test]
    fn pinned_result_blocks_eviction_and_release_allows_oldest_first() {
        let cfg = BulkConfig {
            completed_per_character: 1,
            ..Default::default()
        };
        let mut s = BulkStore::new(cfg).expect("store");
        let a = id("a");
        let r = admit(&mut s, a, ObjId(1), 1, 0);
        complete(&mut s, a, r, 0);
        let p = s.prepare_pin(a, ObjId(1), r, true).expect("pin");
        publish(&mut s, p);
        assert!(matches!(
            s.prepare_admit(a, ObjId(1), 2, plan(), 3),
            Err(BulkError::Retention)
        ));
        assert!(matches!(
            s.prepare_forget(a, ObjId(1), r),
            Err(BulkError::Retention)
        ));
        let p = s.prepare_pin(a, ObjId(1), r, false).expect("release");
        publish(&mut s, p);
        let (_, p) = s.prepare_admit(a, ObjId(1), 2, plan(), 4).expect("evict");
        assert_eq!(p.evicted().len(), 1);
        assert_eq!(p.evicted()[0].id, r);
        publish(&mut s, p);
        assert!(matches!(s.run(a, ObjId(1), r), Err(BulkError::Missing)));
        assert!(matches!(
            s.prepare_admit(a, ObjId(1), 1, plan(), 5),
            Err(BulkError::Consumed)
        ))
    }
    #[test]
    fn corrupt_records_fail_closed_and_invalidate_prepared_work() {
        let mut s = store();
        let a = id("a");
        let (_, p) = s
            .prepare_admit(a, ObjId(1), 1, plan(), 0)
            .expect("prepared");
        assert_eq!(s.load(a, b"{}"), Err(BulkError::Quarantined));
        assert!(s.check(&p).is_err());
        assert!(matches!(
            s.prepare_admit(a, ObjId(1), 2, plan(), 1),
            Err(BulkError::Quarantined)
        ));
        assert_eq!(utc_day(-1), -1)
    }
    #[test]
    fn bench_and_proving_share_account_admission() {
        let mut s = store();
        let a = id("a");
        admit(&mut s, a, ObjId(1), 1, 0);
        let mut p = plan();
        p.kind = WorkKind::Bench;
        p.games = 1;
        p.seeds = vec![0];
        assert!(matches!(
            s.prepare_admit(a, ObjId(2), 2, p, 0),
            Err(BulkError::Busy)
        ))
    }
    #[test]
    fn bench_step_history_never_evicts_retained_proving_or_resets_spent_quota() {
        let cfg = BulkConfig {
            completed_per_character: 1,
            ..Default::default()
        };
        let mut s = BulkStore::new(cfg).unwrap();
        let a = id("bench");
        let actor = ObjId(1);
        let original = admit(&mut s, a, actor, 1, 0);
        complete(&mut s, a, original, 0);
        for nonce in 2..=5 {
            let mut p = plan();
            p.kind = WorkKind::Bench;
            p.games = 1;
            p.seeds = vec![0];
            p.result_reservation_bytes = 1;
            let (run, change) = s.prepare_admit(a, actor, nonce, p, nonce as i64).unwrap();
            publish(&mut s, change);
            let change = s.prepare_grant(a, run, nonce as i64, 250).unwrap();
            let lease = publish(&mut s, change).unwrap();
            let change = s
                .prepare_finish(
                    Receipt {
                        registry: None,
                        conduct: None,
                        lease,
                        actual_unit_ms: 200,
                        result: None,
                        terminal: Some(Status::Complete),
                    },
                    nonce as i64 + 1,
                )
                .unwrap();
            publish(&mut s, change);
            assert!(s.run(a, actor, original).is_ok());
            assert_eq!(s.account(a).unwrap().runs.len(), 2);
        }
        assert_eq!(s.account(a).unwrap().spent_unit_ms, 1000);
    }
    #[test]
    fn p2_tsc_7_terminal_failed_and_cancelled_proving_release_result_reservation() {
        let cfg = BulkConfig {
            completed_per_character: 10,
            bytes_per_character: MAX_ARTIFACT_BYTES, // 16 MiB
            ..Default::default()
        };
        let mut s = BulkStore::new(cfg).expect("store");
        let a = id("p2-tsc-7-reservation");
        let actor = ObjId(1);
        let mut big = plan();
        big.result_reservation_bytes = 8 * 1024 * 1024; // 8 MiB reservation per run

        // Cancel two pinned Proving runs (each had 8 MiB reservation, result == None).
        for nonce in 1..=2 {
            let (run, p) = s
                .prepare_admit(a, actor, nonce, big.clone(), nonce as i64)
                .expect("admit");
            publish(&mut s, p);
            let p = s
                .prepare_cancel(a, actor, run, nonce as i64 + 10)
                .expect("cancel");
            publish(&mut s, p);
            assert_eq!(
                s.run(a, actor, run).expect("run").retained_bytes(),
                big.start.bytes,
                "cancelled run with no result must not retain 8 MiB reservation"
            );
            let p = s.prepare_pin(a, actor, run, true).expect("pin");
            publish(&mut s, p);
        }
        // Admitting a 3rd run (8 MiB + 100 B) fits cleanly under the 16 MiB cap
        // because the 2 pinned cancelled runs charge only 200 B total, not 16 MiB.
        let (third, p) = s
            .prepare_admit(a, actor, 3, big.clone(), 30)
            .expect("third run must not deadlock on phantom reservations");
        publish(&mut s, p);
        let p = s.prepare_grant(a, third, 31, 250).expect("grant");
        let lease = publish(&mut s, p).expect("lease");
        let p = s
            .prepare_finish(
                Receipt {
                    registry: None,
                    conduct: None,
                    lease,
                    actual_unit_ms: 500,
                    result: None,
                    terminal: Some(Status::Failed),
                },
                32,
            )
            .expect("fail");
        publish(&mut s, p);
        assert_eq!(
            s.run(a, actor, third).expect("failed run").retained_bytes(),
            big.start.bytes,
            "failed run with no result must not retain 8 MiB reservation"
        );
    }

    #[test]
    fn p2_tsc_20_preflight_admit_rejects_exhausted_quota_and_pinned_retention_before_prepare() {
        let cfg = BulkConfig {
            daily_unit_minutes: 1, // 60_000 unit-ms
            completed_per_character: 1,
            bytes_per_character: MAX_ARTIFACT_BYTES + 1024,
            ..Default::default()
        };
        let mut s = BulkStore::new(cfg).expect("store");
        let a = id("p2-tsc-20-preflight");
        let actor = ObjId(7);

        // Fresh account passes preflight for both Proving and Registry.
        assert_eq!(
            s.preflight_admit(
                a,
                actor,
                WorkKind::Proving,
                600,
                1 + MAX_ARTIFACT_BYTES,
                1_000
            ),
            Ok(())
        );
        assert_eq!(
            s.preflight_admit(a, actor, WorkKind::Registry, 100, 0, 1_000),
            Ok(())
        );

        // Admit, finish spending the entire daily quota (12 grants of 2_500 sim_ms * 2 units = 60_000),
        // and pin the single proving run.
        let (run, p) = s
            .prepare_admit(a, actor, 1, plan(), 1_000)
            .expect("first admit");
        publish(&mut s, p);
        let max_sim_ms = 250 * LEASE_STEPS;
        let charge_per_grant = max_sim_ms * 2;
        let grants = 60_000 / charge_per_grant;
        assert_eq!(grants * charge_per_grant, 60_000);
        for step in 0..grants {
            let t = 1_001 + step as i64 * 10;
            let p = s.prepare_grant(a, run, t, max_sim_ms).expect("grant");
            let lease = publish(&mut s, p).expect("lease");
            let final_step = step + 1 == grants;
            let p = s
                .prepare_finish(
                    Receipt {
                        registry: None,
                        conduct: None,
                        lease,
                        actual_unit_ms: charge_per_grant,
                        result: final_step.then_some(Artifact {
                            sha256: [9; 32],
                            bytes: 100,
                        }),
                        terminal: final_step.then_some(Status::Complete),
                    },
                    t + 1,
                )
                .expect("finish");
            publish(&mut s, p);
        }
        let p = s.prepare_pin(a, actor, run, true).expect("pin");
        publish(&mut s, p);

        // Same UTC day: 0 quota left -> both Proving and Registry fail Quota synchronously.
        assert_eq!(
            s.preflight_admit(
                a,
                actor,
                WorkKind::Proving,
                100,
                1 + MAX_ARTIFACT_BYTES,
                2_000
            ),
            Err(BulkError::Quota)
        );
        assert_eq!(
            s.preflight_admit(a, actor, WorkKind::Registry, 100, 0, 2_000),
            Err(BulkError::Quota)
        );

        // Next UTC day: quota resets, but the 1 pinned Proving run blocks Proving with Retention
        // while Registry (separate archival retention) passes preflight.
        let next_day = 86_400_000 + 2_000;
        assert_eq!(
            s.preflight_admit(
                a,
                actor,
                WorkKind::Proving,
                100,
                1 + MAX_ARTIFACT_BYTES,
                next_day
            ),
            Err(BulkError::Retention)
        );
        assert_eq!(
            s.preflight_admit(a, actor, WorkKind::Registry, 100, 0, next_day),
            Ok(())
        );
    }
}
