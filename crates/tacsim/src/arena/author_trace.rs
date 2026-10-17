//! Bounded diagnostic retention, independent of simulation state and execution.
//!
//! This owner has no interpreter, world, persistence, source or memory handle.
//! Arena runtime calls it after deciding outcomes. Its methods never refuse an
//! action, charge a fee, draw randomness or alter the result that will land.
//! Neither this store nor its publication cursors belong in hash or snapshot.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::{AccountId, ObjId};
use serde::Serialize;
use smol_str::SmolStr;

use super::intents::IntentKind;
use super::mind::Landed;

/// The last 64 distinct thinks per arena hull.
pub const THINKS: usize = 64;
/// Same author line limits as the sandbox; also checked at retention boundary.
pub const AUTHOR_LINES: usize = 8;
/// Unicode scalar limit, with controls replaced by spaces before publication.
pub const LINE_CHARS: usize = 120;
/// Detailed kernel rows; later calls increment an explicit omitted counter.
pub const KERNEL_CALLS: usize = 32;
/// Hard class III rules cap.
pub const RULE_ROWS: usize = 6;
/// Native and think terms share this per-rule cap.
pub const RULE_TERMS: usize = 4;
/// Already the runtime's emission ceiling, rechecked on diagnostics input.
pub const INTENTS: usize = 16;
/// One update batch at most per second of arena time.
pub const PUBLISH_MS: i64 = 1_000;
/// Protocol version for the separate arena.mind diagnostic payload.
pub const TRACE_VERSION: u32 = 1;

/// Counts tell readers exactly which bounded collections lost detail.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Omitted {
    pub author_lines: u32,
    pub author_chars: u32,
    pub kernel_calls: u32,
    pub rules: u32,
    pub rule_terms: u32,
    pub detail_chars: u32,
    pub intents: u32,
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
fn line(text: &str, omitted: &mut u32) -> String {
    let mut value = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index < LINE_CHARS {
            value.push(if ch.is_control() { ' ' } else { ch });
        } else {
            *omitted = omitted.saturating_add(1);
        }
    }
    value
}

/// Closed kernel names prevent an error value/source fragment becoming a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kernel {
    BestHostile,
    Odds,
    Aspect,
    Contact,
    IsDry,
    WouldOverheat,
    FightLimit,
    HoldsHostile,
    Health,
    Ranges,
    LineTo,
    EntryCell,
    FarEntryCell,
    StandOffCell,
    PatrolStep,
    FallBackToWaypoint,
    Rand,
    RulesCurrent,
    StockThink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KernelCall {
    pub kernel: Kernel,
    pub fee: u64,
}

/// A rule's public fact, already projected from own state/earned plot by caller.
/// None means that fact is unavailable, never guessed from target truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleTerm {
    pub description: String,
    pub value: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleTruth {
    pub slot: u8,
    pub description: String,
    pub terms: Vec<RuleTerm>,
    pub matches: Option<bool>,
}

/// Runner-side builder. Fields stay private so producers cannot bypass bounds.
/// A default is unmeasured, e.g. native ROM, never a false zero-spend claim.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ThinkDiagnostics {
    ops: Option<u64>,
    fees: Option<u64>,
    kernels: Vec<KernelCall>,
    rules: Vec<RuleTruth>,
    omitted: Omitted,
}

impl ThinkDiagnostics {
    /// Measured runner totals; native/unmeasured work remains explicitly absent.
    #[must_use]
    pub fn counts(&self) -> Option<(u64, u64)> {
        self.ops.zip(self.fees)
    }
    #[must_use]
    pub fn measured(ops: u64, fees: u64) -> Self {
        Self {
            ops: Some(ops),
            fees: Some(fees),
            ..Self::default()
        }
    }

    /// Set measured totals after execution; detail collection never charges.
    pub fn totals(&mut self, ops: u64, fees: u64) {
        self.ops = Some(ops);
        self.fees = Some(fees);
    }

