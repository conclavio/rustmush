//! Per-account durable journal. Every mutation is fully serialized before
//! publication. A worker lease is released only by apply_checked, after the
//! world publisher has checked/committed this change and its artifact/ref ops.
use super::{BulkError, invalid, model::*};
use mush_core::{AccountId, ObjId};
use std::collections::{BTreeMap, BTreeSet};

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
}
impl Lease {
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
pub(crate) struct Prepared {
    account: Account,
    expected: u64,
    encoded: Vec<u8>,
    evicted: Vec<Run>,
    lease: Option<Lease>,
}
impl Prepared {
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
}
#[derive(Clone)]
pub(crate) struct BulkStore {
    cfg: BulkConfig,
    generation: u64,
    accounts: BTreeMap<AccountId, Account>,
    quarantined: BTreeSet<AccountId>,
}
impl BulkStore {
    pub fn new(cfg: BulkConfig) -> Result<Self, BulkError> {
        cfg.validate()?;
        Ok(Self {
            cfg,
            generation: 0,
            accounts: BTreeMap::new(),
            quarantined: BTreeSet::new(),
        })
    }
    pub fn account_ids(&self) -> Vec<AccountId> {
        self.accounts.keys().copied().collect()
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
        self.account(id)?.runs.get(&run).ok_or(BulkError::Missing)
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
            Some(account) => {
                self.accounts.insert(id, account);
                self.quarantined.remove(&id);
                Ok(())
            }
            None => {
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
        account.validate(&self.cfg)?;
        let encoded = serde_json::to_vec(&account).map_err(|e| invalid(e.to_string()))?;
        if encoded.len() > MAX_ACCOUNT_DOCUMENT {
            return Err(invalid("account document bound"));
        }
        self.generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation exhausted"))?;
        Ok(Prepared {
            account,
            expected: self.generation,
            encoded,
            evicted,
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
    /// Call only after check and after every other participating owner has
    /// checked. No fallible work may follow the publisher's first mutation.
    pub fn apply_checked(&mut self, change: Prepared) -> Option<Lease> {
        self.generation = self.generation.saturating_add(1);
        self.accounts.insert(change.account.id, change.account);
        change.lease
    }
    pub fn run(&self, account: AccountId, character: ObjId, id: RunId) -> Result<&Run, BulkError> {
        self.account(account)?
            .runs
            .get(&id)
            .filter(|r| r.character == character)
            .ok_or(BulkError::Missing)
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
    pub fn prepare_admit(
        &self,
        id: AccountId,
        character: ObjId,
        nonce: u64,
        plan: Plan,
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
        if available < plan.units() * u64::from(plan.native_substep_ms) {
            return Err(BulkError::Quota);
        }
        let old_benches = account
            .runs
            .values()
            .filter(|r| r.plan.kind == WorkKind::Bench && r.status.terminal())
            .map(|r| r.id)
            .collect::<Vec<_>>();
        let mut evicted = old_benches
            .into_iter()
            .filter_map(|id| account.runs.remove(&id))
            .collect::<Vec<_>>();
        evicted.extend(self.trim(
            &mut account,
            character,
            if plan.kind == WorkKind::Proving {
                plan.start.bytes + plan.result_reservation_bytes
            } else {
                0
            },
            usize::from(plan.kind == WorkKind::Proving),
        )?);
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
        Ok((run, self.prepared(account, evicted, None)?))
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
        if sim_ms == 0 || sim_ms > row.plan.quantum_ms() {
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
            / row.plan.units();
        let sim_ms = sim_ms.min(available);
        if sim_ms < u64::from(row.plan.native_substep_ms) {
            return Err(BulkError::Quota);
        }
        let cost = sim_ms
            .checked_mul(row.plan.units())
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
            units: row.plan.units(),
            start: row.plan.start.sha256,
            kind: row.plan.kind,
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
            sequence: lease.sequence,
            utc_day: lease.utc_day,
            allowance_unit_ms: lease.allowance,
        };
        if row.flight.as_ref() != Some(&expected) || account.utc_day != lease.utc_day {
            return Err(BulkError::Stale);
        }
        if receipt.actual_unit_ms > lease.allowance {
            return Err(invalid("worker exceeded its grant"));
        }
        let refund = lease.allowance - receipt.actual_unit_ms;
        account.spent_unit_ms -= refund;
        account.reserved_unit_ms += refund;
        row.charged_unit_ms -= refund;
        row.flight = None;
        if let Some(result) = receipt.result {
            result.validate()?;
            if result.bytes > row.plan.result_reservation_bytes {
                return Err(invalid("result exceeded reservation"));
            }
            row.result = Some(result)
        }
        let terminal = if row.status == Status::CancelRequested {
            Some(Status::Cancelled)
        } else {
            receipt.terminal
        };
        if let Some(status) = terminal {
            if !status.terminal()
                || (status == Status::Complete
                    && row.plan.kind == WorkKind::Proving
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
        if !row.status.terminal() || row.pins != 0 {
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
}
