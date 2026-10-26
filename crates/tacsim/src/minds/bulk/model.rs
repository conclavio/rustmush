use super::conduct_budget::{ConductBudget, ConductCursor};
use super::registry_budget::{RegistryBudget, RegistryCursor};
use super::{BulkError, invalid};
use mush_core::{AccountId, CommitToken, ObjId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const FORMAT: u16 = 1;
pub(crate) const DAY_MS: i64 = 86_400_000;
pub(crate) const UNIT_MINUTE: u64 = 60_000;
pub(crate) const MAX_CHARACTERS: usize = 64;
pub(crate) const MAX_ACCOUNT_DOCUMENT: usize = 2 * 1024 * 1024;
pub(crate) const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_REFERENCE_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const LEASE_STEPS: u64 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BulkConfig {
    pub proving_workers: usize,
    pub queued: usize,
    pub daily_unit_minutes: u64,
    pub completed_per_character: usize,
    pub bytes_per_character: u64,
    pub bench_idle_ms: u64,
    pub bench_sessions: usize,
    pub bench_state_bytes: u64,
    pub eval_ops: u64,
    pub eval_result_bytes: usize,
}
impl Default for BulkConfig {
    fn default() -> Self {
        Self {
            proving_workers: 1,
            queued: 32,
            daily_unit_minutes: 2_000,
            completed_per_character: 20,
            bytes_per_character: 64 * 1024 * 1024,
            bench_idle_ms: 30 * 60 * 1_000,
            bench_sessions: 64,
            bench_state_bytes: 128 * 1024 * 1024,
            eval_ops: 500,
            eval_result_bytes: 4_096,
        }
    }
}
impl BulkConfig {
    pub fn validate(&self) -> Result<(), BulkError> {
        if !(1..=8).contains(&self.proving_workers)
            || !(1..=128).contains(&self.queued)
            || !(1..=100_000).contains(&self.daily_unit_minutes)
            || !(1..=100).contains(&self.completed_per_character)
            || !(MAX_ARTIFACT_BYTES..=256 * 1024 * 1024).contains(&self.bytes_per_character)
            || !(60_000..=30 * 60 * 1_000).contains(&self.bench_idle_ms)
            || !(1..=256).contains(&self.bench_sessions)
            || !(MAX_ARTIFACT_BYTES..=512 * 1024 * 1024).contains(&self.bench_state_bytes)
            || !(1..=500).contains(&self.eval_ops)
            || !(64..=4_096).contains(&self.eval_result_bytes)
        {
            return Err(invalid("compute limits out of range"));
        }
        Ok(())
    }
    pub fn daily_ms(&self) -> u64 {
        self.daily_unit_minutes * UNIT_MINUTE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RunId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scenario {
    Duel,
    Muster,
    Hold,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Opponent {
    Doctrine,
    Owned,
    Mirror,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkKind {
    Proving,
    Bench,
    Registry,
    Conduct,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Queued,
    Running,
    CancelRequested,
    Complete,
    Cancelled,
    Interrupted,
    Failed,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Cancelled | Self::Interrupted | Self::Failed
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    /// Raw-byte SHA for proving blobs; completed TapeMark-rooted identity for
    /// a historical bench reference. Plan.kind fixes which owner verifies it.
    pub sha256: [u8; 32],
    pub bytes: u64,
}
impl Artifact {
    pub fn validate(&self) -> Result<(), BulkError> {
        if self.bytes == 0 || self.bytes > MAX_REFERENCE_BYTES {
            return Err(invalid("artifact size"));
        }
        Ok(())
    }
}
/// Frozen expanded ground, opening custody, complete seals/config/source refs
/// and engine identity are in a separately hashed artifact. Admission accepts
/// this descriptor only after its producer has durably prepared those bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conduct: Option<ConductBudget>,
    pub kind: WorkKind,
    pub scenario: Scenario,
    pub opponent: Opponent,
    pub per_side: u8,
    pub unit_count: u16,
    /// Total games, even; seed i is played in the two side assignments.
    pub games: u16,
    pub seeds: Vec<u64>,
    pub limit_ms: u32,
    /// Captured native physics substep, never a transport-selected quantum.
    pub native_substep_ms: u32,
    pub start: Artifact,
    pub result_reservation_bytes: u64,
}
impl Plan {
    pub fn validate(&self) -> Result<(), BulkError> {
        self.start.validate()?;
        if let Some(registry) = &self.registry {
            registry.validate()?;
        }
        if let Some(conduct) = &self.conduct {
            conduct.validate()?;
        }
        let schedule_ok = match self.kind {
            WorkKind::Proving => {
                (1..=6).contains(&self.per_side)
                    && self.unit_count == u16::from(self.per_side) * 2
                    && (10..=100).contains(&self.games)
                    && self.games.is_multiple_of(2)
                    && self.seeds.len() == usize::from(self.games / 2)
            }
            WorkKind::Bench => {
                self.games == 1 && self.seeds.len() == 1 && (1..=300).contains(&self.unit_count)
            }
            WorkKind::Registry | WorkKind::Conduct => {
                ((self.kind == WorkKind::Registry && self.registry.is_some())
                    || (self.kind == WorkKind::Conduct && self.conduct.is_some()))
                    && self.per_side == 0
                    && self.unit_count == 0
                    && self.games == 0
                    && self.seeds.is_empty()
                    && self.limit_ms == 0
                    && self.native_substep_ms == 0
                    && self.scenario == Scenario::Duel
                    && self.opponent == Opponent::Doctrine
            }
        };
        if !schedule_ok
            || (self.kind != WorkKind::Registry && self.registry.is_some())
            || (self.kind != WorkKind::Conduct && self.conduct.is_some())
            || (self.kind != WorkKind::Bench && self.start.bytes > MAX_ARTIFACT_BYTES)
            || self
                .seeds
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.seeds.len()
            || (!matches!(self.kind, WorkKind::Registry | WorkKind::Conduct)
                && (self.native_substep_ms < 50 || self.limit_ms == 0))
            || (self.kind == WorkKind::Proving
                && !(1_000..=30 * 60 * 1_000).contains(&self.limit_ms))
            || !(1..=MAX_ARTIFACT_BYTES).contains(&self.result_reservation_bytes)
        {
            return Err(invalid("scenario dimensions, paired seeds or result bound"));
        }
        Ok(())
    }
    pub fn units(&self) -> u64 {
        u64::from(self.unit_count)
    }
    pub fn maximum_unit_ms(&self) -> u64 {
        if let Some(registry) = &self.registry {
            return registry.maximum_unit_ms();
        }
        if let Some(conduct) = &self.conduct {
            return conduct.maximum_unit_ms();
        }
        self.units()
            * (u64::from(self.limit_ms) + u64::from(self.native_substep_ms))
            * u64::from(self.games)
    }
    pub fn initial_charge(&self) -> Result<(u64, u32), BulkError> {
        if let Some(conduct) = &self.conduct {
            let case = conduct.case(ConductCursor::default())?;
            return Ok((u64::from(case.units), case.native_ms));
        }
        if let Some(registry) = &self.registry {
            let case = registry.case(RegistryCursor::default())?;
            Ok((u64::from(case.units), case.native_ms))
        } else {
            Ok((self.units(), self.native_substep_ms))
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Flight {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conduct: Option<ConductCursor>,
    pub sequence: u64,
    pub utc_day: i64,
    pub allowance_unit_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Run {
    /// Exact acknowledged Registry admission. Absent in every M6 run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_job: Option<CommitToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conduct_job: Option<CommitToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conduct: Option<ConductCursor>,
    pub id: RunId,
    pub character: ObjId,
    pub nonce: u64,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
    pub plan: Plan,
    pub status: Status,
    pub charged_unit_ms: u64,
    pub next_sequence: u64,
    pub flight: Option<Flight>,
    pub result: Option<Artifact>,
    pub pins: u8,
}
impl Run {
    pub fn charge(&self) -> Result<(u64, u32, u64), BulkError> {
        match (&self.plan.conduct, self.conduct) {
            (Some(budget), Some(cursor))
                if self.plan.registry.is_none() && self.registry.is_none() =>
            {
                let case = budget.case(cursor)?;
                return Ok((
                    u64::from(case.units),
                    case.native_ms,
                    case.limit_ms()?
                        .checked_sub(cursor.simulated_ms)
                        .ok_or_else(|| invalid("Conduct trial progress"))?,
                ));
            }
            (None, None) => {}
            _ => return Err(invalid("Conduct cursor/plan mismatch")),
        }
        match (&self.plan.registry, self.registry) {
            (Some(budget), Some(cursor)) => {
                let case = budget.case(cursor)?;
                Ok((
                    u64::from(case.units),
                    case.native_ms,
                    case.limit_ms()?
                        .checked_sub(cursor.simulated_ms)
                        .ok_or_else(|| invalid("Registry case progress"))?,
                ))
            }
            (None, None) => Ok((self.plan.units(), self.plan.native_substep_ms, u64::MAX)),
            _ => Err(invalid("Registry cursor/plan mismatch")),
        }
    }
    pub fn retained_bytes(&self) -> u64 {
        // Bench references belong to TapeStore and its independently bounded
        // session budget. Only private proving artifacts consume this owner.
        if self.plan.kind == WorkKind::Bench {
            return 0;
        }
        self.plan.start.bytes
            + self
                .result
                .as_ref()
                .map_or(self.plan.result_reservation_bytes, |r| r.bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Account {
    pub format: u16,
    pub id: AccountId,
    pub utc_day: i64,
    pub spent_unit_ms: u64,
    pub reserved_unit_ms: u64,
    pub request_watermark: u64,
    pub next_run: u64,
    pub active: Option<RunId>,
    pub runs: BTreeMap<RunId, Run>,
}
impl Account {
    pub fn empty(id: AccountId, day: i64) -> Self {
        Self {
            format: FORMAT,
            id,
            utc_day: day,
            spent_unit_ms: 0,
            reserved_unit_ms: 0,
            request_watermark: 0,
            next_run: 1,
            active: None,
            runs: BTreeMap::new(),
        }
    }
    /// The complete future document bound for an admitted Registry run. Every
    /// field that can grow before terminal publication is rendered at its
    /// maximal JSON width, preserving immutable plans and all other actual rows.
    /// This is an encoding bound only, never a simulated/authoritative post-image.
    pub(super) fn registry_document_ceiling(&self) -> Result<Option<usize>, BulkError> {
        let active_registry = self.runs.values().any(|run| {
            matches!(run.plan.kind, WorkKind::Registry | WorkKind::Conduct)
                && !run.status.terminal()
        });
        if !active_registry {
            return Ok(None);
        }
        let mut upper = self.clone();
        {
            upper.utc_day = i64::MIN;
            upper.spent_unit_ms = u64::MAX;
            upper.reserved_unit_ms = u64::MAX;
            upper.request_watermark = u64::MAX;
            upper.next_run = u64::MAX;
            upper.active = Some(RunId(u64::MAX));
            for run in upper.runs.values_mut().filter(|run| {
                matches!(run.plan.kind, WorkKind::Registry | WorkKind::Conduct)
                    && !run.status.terminal()
            }) {
                let cursor = RegistryCursor {
                    index: u16::MAX,
                    simulated_ms: u64::MAX,
                };
                let conduct_cursor = ConductCursor {
                    index: u8::MAX,
                    simulated_ms: u64::MAX,
                };
                let registry = (run.plan.kind == WorkKind::Registry).then_some(cursor);
                let conduct = (run.plan.kind == WorkKind::Conduct).then_some(conduct_cursor);
                run.registry = registry;
                run.conduct = conduct;
                run.finished_ms = Some(i64::MIN);
                run.status = Status::CancelRequested;
                run.charged_unit_ms = u64::MAX;
                run.next_sequence = u64::MAX;
                run.flight = Some(Flight {
                    registry,
                    conduct,
                    sequence: u64::MAX,
                    utc_day: i64::MIN,
                    allowance_unit_ms: u64::MAX,
                });
                run.result = Some(Artifact {
                    sha256: [255; 32],
                    bytes: u64::MAX,
                });
                run.pins = u8::MAX;
            }
        }
        serde_json::to_vec(&upper)
            .map(|bytes| Some(bytes.len()))
            .map_err(|_| invalid("account terminal capacity encoding"))
    }
    pub fn validate(&self, _cfg: &BulkConfig) -> Result<(), BulkError> {
        if self.format != FORMAT
            || self.next_run == 0
            || self.runs.len() > MAX_CHARACTERS * 101
            || self.spent_unit_ms.saturating_add(self.reserved_unit_ms) > 100_000 * UNIT_MINUTE
        {
            return Err(invalid("account quota/record dimensions"));
        }
        let mut active = None;
        let mut totals = BTreeMap::<ObjId, (usize, u64)>::new();
        let mut nonces = std::collections::BTreeSet::new();
        let mut registry_jobs = std::collections::BTreeSet::new();
        let mut conduct_jobs = std::collections::BTreeSet::new();
        for (id, run) in &self.runs {
            run.plan.validate()?;
            let (units, native, remaining) = run.charge()?;
            if run.status == Status::Complete {
                if let (Some(budget), Some(cursor)) = (&run.plan.conduct, run.conduct)
                    && !budget.complete(cursor)?
                {
                    return Err(invalid("Conduct completion at a partial trial"));
                }
                if let (Some(budget), Some(cursor)) = (&run.plan.registry, run.registry) {
                    // Complete is a quota-terminal result. Certification and
                    // measured rejection are distinguished by the actual
                    // artifact/Registry owners, including on paired recovery.
                    if !budget.can_certify(cursor)? && !budget.can_reject(cursor)? {
                        return Err(invalid("Registry completion at a partial class cursor"));
                    }
                }
            }
            if (run.plan.kind == WorkKind::Conduct) != run.conduct_job.is_some()
                || run.conduct_job.is_some_and(|job| !conduct_jobs.insert(job))
                || (run.plan.kind == WorkKind::Registry) != run.registry_job.is_some()
                || run
                    .registry_job
                    .is_some_and(|job| !registry_jobs.insert(job))
                || *id != run.id
                || id.0 == 0
                || run.nonce == 0
                || !nonces.insert(run.nonce)
                || run.status.terminal() != run.finished_ms.is_some()
                || (run.status == Status::Complete
                    && run.plan.kind != WorkKind::Bench
                    && run.result.is_none())
                || id.0 >= self.next_run
                || run.nonce > self.request_watermark
                || run.next_sequence == 0
                || run.charged_unit_ms > run.plan.maximum_unit_ms()
                || run.pins > 1
                || (run.status.terminal() && run.flight.is_some())
            {
                return Err(invalid("run identity/state"));
            }
            if let Some(result) = &run.result {
                result.validate()?;
                if result.bytes > run.plan.result_reservation_bytes {
                    return Err(invalid("result exceeded reservation"));
                }
            }
            if let Some(flight) = &run.flight
                && (flight.sequence.checked_add(1) != Some(run.next_sequence)
                    || flight.allowance_unit_ms == 0
                    || flight.registry != run.registry
                    || flight.conduct != run.conduct
                    || flight.allowance_unit_ms > units * u64::from(native) * LEASE_STEPS
                    || flight.allowance_unit_ms / units > remaining
                    || flight.allowance_unit_ms > run.charged_unit_ms
                    || flight.utc_day != self.utc_day)
            {
                return Err(invalid("flight allowance or cursor"));
            }
            if !run.status.terminal() && active.replace(*id).is_some() {
                return Err(invalid("multiple active runs"));
            }
            let total = totals.entry(run.character).or_default();
            total.0 += 1;
            total.1 = total.1.saturating_add(run.retained_bytes());
        }
        if active != self.active
            || (active.is_none() && self.reserved_unit_ms != 0)
            || totals.len() > MAX_CHARACTERS
            || totals
                .values()
                .any(|(count, bytes)| *count > 101 || *bytes > 256 * 1024 * 1024)
        {
            return Err(invalid("retention or active-run links"));
        }
        Ok(())
    }
}
pub(crate) fn utc_day(now_ms: i64) -> i64 {
    now_ms.div_euclid(DAY_MS)
}

/// Immutable bytes produced off-world, hash checked before the owner publishes
/// the descriptor/reference. This is never a source lookup or filesystem path.
#[derive(Clone, Debug)]
pub(crate) struct ArtifactBytes {
    descriptor: Artifact,
    bytes: std::sync::Arc<[u8]>,
    chunks: Vec<String>,
    summary: Vec<String>,
    context: Option<super::report::Context>,
}
impl ArtifactBytes {
    /// Preparation-worker serialization refuses before the immutable artifact
    /// allocation can exceed its existing byte ceiling.
    pub(crate) fn encode(value: &impl serde::Serialize) -> Result<Self, BulkError> {
        Self::new(super::encoding::encode(value, MAX_ARTIFACT_BYTES as usize)?)
    }
    pub fn new(bytes: Vec<u8>) -> Result<Self, BulkError> {
        use sha2::{Digest as _, Sha256};
        if bytes.is_empty() || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(invalid("immutable proving artifact size"));
        }
        let descriptor = Artifact {
            sha256: Sha256::digest(&bytes).into(),
            bytes: bytes.len() as u64,
        };
        descriptor.validate()?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| invalid("artifact is not UTF-8 JSON"))?;
        let mut chunks = Vec::new();
        let mut at = 0;
        while at < text.len() {
            let mut end = (at + 32 * 1024).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(text[at..end].to_owned());
            at = end;
        }
        Ok(Self {
            descriptor,
            bytes: bytes.into(),
            chunks,
            summary: Vec::new(),
            context: None,
        })
    }
    pub fn descriptor(&self) -> &Artifact {
        &self.descriptor
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Prepared off-world; publication moves these strings into JSON writes.
    pub(super) fn take_chunks(&mut self) -> Vec<String> {
        std::mem::take(&mut self.chunks)
    }
    pub(super) fn with_summary(mut self, summary: Vec<String>) -> Result<Self, BulkError> {
        if summary.len() > 200
            || summary
                .iter()
                .any(|s| s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(invalid("report rendering bound"));
        }
        self.summary = summary;
        Ok(self)
    }
    pub(super) fn with_context(
        mut self,
        context: super::report::Context,
    ) -> Result<Self, BulkError> {
        context.validate()?;
        self.context = Some(context);
        Ok(self)
    }
    pub(super) fn take_context(&mut self) -> Option<super::report::Context> {
        self.context.take()
    }
    pub(super) fn take_summary(&mut self) -> Vec<String> {
        std::mem::take(&mut self.summary)
    }
}