    pub fn kernel(&mut self, kernel: Kernel, fee: u64) {
        if self.kernels.len() < KERNEL_CALLS {
            self.kernels.push(KernelCall { kernel, fee });
        } else {
            self.omitted.kernel_calls = self.omitted.kernel_calls.saturating_add(1);
        }
    }

    /// One final fact set per rule slot. Later sampling of a slot replaces that
    /// diagnostic row; it never modifies the complete live rule or its lease.
    pub fn rule<'a>(
        &mut self,
        slot: u8,
        description: &str,
        matches: Option<bool>,
        terms: impl IntoIterator<Item = (&'a str, Option<bool>)>,
    ) {
        if !(1..=6).contains(&slot) {
            self.omitted.rules = self.omitted.rules.saturating_add(1);
            return;
        }
        let description = line(description, &mut self.omitted.detail_chars);
        let mut values = Vec::new();
        for (description, value) in terms {
            if values.len() < RULE_TERMS {
                values.push(RuleTerm {
                    description: line(description, &mut self.omitted.detail_chars),
                    value,
                });
            } else {
                self.omitted.rule_terms = self.omitted.rule_terms.saturating_add(1);
            }
        }
        let row = RuleTruth {
            slot,
            description,
            terms: values,
            matches,
        };
        if let Some(old) = self.rules.iter_mut().find(|row| row.slot == slot) {
            *old = row;
        } else if self.rules.len() < RULE_ROWS {
            self.rules.push(row);
            self.rules.sort_by_key(|row| row.slot);
        } else {
            self.omitted.rules = self.omitted.rules.saturating_add(1);
        }
    }
}

/// Only coarse fault labels enter public traces. Raw interpreter errors may
/// name private source/memory and must stay behind owned-source authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    Ops,
    Limit,
    Type,
    Memory,
    Panic,
    Intents,
    Unknown,
}
impl Fault {
    #[must_use]
    pub fn from_cause(cause: &str) -> Self {
        match cause {
            "ops" => Self::Ops,
            "limit" => Self::Limit,
            "type" => Self::Type,
            "memory" => Self::Memory,
            "panic" => Self::Panic,
            "intents" => Self::Intents,
            _ => Self::Unknown,
        }
    }
}

/// Runtime-owned reason for switching to the native ROM after this think.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RomDrop {
    Core,
    Strikes,
}

/// A completed think can be cancelled before landing without a script fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cancelled {
    Epoch,
    Controller,
    Wreck,
    Ended,
    Removed,
    Superseded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Applied,
    Refused { reason: SmolStr },
    Faulted { fault: Fault },
    Cancelled { reason: Cancelled },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IntentTrace {
    /// A closed runtime IntentKind::as_str word, never Debug of raw arguments.
    pub action: SmolStr,
    /// Optional public-view argument rendering, never raw source, memory or IDs.
    pub detail: Option<String>,
    pub outcome: Outcome,
}

/// Safe public record: deliberately no source, memory, raw fault text or actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThinkTrace {
    pub settled: bool,
    pub fault: Option<Fault>,
    pub rom_drop: Option<RomDrop>,
    pub think_seq: u64,
    pub t_ms: i64,
    pub landed_ms: Option<i64>,
    pub rom: bool,
    pub diagnostics: ThinkDiagnostics,
    pub author: Vec<String>,
    pub intents: Vec<IntentTrace>,
    pub outcome: Outcome,
}

