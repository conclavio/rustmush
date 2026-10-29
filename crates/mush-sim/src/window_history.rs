//! Same-commit window record family and original journal joins. Registration
//! alone enables no provider, native publication, live route or eligibility.

use super::*;
use mush_core::locomotion::composed::{ComposedLocomotionReader, ComposedLocomotionReceipt};
use mush_core::{
    ActionTxn, CanonicalId, CommitToken, ModError, ModuleBootData, ModuleCtx, OwnerId,
    PreparedLedgerAppend, Registrar, ReplayWrite, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId,
};

pub const WINDOW_MOVEMENT_LOG: &str = "window_movement";
// Each journal body is at most 16 KiB. Even one chunk per commit, with its
// key/envelope overhead, stays within the 8 MiB original-history boot budget.
// A checkpoint/archival protocol is required before widening this horizon.
pub const MAX_WINDOW_MOVEMENT_COMMITS: u64 = 256;
const FAMILY: &str = "window_locomotion_crossings";
const FAMILY_BYTES: usize = 192 * 1024;
const HISTORY_BYTES: usize = 8 * 1024 * 1024;
const HISTORY_CHUNKS: usize = 4096;
const MAX_ROWS: usize = mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS;
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}

/// Real Sim registration and typed-state handles, never a replacement owner.
/// The actual provider retains native state/append bindings independently.
#[derive(Clone)]
pub struct WindowMovementHistory {
    records: StateFamilyHandle<CommitToken, WindowMovementRecord>,
}
impl WindowMovementHistory {
    pub fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        if reg.module() != crate::NS {
            return Err(bad(
                "window history must be registered by the actual Sim owner",
            ));
        }
        let id = |suffix| CanonicalId::new(format!("sim.{FAMILY}.{suffix}.v1")).map_err(bad);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(bad)?,
            id("key")?,
            id("value")?,
            1,
            id("decode")?,
            id("validate")?,
            id("canonical")?,
        )
        .and_then(|d| d.with_limits(128, FAMILY_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &CommitToken| Ok(key.to_string()),
            |value: &Value| WindowMovementRecord::decode(value).map_err(|e| e.to_string()),
            |record: &WindowMovementRecord| {
                record.evidence().map(|_| ()).map_err(|e| e.to_string())
            },
            |record: &WindowMovementRecord| record.encode().map_err(|e| e.to_string()),
        )?;
        Ok(Self { records })
    }
    /// Stage only an exact receipt/capture/prepared-entry join. Applying the
    /// actual native patch and acknowledging its complete bundle remain the
    /// real provider/kernel's responsibility; this installs nothing itself.
    pub fn append(
        &self,
        txn: &mut ActionTxn<'_>,
        receipt: &ComposedLocomotionReceipt,
        capture: &OrderedWindowMovementCapture,
        append: &PreparedLedgerAppend,
    ) -> Result<(), ParticipantError> {
        let record = WindowMovementRecord::for_receipt(receipt, capture, append.seq())?;
        check_append(&record, append)?;
        if txn
            .get(&self.records, &receipt.commit())
            .map_err(|_| refused())?
            .is_some()
        {
            return Err(refused());
        }
        txn.insert(&self.records, &receipt.commit(), record)
            .map_err(|_| refused())
    }
    /// Both directions are required, including zero-crossing/no-native-phase
    /// captures. A different receipt profile cannot stand in for the v2 one.
    pub fn audit_typed(
        &self,
        ctx: &ModuleCtx<'_>,
        reader: &ComposedLocomotionReader,
    ) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0_usize;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(|_| refused())?;
            for entry in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > MAX_ROWS || entry.encoded_key() != entry.value().commit().to_string() {
                    return Err(refused());
                }
                let receipt = reader
                    .read_receipt(ctx, &entry.value().commit())?
                    .ok_or_else(refused)?;
                entry.value().matches_receipt(&receipt)?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                break;
            }
        }
        cursor = None;
        count = 0;
        loop {
            let page = reader.scan_receipts(ctx, cursor.as_deref(), 128)?;
            for receipt in page.entries() {
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > MAX_ROWS {
                    return Err(refused());
                }
                ctx.action_state(&self.records, &receipt.commit())
                    .map_err(|_| refused())?
                    .ok_or_else(refused)?
                    .matches_receipt(receipt)?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
    /// Original journal entries must match their exact retained records and
    /// genuine receipts. Nothing reclassifies current geometry or reconstructs
    /// current placement from history. This is not continuous aboard evidence.
    pub fn audit_native(
        &self,
        ctx: &ModuleCtx<'_>,
        data: &ModuleBootData,
        reader: &ComposedLocomotionReader,
    ) -> Result<(), ModError> {
        self.audit_typed(ctx, reader).map_err(bad)?;
        if data
            .omitted
            .get(WINDOW_MOVEMENT_LOG)
            .is_some_and(|floor| *floor > 0)
        {
            return Err(ModError::RetryFullFetch {
                module: crate::NS.into(),
                reason: "window movement needs its original journal positions".into(),
            });
        }
        let seen = journal_commits(data, |commit| {
            ctx.action_state(&self.records, commit).map_err(bad)
        })?;
        let mut cursor = None;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(bad)?;
            for entry in page.entries() {
                if !seen.contains(&entry.value().commit()) {
                    return Err(bad(
                        "window movement record has no original native journal entry",
                    ));
                }
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}

// Passive journal/record matching only. The public audit first requires the
// independent genuine Core receipt join, then checks for typed orphans too.
pub(super) fn journal_commits<'a>(
    data: &ModuleBootData,
    record: impl Fn(&CommitToken) -> Result<Option<&'a WindowMovementRecord>, ModError>,
) -> Result<BTreeSet<CommitToken>, ModError> {
    let docs = journal_docs(data)?;
    let history = mush_core::inspect_ledger(&docs, None);
    if !history.is_clean() {
        return Err(bad("window movement journal is not complete"));
    }
    let mut seen = BTreeSet::new();
    for (_, position) in history.chunk_docs(WINDOW_MOVEMENT_LOG) {
        let (_, text) = docs
            .get(position)
            .ok_or_else(|| bad("window journal index is inconsistent"))?;
        let chunk = mush_core::ledger::parse_chunk(text).map_err(bad)?;
        for entry in chunk.entries {
            if seen.len() >= MAX_ROWS {
                return Err(bad("window movement journal exceeds its record bound"));
            }
            let value = entry
                .value
                .get("commit")
                .ok_or_else(|| bad("window journal commit is missing"))?;
            mush_core::canonical_json_bytes_with_limit(value, 128).map_err(bad)?;
            let commit: CommitToken = serde_json::from_value(value.clone()).map_err(bad)?;
            if !seen.insert(commit) {
                return Err(bad("window journal repeats a committed movement"));
            }
            let record = record(&commit)?
                .ok_or_else(|| bad("window journal has no original typed movement record"))?;
            record
                .matches_journal_entry(entry.seq, &entry.value)
                .map_err(bad)?;
        }
    }
    Ok(seen)
}

pub(super) fn check_append(
    record: &WindowMovementRecord,
    append: &PreparedLedgerAppend,
) -> Result<(), ParticipantError> {
    if append.seq() >= MAX_WINDOW_MOVEMENT_COMMITS {
        return Err(ParticipantError::ImageLimit);
    }
    let write = append
        .authoritative_replay_write(
            OwnerId::new(crate::NS).map_err(|_| ParticipantError::Authority)?,
        )
        .map_err(|_| ParticipantError::Encoding)?;
    let ReplayWrite::LedgerChunkSet { write } = write else {
        return Err(refused());
    };
    if write.log_id() != WINDOW_MOVEMENT_LOG {
        return Err(refused());
    }
    let entry = write.chunk().entries.last().ok_or_else(refused)?;
    record.matches_journal_entry(entry.seq, &entry.value)
}

pub(super) fn journal_docs(data: &ModuleBootData) -> Result<Vec<mush_core::LedgerDoc>, ModError> {
    let prefix = format!("log:{WINDOW_MOVEMENT_LOG}:");
    if data.kv.iter().any(|(key, _)| key.starts_with(&prefix)) {
        return Err(bad(
            "window journal data must be normalized before its audit",
        ));
    }
    let mut remaining = HISTORY_BYTES;
    let mut docs = Vec::new();
    for (key, text) in &data.ledger {
        if !key.starts_with(&prefix) {
            continue;
        }
        if key.len() > 128 {
            return Err(bad("window journal key exceeds its bound"));
        }
        let (log, chunk) = mush_core::ledger::parse_chunk_key(key)
            .ok_or_else(|| bad("window journal key is malformed"))?;
        if log.as_str() != WINDOW_MOVEMENT_LOG
            || key.as_str() != mush_core::chunk_key(WINDOW_MOVEMENT_LOG, chunk).as_str()
        {
            return Err(bad("window journal key is not canonical"));
        }
        remaining = remaining
            .checked_sub(key.len())
            .and_then(|bytes| bytes.checked_sub(text.len()))
            .ok_or_else(|| bad("window journal exceeds its byte bound"))?;
        if docs.len() >= HISTORY_CHUNKS {
            return Err(bad("window journal exceeds its chunk bound"));
        }
        docs.push((key.clone(), text.clone()));
    }
    Ok(docs)
}
