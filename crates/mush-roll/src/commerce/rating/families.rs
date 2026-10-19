//! Actual owner-stamped typed families and complete retention evidence reads.
//! Registration is Roll-private. No family handle or arbitrary write callback
//! is handed to a transport. Structural resolution does not replace the fixed
//! Registry producer/current-compatibility check at commercial admission.

use mush_core::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::mechanics::CanonicalId;
use mush_core::{CommitToken, ModError, ModuleCtx, Registrar};
use serde::{Serialize, de::DeserializeOwned};

use super::execution::{EXECUTION_BYTES, RatingExecution};
use super::*;
use crate::RollStore;
use crate::commerce::read::CommerceRead;
use crate::commerce::roots::{RequiredRoot, RootAnchor};

#[derive(Clone)]
pub struct RatingFamilies {
    pub(crate) receipts: StateFamilyHandle<CommitToken, RatingReceipt>,
    pub(crate) pages: StateFamilyHandle<RatingPageKey, RatingPage>,
    pub(crate) jobs: StateFamilyHandle<CommitToken, RatingJob>,
    pub(crate) completions: StateFamilyHandle<CommitToken, RatingCompletion>,
    pub(crate) status: StateFamilyHandle<CommitToken, RatingStatus>,
    pub(crate) executions: StateFamilyHandle<CommitToken, RatingExecution>,
}