impl ThinkTrace {
    fn new(
        think_seq: u64,
        t_ms: i64,
        rom: bool,
        diagnostics: Option<ThinkDiagnostics>,
        author: &[String],
        intents: &[IntentKind],
        fault: Option<&str>,
    ) -> Self {
        let mut diagnostics = diagnostics.unwrap_or_default();
        diagnostics.omitted.author_lines = count(author.len().saturating_sub(AUTHOR_LINES));
        diagnostics.omitted.intents = count(intents.len().saturating_sub(INTENTS));
        let author = author
            .iter()
            .take(AUTHOR_LINES)
            .map(|text| line(text, &mut diagnostics.omitted.author_chars))
            .collect();
        let outcome = fault.map_or(Outcome::Pending, |fault| Outcome::Faulted {
            fault: Fault::from_cause(fault),
        });
        let intents = intents
            .iter()
            .take(INTENTS)
            .map(|kind| IntentTrace {
                action: kind.as_str().into(),
                detail: None,
                outcome: outcome.clone(),
            })
            .collect();
        Self {
            settled: true,
            fault: fault.map(Fault::from_cause),
            rom_drop: None,
            think_seq,
            t_ms,
            landed_ms: None,
            rom,
            diagnostics,
            author,
            intents,
            outcome,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Ring {
    rows: VecDeque<ThinkTrace>,
    dirty: BTreeSet<u64>,
}

/// A bounded delta to the world's read-only mirror. The retention floor evicts
/// records that fell out even when no newer row in that delta needs insertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceUpdate {
    pub unit: ObjId,
    pub first_seq: u64,
    pub records: Vec<ThinkTrace>,
}

/// Arena-local RAM only. It is deliberately not serializable/deserializable.
#[derive(Clone, Debug, Default)]
pub struct TraceStore {
    units: BTreeMap<ObjId, Ring>,
    published_at: Option<i64>,
    omitted_units: u64,
    disabled: bool,
}

impl TraceStore {
    /// Diagnostic collection can be disabled without touching execution. Turning
    /// it off discards retained diagnostics and publication state only.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.disabled = !enabled;
        if !enabled {
            self.units.clear();
            self.published_at = None;
        }
    }
    /// Bounded pending joins, for diagnostic cancellation at lifecycle boundaries.
    pub(crate) fn pending(&self) -> Vec<(ObjId, u64)> {
        self.units
            .iter()
            .flat_map(|(unit, ring)| {
                ring.rows
                    .iter()
                    .filter(|row| matches!(row.outcome, Outcome::Pending))
                    .map(|row| (*unit, row.think_seq))
            })
            .collect()
    }

    /// An arena-side validation failure, such as intent overflow. A stale answer
    /// remains cancelled even if the returned answer also has a script fault.
    pub(crate) fn mark_fault(&mut self, unit: ObjId, seq: u64, cause: &str) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == seq) else {
            return false;
        };
        let fault = Fault::from_cause(cause);
        row.fault = Some(fault);
        if matches!(row.outcome, Outcome::Pending) {
            row.outcome = Outcome::Faulted { fault };
            for intent in &mut row.intents {
                intent.outcome = row.outcome.clone();
            }
        }
        ring.dirty.insert(seq);
        true
    }

    /// A nonblocking egress queue refused this batch. Only still-retained rows
    /// become dirty again; the one-second publication clock never rewinds.
    pub(crate) fn retry_updates(&mut self, updates: &[TraceUpdate]) {
        for update in updates {
            if let Some(ring) = self.units.get_mut(&update.unit) {
                for row in &update.records {
                    if ring.rows.iter().any(|held| held.think_seq == row.think_seq) {
                        ring.dirty.insert(row.think_seq);
                    }
                }
            }
        }
    }

    /// Final transport flush after match end. Client feed publication is still
    /// throttled by the world consumer; this prevents the last fraction of a
    /// second being lost when the arena task exits.
    pub(crate) fn finish_updates(&mut self, at_ms: i64) -> Vec<TraceUpdate> {
        self.published_at = None;
        self.take_updates(at_ms)
    }

    /// Allocate the row when collection captures the public view, before a
    /// worker can race a controller/epoch cancellation. In-flight cancellation
    /// therefore has a diagnostic target even before the answer returns.
    pub fn begin(&mut self, unit: ObjId, think_seq: u64, t_ms: i64, rom: bool) -> bool {
        if self.disabled {
            return false;
        }
        if !self.units.contains_key(&unit)
            && self.units.len() >= crate::definitions::arena::MAX_ARENA_UNITS
        {
            self.omitted_units = self.omitted_units.saturating_add(1);
            return false;
        }
        let ring = self.units.entry(unit).or_default();
        if ring
            .rows
            .back()
            .is_some_and(|last| last.think_seq >= think_seq)
        {
            return false;
        }
        let mut row = ThinkTrace::new(think_seq, t_ms, rom, None, &[], &[], None);
        row.settled = false;
        if ring.rows.len() == THINKS
            && let Some(old) = ring.rows.pop_front()
        {
            ring.dirty.remove(&old.think_seq);
        }
        ring.dirty.insert(think_seq);
        ring.rows.push_back(row);
        true
    }

    /// Attach a worker answer exactly once, including a late answer to a think
    /// already cancelled. Its data cannot reverse cancellation or grant landing.
    pub fn complete(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        diagnostics: Option<ThinkDiagnostics>,
        author: &[String],
        intents: &[IntentKind],
        fault: Option<&str>,
    ) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == think_seq) else {
            return false;
        };
        if row.settled {
            return false;
        }
        let mut ready = ThinkTrace::new(
            think_seq,
            row.t_ms,
            row.rom,
            diagnostics,
            author,
            intents,
            fault,
        );
        if matches!(row.outcome, Outcome::Cancelled { .. }) {
            ready.outcome = row.outcome.clone();
            ready.landed_ms = row.landed_ms;
            for intent in &mut ready.intents {
                intent.outcome = ready.outcome.clone();
            }
        }
        *row = ready;
        ring.dirty.insert(think_seq);
        true
    }

    /// Convenience for synchronous producers/tests; production async collection
    /// uses begin/complete so cancellations while the worker runs are retained.
    #[allow(clippy::too_many_arguments)] // Combines the exact begin/complete inputs without copying trace data.
    pub fn record(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        t_ms: i64,
        rom: bool,
        diagnostics: Option<ThinkDiagnostics>,
        author: &[String],
        intents: &[IntentKind],
        fault: Option<&str>,
    ) -> bool {
        self.begin(unit, think_seq, t_ms, rom)
            && self.complete(unit, think_seq, diagnostics, author, intents, fault)
    }

    /// Attach actual outcomes in original emission order. No lookup by "latest"
    /// is permitted: a delayed landing must join its exact unit and sequence.
    pub fn land(&mut self, unit: ObjId, think_seq: u64, at_ms: i64, landed: &[Landed]) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == think_seq) else {
            return false;
        };
        if !row.settled
            || !matches!(row.outcome, Outcome::Pending)
            || landed.len() != row.intents.len()
        {
            return false;
        }
        for (intent, outcome) in row.intents.iter_mut().zip(landed) {
            intent.outcome = match outcome {
                Landed::Applied => Outcome::Applied,
                Landed::Refused(reason) => Outcome::Refused {
                    reason: reason.as_str().into(),
                },
            };
        }
        row.landed_ms = Some(at_ms);
        // A completed think may contain individual refusals; its completion is
        // still distinct from a script fault or a whole-think cancellation.
        row.outcome = Outcome::Applied;
        ring.dirty.insert(think_seq);
        true
    }

    pub fn cancel(&mut self, unit: ObjId, think_seq: u64, at_ms: i64, reason: Cancelled) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == think_seq) else {
            return false;
        };
        if !matches!(row.outcome, Outcome::Pending) {
            return false;
        }
        row.outcome = Outcome::Cancelled { reason };
        for intent in &mut row.intents {
            intent.outcome = row.outcome.clone();
        }
        row.landed_ms = Some(at_ms);
        ring.dirty.insert(think_seq);
        true
    }

    /// Attach a public argument rendering from the captured MindView, e.g.
    /// heading/speed/cell or the earned contact designator. Never pass Debug of
    /// IntentKind, a raw ContactId/ObjId or an interpreter error to this seam.
    pub fn intent_detail(
        &mut self,
        unit: ObjId,
        think_seq: u64,
        index: usize,
        detail: &str,
    ) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == think_seq) else {
            return false;
        };
        let Some(intent) = row.intents.get_mut(index) else {
            return false;
        };
        intent.detail = Some(line(detail, &mut row.diagnostics.omitted.detail_chars));
        ring.dirty.insert(think_seq);
        true
    }

    /// Mark the exact think whose fault strike or post-landing core loss
    /// switched the hull to ROM. It does not change that think's own outcome.
    pub fn drop_to_rom(&mut self, unit: ObjId, think_seq: u64, reason: RomDrop) -> bool {
        let Some(ring) = self.units.get_mut(&unit) else {
            return false;
        };
        let Some(row) = ring.rows.iter_mut().find(|row| row.think_seq == think_seq) else {
            return false;
        };
        if !row.settled || matches!(row.outcome, Outcome::Pending) {
            return false;
        }
        row.rom_drop = Some(reason);
        ring.dirty.insert(think_seq);
        true
    }

    /// Host-only read. World adapters MUST perform TraceAccess authorization
    /// against current authenticated state before sending any rows to a client.
    #[must_use]
    pub fn recent(&self, unit: ObjId, n: Option<usize>) -> Vec<ThinkTrace> {
        let n = n.unwrap_or(3).clamp(1, THINKS);
        self.units.get(&unit).map_or_else(Vec::new, |ring| {
            ring.rows
                .iter()
                .skip(ring.rows.len().saturating_sub(n))
                .cloned()
                .collect()
        })
    }

    /// Changes only; no poll can produce more than one batch per sim second.
    /// A backwards clock cannot reset the limit. Fresh/resumed arenas own a new
    /// empty store, so their publication clock starts independently.
    pub fn take_updates(&mut self, at_ms: i64) -> Vec<TraceUpdate> {
        if self
            .published_at
            .is_some_and(|last| at_ms.saturating_sub(last) < PUBLISH_MS)
        {
            return Vec::new();
        }
        let mut updates = Vec::new();
        for (unit, ring) in &mut self.units {
            if ring.dirty.is_empty() {
                continue;
            }
            let first_seq = ring.rows.front().map_or(0, |row| row.think_seq);
            let records = ring
                .rows
                .iter()
                .filter(|row| ring.dirty.contains(&row.think_seq))
                .cloned()
                .collect();
            ring.dirty.clear();
            updates.push(TraceUpdate {
                unit: *unit,
                first_seq,
                records,
            });
        }
        if !updates.is_empty() {
            self.published_at = Some(at_ms);
        }
        updates
    }

    /// An internal arena invariant overflow is explicit even though execution
    /// proceeds. Production arenas already enforce MAX_ARENA_UNITS.
    #[must_use]
    pub const fn omitted_units(&self) -> u64 {
        self.omitted_units
    }
}

/// The bridge retains this bounded mirror, not worker state or private source.
/// Egress is trusted server output; external clients cannot construct updates.
#[derive(Clone, Debug, Default)]
pub struct TraceMirror {
    units: BTreeMap<ObjId, VecDeque<ThinkTrace>>,
}
impl TraceMirror {
    pub fn apply(&mut self, update: TraceUpdate) {
        if !self.units.contains_key(&update.unit)
            && self.units.len() >= crate::definitions::arena::MAX_ARENA_UNITS
        {
            return;
        }
        let rows = self.units.entry(update.unit).or_default();
        rows.retain(|row| row.think_seq >= update.first_seq);
        for row in update.records.into_iter().take(THINKS) {
            if row.think_seq < update.first_seq {
                continue;
            }
            if let Some(old) = rows.iter_mut().find(|old| old.think_seq == row.think_seq) {
                *old = row;
            } else {
                rows.push_back(row);
            }
        }
        rows.make_contiguous().sort_by_key(|row| row.think_seq);
        while rows.len() > THINKS {
            rows.pop_front();
        }
    }

    /// One byte-identical absence for missing, disconnected and unauthorized.
    /// A client-supplied unit may select a row but never supplies these grants.
    #[must_use]
    pub fn read(
        &self,
        unit: ObjId,
        n: Option<usize>,
        access: &TraceAccess,
    ) -> Option<Vec<ThinkTrace>> {
        if !access.permits(unit) {
            return None;
        }
        let rows = self.units.get(&unit)?;
        let n = n.unwrap_or(3).clamp(1, THINKS);
        Some(
            rows.iter()
                .skip(rows.len().saturating_sub(n))
                .cloned()
                .collect(),
        )
    }
}