fn descriptor(name: &str, limit: usize) -> Result<StateFamilyDescriptor, ModError> {
    let id = StateFamilyId::new(name).map_err(|e| ModError::msg(e.to_string()))?;
    let canonical = |suffix: &str| {
        CanonicalId::new(format!("roll.{name}.{suffix}.v1"))
            .map_err(|e| ModError::msg(e.to_string()))
    };
    StateFamilyDescriptor::new(
        id,
        canonical("key")?,
        canonical("value")?,
        FORMAT,
        canonical("decode")?,
        canonical("validate")?,
        canonical("serialize")?,
    )
    .and_then(|d| d.with_limits(80, limit))
    .map_err(|e| ModError::msg(e.to_string()))
}
fn decode<T: DeserializeOwned>(value: &serde_json::Value, limit: usize) -> Result<T, String> {
    let bytes = encoded(value, limit - TYPED_ENVELOPE_RESERVE).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
fn serialize<T: Serialize>(value: &T, limit: usize) -> Result<serde_json::Value, String> {
    encoded(value, limit - TYPED_ENVELOPE_RESERVE).map_err(|e| e.to_string())?;
    serde_json::to_value(value).map_err(|e| e.to_string())
}
impl RatingFamilies {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        let receipts = reg.state_family(
            descriptor("rating_receipts", MAX_RECEIPT_BYTES)?,
            |key: &CommitToken| Ok(key.to_string()),
            |v: &serde_json::Value| decode::<RatingReceipt>(v, MAX_RECEIPT_BYTES),
            |v: &RatingReceipt| v.validate().map_err(|e| e.to_string()),
            |v: &RatingReceipt| serialize(v, MAX_RECEIPT_BYTES),
        )?;
        let pages = reg.state_family(
            descriptor("rating_pages", MAX_PAGE_BYTES)?,
            |key: &RatingPageKey| Ok(key.encoded()),
            |v: &serde_json::Value| decode::<RatingPage>(v, MAX_PAGE_BYTES),
            |v: &RatingPage| v.validate().map_err(|e| e.to_string()),
            |v: &RatingPage| serialize(v, MAX_PAGE_BYTES),
        )?;
        let jobs = reg.state_family(
            descriptor("rating_jobs", 4096)?,
            |key: &CommitToken| Ok(key.to_string()),
            |v: &serde_json::Value| decode::<RatingJob>(v, 4096),
            |v: &RatingJob| v.validate().map_err(|e| e.to_string()),
            |v: &RatingJob| serialize(v, 4096),
        )?;
        let completions = reg.state_family(
            descriptor("rating_completions", 4096)?,
            |key: &CommitToken| Ok(key.to_string()),
            |v: &serde_json::Value| decode::<RatingCompletion>(v, 4096),
            |v: &RatingCompletion| v.validate().map_err(|e| e.to_string()),
            |v: &RatingCompletion| serialize(v, 4096),
        )?;
        let status = reg.state_family(
            descriptor("rating_status", 4096)?,
            |key: &CommitToken| Ok(key.to_string()),
            |v: &serde_json::Value| decode::<RatingStatus>(v, 4096),
            |v: &RatingStatus| v.validate().map_err(|e| e.to_string()),
            |v: &RatingStatus| serialize(v, 4096),
        )?;
        let executions = reg.state_family(
            descriptor("rating_executions", EXECUTION_BYTES)?,
            |key: &CommitToken| Ok(key.to_string()),
            |v: &serde_json::Value| decode::<RatingExecution>(v, EXECUTION_BYTES),
            |v: &RatingExecution| v.validate().map_err(|e| e.to_string()),
            |v: &RatingExecution| serialize(v, EXECUTION_BYTES),
        )?;
        Ok(Self {
            receipts,
            pages,
            jobs,
            completions,
            status,
            executions,
        })
    }

    /// Reads the actual committed row and binds its full immutable digest.
    /// This is structural provenance; current compatibility and the closed
    /// producer's acknowledged completion are separate commercial guards.
    pub fn resolve_receipt<'a>(
        &self,
        ctx: &'a ModuleCtx<'_>,
        reference: &RatingRef,
    ) -> Result<&'a RatingReceipt, RatingError> {
        self.resolve_receipt_read(ctx, reference)
    }

    fn resolve_receipt_read<'a>(
        &self,
        ctx: &'a impl CommerceRead,
        reference: &RatingRef,
    ) -> Result<&'a RatingReceipt, RatingError> {
        let row = read(ctx, &self.receipts, &reference.job)?;
        if row.reference()? != *reference {
            return Err(RatingError::Identity);
        }
        Ok(row)
    }

    /// Complete committed evidence and Active status, bound to an expected
    /// manifest. This is deliberately not an eligibility capability: the
    /// closed game reader must supply its actual current compatibility, and
    /// completion publication must come from the registered measured producer.
    /// A transport-supplied digest cannot establish either premise.
    pub(crate) fn complete_active_receipt<'a>(
        &self,
        ctx: &'a impl CommerceRead,
        reference: &RatingRef,
        expected: &RatingCompatibility,
    ) -> Result<&'a RatingReceipt, RatingError> {
        let row = self.resolve_receipt_read(ctx, reference)?;
        self.complete_receipt(ctx, row)?;
        let status = read(ctx, &self.status, &row.publication.job)?;
        if row.compatibility() != expected
            || status.rating() != reference
            || !matches!(status.standing(), RatingStanding::Active {})
        {
            return Err(RatingError::Invalid);
        }
        Ok(row)
    }

    /// Same exact evidence check on the authoritative action's scratch view.
    /// Reads stay inside the typed owner's revision/reservation fence. The
    /// caller must also guard the actual registered manifest DefinitionRef;
    /// a request-provided expected digest is never current compatibility.
    pub(crate) fn complete_active_receipt_in_txn(
        &self,
        txn: &mut ActionTxn<'_>,
        reference: &RatingRef,
        expected: &RatingCompatibility,
    ) -> Result<RatingReceipt, RatingError> {
        let row = read_txn(txn, &self.receipts, &reference.job)?;
        row.validate()?;
        if row.reference()? != *reference || row.compatibility() != expected {
            return Err(RatingError::Identity);
        }
        let completion = read_txn(txn, &self.completions, &row.publication.job)?;
        let job = read_txn(txn, &self.jobs, &row.publication.job)?;
        let execution = read_txn(txn, &self.executions, &row.publication.job)?;
        let status = read_txn(txn, &self.status, &row.publication.job)?;
        if !completion.matches(&row, &execution)?
            || !job.matches_receipt(&row)?
            || status.rating() != reference
            || !matches!(status.standing(), RatingStanding::Active {})
        {
            return Err(RatingError::Incomplete);
        }
        let mut pages = Vec::with_capacity(row.result().pages().len());
        for reference in row.result().pages() {
            let key = RatingPageKey::new(row.publication.job, reference.ordinal())?;
            let page = read_txn(txn, &self.pages, &key)?;
            if page.key() != key {
                return Err(RatingError::Identity);
            }
            pages.push(page);
        }
        row.verify_pages(&pages)?;
        execution.verify_completion(&job, &row, &pages)?;
        Ok(row)
    }

    fn complete_receipt(
        &self,
        ctx: &impl CommerceRead,
        row: &RatingReceipt,
    ) -> Result<(), RatingError> {
        row.validate()?;
        let completion = read(ctx, &self.completions, &row.publication.job)?;
        let execution = read(ctx, &self.executions, &row.publication.job)?;
        if !completion.matches(row, execution)? {
            return Err(RatingError::Identity);
        }
        let job = read(ctx, &self.jobs, &row.publication.job)?;
        if !job.matches_receipt(row)? {
            return Err(RatingError::Incomplete);
        }
        let status = read(ctx, &self.status, &row.publication.job)?;
        if status.rating() != &row.reference()? {
            return Err(RatingError::Identity);
        }
        let mut pages = Vec::with_capacity(row.result().pages().len());
        // Exact keys come from the checked receipt. Batching only reduces read
        // calls: every requested row is charged and every full page is still
        // read and verified. There is no scan cursor or retained proof cache.
        for references in row
            .result()
            .pages()
            .chunks(mush_core::read_service::MAX_OWNER_READ_PAGE)
        {
            let keys = references
                .iter()
                .map(|reference| RatingPageKey::new(row.publication.job, reference.ordinal()))
                .collect::<Result<Vec<_>, _>>()?;
            let found = ctx
                .get_many(&self.pages, &keys)
                .map_err(|_| RatingError::Unavailable)?;
            if found.len() != keys.len() {
                return Err(RatingError::Incomplete);
            }
            for (key, page) in keys.into_iter().zip(found) {
                let page = page.ok_or(RatingError::Incomplete)?;
                if page.key() != key {
                    return Err(RatingError::Identity);
                }
                pages.push(page.clone());
            }
        }
        row.verify_pages(&pages)?;
        execution.verify_completion(job, row, &pages)
    }

    /// Exact actual-record projection for the reverse root audit. Missing rows
    /// and unavailable families differ; neither can open collection by itself.
    pub(crate) fn root_subject(
        &self,
        ctx: &ModuleCtx<'_>,
        anchor: &RootAnchor,
    ) -> Result<Option<RevisionIdentity>, RatingError> {
        match anchor {
            RootAnchor::Rating { job, id } => {
                let Some(row) = ctx
                    .action_state(&self.receipts, job)
                    .map_err(|_| RatingError::Unavailable)?
                else {
                    return Ok(None);
                };
                if row.id() != *id || row.publication.job != *job {
                    return Err(RatingError::Identity);
                }
                self.complete_receipt(ctx, row)?;
                Ok(Some(row.subject().revision()))
            }
            RootAnchor::RatingJob(token) => {
                let Some(row) = ctx
                    .action_state(&self.jobs, token)
                    .map_err(|_| RatingError::Unavailable)?
                else {
                    return Ok(None);
                };
                if row.job() != *token {
                    return Err(RatingError::Identity);
                }
                row.validate()?;
                Ok(Some(row.subject().revision()))
            }
            RootAnchor::RatingPending(token) => {
                let Some(row) = ctx
                    .action_state(&self.jobs, token)
                    .map_err(|_| RatingError::Unavailable)?
                else {
                    return Ok(None);
                };
                if row.job() != *token {
                    return Err(RatingError::Identity);
                }
                row.validate()?;
                if !matches!(
                    row.state(),
                    RatingJobState::Queued {} | RatingJobState::Running { .. }
                ) {
                    return Ok(None);
                }
                Ok(Some(row.subject().revision()))
            }
            _ => Err(RatingError::Invalid),
        }
    }

    /// Actual typed nonterminal rows for the closed cross-owner boot join.
    /// Terminal evidence remains covered by the complete receipt/root audit;
    /// it does not require an indefinitely retained private quota document.
    pub(crate) fn stopped_executions(
        &self,
        ctx: &ModuleCtx<'_>,
    ) -> Result<Vec<super::execution::RatingExecution>, RatingError> {
        let mut rows = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut visited = 0_usize;
        scan(ctx, &self.jobs, |key, job| {
            visited = visited.checked_add(1).ok_or(RatingError::Bound)?;
            if visited > crate::MAX_RECORDS {
                return Err(RatingError::Bound);
            }
            job.validate()?;
            if key != job.job().to_string() {
                return Err(RatingError::Identity);
            }
            let execution = read(ctx, &self.executions, &job.job())?;
            execution.matches_job(job)?;
            if !execution.observation().phase.terminal() {
                if rows.len() == super::protocol::MAX_STOPPED_REGISTRY_JOBS {
                    return Err(RatingError::Bound);
                }
                if !seen.insert(job.job()) {
                    return Err(RatingError::Identity);
                }
                rows.push(execution.clone());
            }
            Ok(())
        })?;
        Ok(rows)
    }

    /// Synchronous final retention audit, after source hydration. It scans all
    /// six real families even when no Roll roots exist. No absence of records
    /// is substituted for an unavailable owner-scoped typed reader. The caller
    /// also reverse-checks all Roll roots and binds typed/source generations.
    pub(crate) fn audit_for_collection(
        &self,
        ctx: &ModuleCtx<'_>,
        roll: &RollStore,
    ) -> Result<(), RatingError> {
        scan(ctx, &self.receipts, |key, row| {
            if key != row.publication.job.to_string() {
                return Err(RatingError::Identity);
            }
            self.complete_receipt(ctx, row)?;
            required(
                roll,
                RootAnchor::Rating {
                    job: row.publication.job,
                    id: row.id(),
                },
                row.subject().revision(),
            )
        })?;
        scan(ctx, &self.jobs, |key, row| {
            row.validate()?;
            if key != row.job().to_string() {
                return Err(RatingError::Identity);
            }
            let execution = read(ctx, &self.executions, &row.job())?;
            execution.matches_job(row)?;
            for obligation in crate::commerce::rating_roots::required(row)? {
                required(roll, obligation.anchor, obligation.identity)?;
            }
            if let RatingJobState::Completed { receipt } = row.state() {
                let receipt = self.resolve_receipt(ctx, receipt)?;
                if !row.matches_receipt(receipt)? {
                    return Err(RatingError::Identity);
                }
                self.complete_receipt(ctx, receipt)?;
            }
            Ok(())
        })?;
        scan(ctx, &self.completions, |key, row| {
            if key != row.job().to_string() {
                return Err(RatingError::Identity);
            }
            let receipt = self.resolve_receipt(ctx, row.receipt())?;
            let execution = read(ctx, &self.executions, &row.job())?;
            if !row.matches(receipt, execution)? {
                return Err(RatingError::Identity);
            }
            Ok(())
        })?;
        scan(ctx, &self.executions, |key, row| {
            row.validate()?;
            if key != row.job().to_string() {
                return Err(RatingError::Identity);
            }
            row.matches_job(read(ctx, &self.jobs, &row.job())?)
        })?;
        scan(ctx, &self.status, |key, row| {
            if key != row.rating().job().to_string() {
                return Err(RatingError::Identity);
            }
            self.resolve_receipt(ctx, row.rating())?;
            Ok(())
        })?;
        // Each page has one exact parent key. The content hash excludes this
        // wrapper, so the receipt can bind page hashes without a digest cycle.
        scan(ctx, &self.pages, |key, page| {
            page.validate()?;
            if key != page.key().encoded() {
                return Err(RatingError::Identity);
            }
            let parent = read(ctx, &self.receipts, &page.job)?;
            if parent.publication.job != page.job
                || parent.id() != page.rating
                || parent.result().id() != page.result
            {
                return Err(RatingError::Identity);
            }
            let expected = parent
                .result()
                .pages()
                .get(usize::from(page.content.ordinal))
                .ok_or(RatingError::Incomplete)?;
            if expected.ordinal() != page.content.ordinal
                || expected.digest() != page.content.digest()?
            {
                return Err(RatingError::Identity);
            }
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "families/page_read_tests.rs"]
mod page_read_tests;

fn required(
    roll: &RollStore,
    anchor: RootAnchor,
    identity: RevisionIdentity,
) -> Result<(), RatingError> {
    let reference = identity.revision();
    let revision = roll
        .revision(reference.key.character, reference.key)
        .map_err(|_| RatingError::Unavailable)?;
    if !identity
        .matches(revision)
        .map_err(|_| RatingError::Identity)?
    {
        return Err(RatingError::Identity);
    }
    roll.check_required_root(&RequiredRoot { anchor, identity })
        .map_err(|_| RatingError::Incomplete)
}
fn read<'a, K, V>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<K, V>,
    key: &K,
) -> Result<&'a V, RatingError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    ctx.get(handle, key)
        .map_err(|_| RatingError::Unavailable)?
        .ok_or(RatingError::Incomplete)
}
fn scan<K, V>(
    ctx: &ModuleCtx<'_>,
    handle: &StateFamilyHandle<K, V>,
    mut visit: impl FnMut(&str, &V) -> Result<(), RatingError>,
) -> Result<(), RatingError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut after: Option<String> = None;
    loop {
        let page = ctx
            .scan_action_state(handle, after.as_deref(), 128)
            .map_err(|_| RatingError::Unavailable)?;
        for row in page.entries() {
            visit(row.encoded_key(), row.value())?;
        }
        let Some(next) = page.next_after() else {
            return Ok(());
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(RatingError::Invalid);
        }
        after = Some(next.to_string());
    }
}

fn read_txn<K, V>(
    txn: &mut ActionTxn<'_>,
    handle: &StateFamilyHandle<K, V>,
    key: &K,
) -> Result<V, RatingError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    txn.get(handle, key)
        .map_err(|_| RatingError::Unavailable)?
        .cloned()
        .ok_or(RatingError::Incomplete)
}