/// Facts resolved by the world adapter afresh on EVERY push/query/line read.
/// Never deserialize them or fill them from request owner/account/principal ids.
/// A matching current fill setter applies only to that unit's own fill choice;
/// merely being the same side's pilot, a watcher or a new lead grants nothing.
#[derive(Clone, Debug)]
pub struct TraceAccess {
    pub unit: ObjId,
    pub actor: ObjId,
    pub account: Option<AccountId>,
    pub connected: bool,
    pub staff: bool,
    pub sealed_principal: Option<ObjId>,
    pub sealed_owner: Option<ObjId>,
    pub sealed_warrant_principal: Option<ObjId>,
    pub current_fill_setter: Option<AccountId>,
}
impl TraceAccess {
    #[must_use]
    pub fn permits(&self, unit: ObjId) -> bool {
        self.unit == unit
            && self.connected
            && (self.staff
                || [
                    self.sealed_principal,
                    self.sealed_owner,
                    self.sealed_warrant_principal,
                ]
                .into_iter()
                .flatten()
                .any(|actor| actor == self.actor)
                || self
                    .account
                    .zip(self.current_fill_setter)
                    .is_some_and(|(actor, setter)| actor == setter))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn refused_egress_retries_latest_rows_without_rewinding_the_throttle() {
        let mut store = super::TraceStore::default();
        let unit = mush_core::ObjId(4);
        store.begin(unit, 0, 0, false);
        let first = store.take_updates(0);
        assert_eq!(first.len(), 1);
        store.retry_updates(&first);
        store.complete(unit, 0, None, &[], &[], None);
        store.land(unit, 0, 50, &[]);
        assert!(store.take_updates(999).is_empty());
        let retry = store.take_updates(1000);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].records[0].outcome, super::Outcome::Applied);
        store.begin(unit, 1, 1000, false);
        assert!(store.take_updates(1000).is_empty());
        assert_eq!(store.finish_updates(1000)[0].records[0].think_seq, 1);
    }

    #[test]
    fn arena_fault_annotation_preserves_cancelled_authority_and_measured_totals() {
        let mut store = super::TraceStore::default();
        let unit = mush_core::ObjId(4);
        store.begin(unit, 0, 0, false);
        store.cancel(unit, 0, 1, super::Cancelled::Epoch);
        store.complete(
            unit,
            0,
            Some(super::ThinkDiagnostics::measured(7, 3)),
            &[],
            &[],
            None,
        );
        store.mark_fault(unit, 0, "intents");
        let row = store.recent(unit, None).pop().expect("row");
        assert_eq!(row.diagnostics.counts(), Some((7, 3)));
        assert_eq!(row.fault, Some(super::Fault::Intents));
        assert!(matches!(
            row.outcome,
            super::Outcome::Cancelled {
                reason: super::Cancelled::Epoch
            }
        ));
        assert_eq!(super::ThinkDiagnostics::default().counts(), None);
    }
    use super::super::intents::Refusal;
    use super::*;

    fn record(store: &mut TraceStore, unit: u32, seq: u64) {
        assert!(store.record(
            ObjId(unit),
            seq,
            i64::try_from(seq).unwrap(),
            false,
            Some(ThinkDiagnostics::measured(seq, 20)),
            &[],
            &[IntentKind::Startup],
            None
        ));
    }
    fn access(actor: u32) -> TraceAccess {
        TraceAccess {
            unit: ObjId(1),
            actor: ObjId(actor),
            account: None,
            connected: true,
            staff: false,
            sealed_principal: Some(ObjId(10)),
            sealed_owner: None,
            sealed_warrant_principal: None,
            current_fill_setter: None,
        }
    }

    #[test]
    fn bounded_retention_and_exact_outcome_joins_do_not_cross_units_or_thinks() {
        let mut store = TraceStore::default();
        for seq in 1..=70 {
            record(&mut store, 1, seq);
        }
        record(&mut store, 2, 70);
        assert_eq!(store.recent(ObjId(1), Some(1000)).len(), THINKS);
        assert_eq!(
            store.recent(ObjId(1), Some(64)).first().unwrap().think_seq,
            7
        );
        assert!(!store.land(ObjId(1), 6, 100, &[Landed::Applied]));
        assert!(store.land(ObjId(1), 69, 100, &[Landed::Refused(Refusal::Epoch)]));
        assert!(store.land(ObjId(2), 70, 100, &[Landed::Applied]));
        let one = store.recent(ObjId(1), None);
        assert!(matches!(one.last().unwrap().outcome, Outcome::Pending));
        assert!(
            matches!(&one.get(1).unwrap().intents.first().unwrap().outcome,
            Outcome::Refused { reason } if reason == "epoch")
        );
        assert!(!store.land(ObjId(1), 69, 200, &[Landed::Applied]));
        assert!(!store.record(ObjId(1), 69, 0, false, None, &[], &[], None));
        assert!(matches!(
            store.recent(ObjId(2), None).first().unwrap().outcome,
            Outcome::Applied
        ));
    }

    #[test]
    fn every_collection_truncates_explicitly_without_changing_measured_totals() {
        let mut diag = ThinkDiagnostics::measured(999, 888);
        for _ in 0..1000 {
            diag.kernel(Kernel::Contact, 1);
        }
        for slot in 1..=8 {
            diag.rule(
                slot,
                &"x".repeat(200),
                Some(false),
                (0..9).map(|_| ("own public fact", Some(true))),
            );
        }
        let author = vec![format!("\u{1b}{}", "é".repeat(200)); 10];
        let mut store = TraceStore::default();
        assert!(store.record(
            ObjId(1),
            1,
            0,
            false,
            Some(diag),
            &author,
            &vec![IntentKind::Startup; 20],
            Some("private memory / source in an error")
        ));
        let row = store.recent(ObjId(1), None).pop().unwrap();
        assert_eq!(row.diagnostics.ops, Some(999));
        assert_eq!(row.diagnostics.fees, Some(888));
        assert_eq!(row.diagnostics.kernels.len(), KERNEL_CALLS);
        assert_eq!(row.diagnostics.omitted.kernel_calls, 968);
        assert_eq!(row.diagnostics.rules.len(), RULE_ROWS);
        assert!(
            row.diagnostics
                .rules
                .iter()
                .all(|rule| rule.terms.len() == RULE_TERMS)
        );
        assert_eq!(row.author.len(), AUTHOR_LINES);
        assert!(
            row.author
                .iter()
                .all(|line| line.chars().count() == LINE_CHARS && !line.contains('\u{1b}'))
        );
        assert_eq!(row.diagnostics.omitted.author_lines, 2);
        assert_eq!(row.diagnostics.omitted.intents, 4);
        assert_eq!(row.intents.len(), INTENTS);
        let json = serde_json::to_string(&row).unwrap();
        assert!(!json.contains("private memory") && !json.contains("source in an error"));
        assert!(matches!(
            row.outcome,
            Outcome::Faulted {
                fault: Fault::Unknown
            }
        ));
    }

    #[test]
    fn faults_cancelled_and_rom_are_distinct_and_never_falsely_land() {
        let mut store = TraceStore::default();
        assert!(store.record(ObjId(1), 1, 0, false, None, &[], &[], Some("ops")));
        assert!(!store.land(ObjId(1), 1, 1, &[]));
        assert!(store.drop_to_rom(ObjId(1), 1, RomDrop::Strikes));
        record(&mut store, 1, 2);
        assert!(store.cancel(ObjId(1), 2, 3, Cancelled::Controller));
        assert!(!store.land(ObjId(1), 2, 4, &[Landed::Applied]));
        assert!(store.record(ObjId(1), 3, 4, true, None, &[], &[], None));
        assert!(store.land(ObjId(1), 3, 5, &[]));
        let rows = store.recent(ObjId(1), None);
        assert!(matches!(
            rows.first().unwrap().outcome,
            Outcome::Faulted { fault: Fault::Ops }
        ));
        assert!(matches!(
            rows.get(1).unwrap().outcome,
            Outcome::Cancelled {
                reason: Cancelled::Controller
            }
        ));
        assert!(rows.last().unwrap().rom);
        assert_eq!(rows.last().unwrap().diagnostics.ops, None);
    }

    #[test]
    fn egress_is_once_per_sim_second_and_mirror_replaces_joined_rows() {
        let mut store = TraceStore::default();
        let mut mirror = TraceMirror::default();
        record(&mut store, 1, 1);
        for update in store.take_updates(0) {
            mirror.apply(update);
        }
        assert!(store.land(ObjId(1), 1, 50, &[Landed::Applied]));
        assert!(store.take_updates(-1000).is_empty());
        assert!(store.take_updates(999).is_empty());
        assert!(matches!(
            mirror
                .read(ObjId(1), None, &access(10))
                .unwrap()
                .first()
                .unwrap()
                .outcome,
            Outcome::Pending
        ));
        for update in store.take_updates(1000) {
            mirror.apply(update);
        }
        assert!(matches!(
            mirror
                .read(ObjId(1), None, &access(10))
                .unwrap()
                .first()
                .unwrap()
                .outcome,
            Outcome::Applied
        ));
        for seq in 2..=100 {
            record(&mut store, 1, seq);
        }
        for update in store.take_updates(2000) {
            mirror.apply(update);
        }
        let rows = mirror.read(ObjId(1), Some(64), &access(10)).unwrap();
        assert_eq!(rows.len(), THINKS);
        assert_eq!(rows.first().unwrap().think_seq, 37);
    }

    #[test]
    fn own_principal_current_fill_setter_and_staff_require_a_current_connection() {
        let mut grant = access(10);
        assert!(grant.permits(ObjId(1)));
        grant.actor = ObjId(11);
        assert!(!grant.permits(ObjId(1))); // another pilot/watcher has no authority
        grant.account = Some(AccountId::mint("three", 1).unwrap());
        grant.current_fill_setter = Some(AccountId::mint("three", 1).unwrap());
        assert!(grant.permits(ObjId(1)));
        grant.current_fill_setter = Some(AccountId::mint("four", 1).unwrap());
        assert!(!grant.permits(ObjId(1))); // changed fill ownership is checked anew
        grant.staff = true;
        assert!(grant.permits(ObjId(1)));
        grant.connected = false;
        assert!(!grant.permits(ObjId(1)));
        grant.connected = true;
        grant.staff = false;
        grant.sealed_owner = Some(ObjId(11));
        assert!(grant.permits(ObjId(1)));
    }

    #[test]
    fn cancellation_while_a_worker_runs_survives_its_late_fault() {
        let mut store = TraceStore::default();
        assert!(store.begin(ObjId(1), 1, 0, false));
        assert!(!store.land(ObjId(1), 1, 1, &[]));
        assert!(store.cancel(ObjId(1), 1, 5, Cancelled::Epoch));
        assert!(store.complete(
            ObjId(1),
            1,
            Some(ThinkDiagnostics::measured(50, 20)),
            &["late trace".into()],
            &[IntentKind::Startup],
            Some("ops")
        ));
        let row = store.recent(ObjId(1), None).pop().unwrap();
        assert!(row.settled);
        assert_eq!(row.fault, Some(Fault::Ops));
        assert_eq!(row.landed_ms, Some(5));
        assert!(matches!(
            row.outcome,
            Outcome::Cancelled {
                reason: Cancelled::Epoch
            }
        ));
        assert!(matches!(
            row.intents.first().unwrap().outcome,
            Outcome::Cancelled {
                reason: Cancelled::Epoch
            }
        ));
        assert!(!store.complete(ObjId(1), 1, None, &[], &[], None));
        assert!(
            !access(10).permits(ObjId(2)),
            "a grant for another hull cannot be reused"
        );
    }

    #[test]
    fn arena_unit_bound_prevents_instrumentation_growth() {
        let mut store = TraceStore::default();
        for unit in 1..=24 {
            record(&mut store, unit, 1);
        }
        assert!(!store.record(ObjId(25), 1, 0, false, None, &[], &[], None));
        assert_eq!(store.omitted_units(), 1);
        assert!(store.recent(ObjId(25), None).is_empty());
    }
}
