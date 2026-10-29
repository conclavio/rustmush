//! Closed Registry evidence codecs. Structural data is never certification
//! authority: only the registered producer may publish these actual families.
//! Receipt resolution also requires complete pages, completion, current game
//! compatibility and the ordered all-loaded source/root audit.

pub mod reconformance;

use std::{fmt, marker::PhantomData};

use mush_core::{ClockStamp, CommitToken, mechanics::DefinitionDigest};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, SeqAccess, Visitor},
};

use super::{RevisionIdentity, WorldIcSeconds};

pub mod execution;
pub mod families;
pub mod nonconformance;
pub(crate) mod producer;
pub mod protocol;
pub mod registration;
pub mod score;
pub mod service_read;
pub use families::RatingFamilies;
pub use score::{RatingScore, RatingTally, rating_score, score_per_credit};

pub const FORMAT: u16 = 1;
pub const MAX_CASES: usize = 1_755;
pub const MAX_PAGE_ROWS: usize = 15;
pub const MAX_PAGES: usize = 117;
pub const MAX_CASE_BYTES: usize = 2 * 1024;
pub const MAX_PAGE_BYTES: usize = 32 * 1024;
pub const MAX_RECEIPT_BYTES: usize = 64 * 1024;
pub const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// Reserve framing for the complete typed record, not just its value JSON.
pub const TYPED_ENVELOPE_RESERVE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RatingError {
    #[error("invalid or unsupported Registry evidence")]
    Invalid,
    #[error("Registry evidence exceeds its bounded representation")]
    Bound,
    #[error("Registry evidence digest or key does not match")]
    Identity,
    #[error("Registry evidence is incomplete or inconsistent")]
    Incomplete,
    #[error("the actual Registry family or source is unavailable")]
    Unavailable,
}

/// Bounds allocation during typed sequence decoding as well as after it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Rows<T, const N: usize>(Vec<T>);
impl<T, const N: usize> Rows<T, N> {
    fn new(items: Vec<T>) -> Result<Self, RatingError> {
        if items.len() > N {
            return Err(RatingError::Bound);
        }
        Ok(Self(items))
    }
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Rows<T, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Bounded<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Bounded<T, N> {
            type Value = Rows<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} entries")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|n| n > N) {
                    return Err(de::Error::custom(RatingError::Bound));
                }
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
                while let Some(item) = seq.next_element()? {
                    if items.len() == N {
                        return Err(de::Error::custom(RatingError::Bound));
                    }
                    items.push(item);
                }
                Ok(Rows(items))
            }
        }
        d.deserialize_seq(Bounded::<T, N>(PhantomData))
    }
}

// Each value validates even through ordinary serde, not only helper decoders.
macro_rules! record {
    ($name:ident, $wire:ident, {$($(#[$attribute:meta])* $field:ident: $ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $($field: $ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($(#[$attribute])* $field: $ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let wire = $wire::deserialize(d)?;
                let value = Self { $($field: wire.$field),* };
                value.validate().map_err(de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

macro_rules! digest_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(DefinitionDigest);
        impl $name {
            pub fn from_hex(text: &str) -> Result<Self, RatingError> {
                DefinitionDigest::from_hex(text)
                    .map(Self)
                    .map_err(|_| RatingError::Identity)
            }
            #[must_use]
            pub const fn digest(self) -> DefinitionDigest {
                self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
digest_id!(RatingId);
digest_id!(ResultId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RatingClass {
    I,
    II,
    III,
}
impl RatingClass {
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        match self {
            Self::I => 1,
            Self::II => 2,
            Self::III => 3,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingRole {
    Combat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistryManifestName {
    #[serde(rename = "combat.v1")]
    CombatV1,
}

record!(RatingSubject, SubjectWire, {
    revision: RevisionIdentity, envelope: DefinitionDigest, schema: DefinitionDigest, definition: DefinitionDigest,
    expanded: DefinitionDigest, declared: RatingClass, role: RatingRole,
});
impl RatingSubject {
    /// Captures structural identities only. The worker must read and verify the
    /// exact saved revision, source, envelope and expansion before execution.
    pub fn new(
        revision: RevisionIdentity,
        envelope: DefinitionDigest,
        schema: DefinitionDigest,
        definition: DefinitionDigest,
        expanded: DefinitionDigest,
        declared: RatingClass,
    ) -> Self {
        Self {
            revision,
            envelope,
            schema,
            definition,
            expanded,
            declared,
            role: RatingRole::Combat,
        }
    }
    fn validate(&self) -> Result<(), RatingError> {
        Ok(())
    }
    #[must_use]
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    /// Exact canonical public schema identity; distinct from full definition
    /// and current engine/ABI compatibility. It does not grant disclosure.
    #[must_use]
    pub const fn schema(&self) -> DefinitionDigest {
        self.schema
    }
    #[must_use]
    pub const fn definition(&self) -> DefinitionDigest {
        self.definition
    }
    #[must_use]
    pub const fn envelope(&self) -> DefinitionDigest {
        self.envelope
    }
    #[must_use]
    pub const fn expanded(&self) -> DefinitionDigest {
        self.expanded
    }
    #[must_use]
    pub const fn declared(&self) -> RatingClass {
        self.declared
    }
    pub fn digest(&self) -> Result<DefinitionDigest, RatingError> {
        digest("tacsim.registry.subject.v1", self, 4096)
    }
}
record!(RatingCompatibility, CompatibilityWire, { manifest: RegistryManifestName, digest: DefinitionDigest });
impl RatingCompatibility {
    #[must_use]
    pub const fn combat_v1(digest: DefinitionDigest) -> Self {
        Self {
            manifest: RegistryManifestName::CombatV1,
            digest,
        }
    }
    fn validate(&self) -> Result<(), RatingError> {
        Ok(())
    }
    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialLayout {
    Duel,
    Muster,
    Hold,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeName {
    EmptyArmed,
    EmptyUnarmed,
    Hostile450Armed,
    Hostile450Unarmed,
    HurtTwoArmed,
    HurtTwoUnarmed,
    OrderLossArmed,
    OrderLossUnarmed,
    DensityArmed,
    DensityUnarmed,
    FordMobile,
    FordDisabled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelName {
    MemoryKeysAt,
    MemoryKeysOver,
    MemoryValueAt,
    MemoryValueOver,
    RulesSlotsAt,
    RulesSlotsOver,
    MeterFinalFee,
    MeterCallback,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaseKind {
    Trial {
        variant: u8,
        layout: TrialLayout,
        seed: u8,
        swapped: bool,
    },
    Probe {
        pair: u8,
        name: ProbeName,
    },
    ProfileCycle {},
    Kernel {
        name: KernelName,
    },
}
record!(CaseKey, CaseKeyWire, { class: RatingClass, case: CaseKind });
impl CaseKey {
    pub fn new(class: RatingClass, case: CaseKind) -> Result<Self, RatingError> {
        let value = Self { class, case };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        match self.case {
            CaseKind::Trial { variant, seed, .. } if variant >= 12 || !(1..=2).contains(&seed) => {
                Err(RatingError::Bound)
            }
            CaseKind::Probe { pair, .. } if pair >= 36 => Err(RatingError::Bound),
            _ => Ok(()),
        }
    }
    #[must_use]
    pub const fn class(&self) -> RatingClass {
        self.class
    }
    #[must_use]
    pub const fn kind(&self) -> CaseKind {
        self.case
    }
}
impl PartialOrd for CaseKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CaseKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.class, self.case).cmp(&(other.class, other.case))
    }
}

record!(Peak, PeakWire, { ops: u64, fees: u64, unit: u16, think_seq: u64 });
impl Peak {
    pub fn new(ops: u64, fees: u64, unit: u16, think_seq: u64) -> Result<Self, RatingError> {
        let value = Self {
            ops,
            fees,
            unit,
            think_seq,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.total()?;
        if self.unit >= 33 {
            Err(RatingError::Bound)
        } else {
            Ok(())
        }
    }
    pub fn total(&self) -> Result<u64, RatingError> {
        self.ops.checked_add(self.fees).ok_or(RatingError::Bound)
    }
    #[must_use]
    pub const fn ops(&self) -> u64 {
        self.ops
    }
    #[must_use]
    pub const fn fees(&self) -> u64 {
        self.fees
    }
}
record!(MeterSummary, MeterWire, {
    thinks: u64, max_ops: u64, max_fees: u64, #[serde(deserialize_with="required_option")] peak: Option<Peak>,
    ops_faults: u64, other_faults: u64, dropped: u64, unmeasured: u64,
});
impl MeterSummary {
    #[allow(clippy::too_many_arguments)] // Closed measurements; no caller-defined map or callback.
    pub fn new(
        thinks: u64,
        max_ops: u64,
        max_fees: u64,
        peak: Option<Peak>,
        ops_faults: u64,
        other_faults: u64,
        dropped: u64,
        unmeasured: u64,
    ) -> Result<Self, RatingError> {
        let value = Self {
            thinks,
            max_ops,
            max_fees,
            peak,
            ops_faults,
            other_faults,
            dropped,
            unmeasured,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if [
            self.ops_faults,
            self.other_faults,
            self.dropped,
            self.unmeasured,
        ]
        .iter()
        .any(|n| *n > self.thinks)
            || self
                .ops_faults
                .checked_add(self.other_faults)
                .ok_or(RatingError::Bound)?
                > self.thinks
        {
            return Err(RatingError::Invalid);
        }
        match (
            &self.peak,
            self.thinks
                .checked_sub(self.unmeasured)
                .ok_or(RatingError::Invalid)?,
        ) {
            (None, 0) if self.max_ops == 0 && self.max_fees == 0 => Ok(()),
            (Some(peak), n)
                if n > 0
                    && peak.ops <= self.max_ops
                    && peak.fees <= self.max_fees
                    && peak.total()? >= self.max_ops.max(self.max_fees) =>
            {
                Ok(())
            }
            _ => Err(RatingError::Invalid),
        }
    }
    #[must_use]
    pub const fn thinks(&self) -> u64 {
        self.thinks
    }
    #[must_use]
    pub const fn peak(&self) -> Option<&Peak> {
        self.peak.as_ref()
    }
    #[must_use]
    pub const fn max_ops(&self) -> u64 {
        self.max_ops
    }
    #[must_use]
    pub const fn max_fees(&self) -> u64 {
        self.max_fees
    }
    #[must_use]
    pub const fn ops_faults(&self) -> u64 {
        self.ops_faults
    }
    #[must_use]
    pub fn clean(&self) -> bool {
        self.ops_faults == 0 && self.other_faults == 0 && self.dropped == 0 && self.unmeasured == 0
    }
    #[must_use]
    pub fn only_budget_faults(&self) -> bool {
        self.ops_faults > 0 && self.other_faults == 0 && self.dropped == 0 && self.unmeasured == 0
    }
}

record!(CaseBinding, BindingWire, { source: DefinitionDigest, definition: DefinitionDigest, card: DefinitionDigest, program: DefinitionDigest });
impl CaseBinding {
    #[must_use]
    pub const fn new(
        source: DefinitionDigest,
        definition: DefinitionDigest,
        card: DefinitionDigest,
        program: DefinitionDigest,
    ) -> Self {
        Self {
            source,
            definition,
            card,
            program,
        }
    }
    fn validate(&self) -> Result<(), RatingError> {
        Ok(())
    }
    #[must_use]
    pub const fn source(&self) -> DefinitionDigest {
        self.source
    }
    #[must_use]
    pub const fn card(&self) -> DefinitionDigest {
        self.card
    }
    #[must_use]
    pub const fn program(&self) -> DefinitionDigest {
        self.program
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Armed,
    Unarmed,
    EngineBoundary,
}
record!(Coverage, CoverageWire, { applicability: Applicability, required_thinks: u32, evaluated_thinks: u32, witness: DefinitionDigest });
impl Coverage {
    pub fn new(
        applicability: Applicability,
        required_thinks: u32,
        evaluated_thinks: u32,
        witness: DefinitionDigest,
    ) -> Result<Self, RatingError> {
        let value = Self {
            applicability,
            required_thinks,
            evaluated_thinks,
            witness,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.required_thinks == 0
            || self.required_thinks > 1_000_000
            || self.evaluated_thinks > 1_000_000
        {
            return Err(RatingError::Bound);
        }
        Ok(())
    }
    #[must_use]
    pub const fn applicability(&self) -> Applicability {
        self.applicability
    }
    #[must_use]
    pub const fn required_thinks(&self) -> u32 {
        self.required_thinks
    }
    #[must_use]
    pub const fn evaluated_thinks(&self) -> u32 {
        self.evaluated_thinks
    }
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.evaluated_thinks >= self.required_thinks
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelOutcome {
    Passed,
    Ops,
    Limit,
    Memory,
    Type,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaseOutcome {
    Conformant {},
    BudgetExceeded {},
    Kernel {
        expected: KernelOutcome,
        actual: KernelOutcome,
    },
}
record!(TrialEnd, TrialEndWire, { at_ms: u64, state_hash: String, ledger: DefinitionDigest, outcome: TrialOutcome });
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialOutcome {
    Win,
    Loss,
    Draw,
    Timeout,
}
impl TrialEnd {
    pub fn new(
        at_ms: u64,
        state_hash: u64,
        ledger: DefinitionDigest,
        outcome: TrialOutcome,
    ) -> Result<Self, RatingError> {
        let value = Self {
            at_ms,
            state_hash: format!("{state_hash:016x}"),
            ledger,
            outcome,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.at_ms == 0
            || self.at_ms > 600_000 + 60_000
            || self.state_hash.len() != 16
            || !self
                .state_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
    /// How the trial ended (doc 17h §3 M8f: the rating score's win count).
    #[must_use]
    pub const fn outcome(&self) -> TrialOutcome {
        self.outcome
    }
}
record!(CaseResult, CaseResultWire, { key: CaseKey, binding: CaseBinding, meter: MeterSummary, coverage: Coverage, refusals: u32, outcome: CaseOutcome, #[serde(deserialize_with="required_option")] terminal: Option<TrialEnd> });
impl CaseResult {
    pub fn new(
        key: CaseKey,
        binding: CaseBinding,
        meter: MeterSummary,
        coverage: Coverage,
        refusals: u32,
        outcome: CaseOutcome,
        terminal: Option<TrialEnd>,
    ) -> Result<Self, RatingError> {
        let value = Self {
            key,
            binding,
            meter,
            coverage,
            refusals,
            outcome,
            terminal,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.key.validate()?;
        self.meter.validate()?;
        self.coverage.validate()?;
        let trial = matches!(self.key.case, CaseKind::Trial { .. });
        if (!trial && self.terminal.is_some())
            || (trial
                && matches!(self.outcome, CaseOutcome::Conformant {})
                && self.terminal.is_none())
            || u64::from(self.coverage.evaluated_thinks) > self.meter.thinks
        {
            return Err(RatingError::Invalid);
        }
        if let Some(end) = &self.terminal {
            end.validate()?;
        }
        match (&self.outcome, self.key.case) {
            (CaseOutcome::Kernel { expected, actual }, CaseKind::Kernel { .. })
                if expected == actual
                    && self.coverage.applicability == Applicability::EngineBoundary
                    && self.meter.thinks == 1
                    && match actual {
                        KernelOutcome::Passed => self.meter.clean(),
                        KernelOutcome::Ops => self.meter.only_budget_faults(),
                        KernelOutcome::Limit
                        | KernelOutcome::Memory
                        | KernelOutcome::Type
                        | KernelOutcome::Unknown => {
                            self.meter.other_faults == 1
                                && self.meter.ops_faults == 0
                                && self.meter.unmeasured == 0
                                && self.meter.dropped == 0
                        }
                    }
                    && self.coverage.complete() => {}
            (CaseOutcome::Conformant {}, case)
                if !matches!(case, CaseKind::Kernel { .. })
                    && self.meter.clean()
                    && self.meter.thinks > 0
                    && self.coverage.complete()
                    && self.coverage.applicability != Applicability::EngineBoundary => {}
            (CaseOutcome::BudgetExceeded {}, case)
                if !matches!(case, CaseKind::Kernel { .. })
                    && self.meter.only_budget_faults()
                    && self.coverage.applicability != Applicability::EngineBoundary => {}
            _ => return Err(RatingError::Invalid),
        }
        encoded(self, MAX_CASE_BYTES)?;
        Ok(())
    }
    #[must_use]
    pub const fn key(&self) -> &CaseKey {
        &self.key
    }
    #[must_use]
    pub const fn binding(&self) -> &CaseBinding {
        &self.binding
    }
    #[must_use]
    pub const fn meter(&self) -> &MeterSummary {
        &self.meter
    }
    #[must_use]
    pub const fn coverage(&self) -> &Coverage {
        &self.coverage
    }
    #[must_use]
    pub const fn outcome(&self) -> &CaseOutcome {
        &self.outcome
    }
    /// A trial's terminal (doc 17h §3 M8f: the rating score reads it).
    #[must_use]
    pub const fn terminal(&self) -> Option<&TrialEnd> {
        self.terminal.as_ref()
    }
}

record!(PageContent, PageContentWire, { format: u16, subject: DefinitionDigest, compatibility: DefinitionDigest, ordinal: u16, rows: Rows<CaseResult,MAX_PAGE_ROWS> });
impl PageContent {
    pub fn new(
        subject: DefinitionDigest,
        compatibility: DefinitionDigest,
        ordinal: u16,
        rows: Vec<CaseResult>,
    ) -> Result<Self, RatingError> {
        let value = Self {
            format: FORMAT,
            subject,
            compatibility,
            ordinal,
            rows: Rows::new(rows)?,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.format != FORMAT
            || usize::from(self.ordinal) >= MAX_PAGES
            || self.rows.0.is_empty()
            || self
                .rows
                .0
                .windows(2)
                .any(|pair| matches!(pair, [a,b] if a.key >= b.key))
        {
            return Err(RatingError::Invalid);
        }
        for row in &self.rows.0 {
            row.validate()?;
        }
        encoded(self, MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, RatingError> {
        digest(
            "tacsim.registry.page.v1",
            self,
            MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE,
        )
    }
    #[must_use]
    pub fn rows(&self) -> &[CaseResult] {
        &self.rows.0
    }
}
record!(RatingPageRef, PageRefWire, { ordinal:u16, digest:DefinitionDigest, rows:u16 });
impl RatingPageRef {
    fn validate(&self) -> Result<(), RatingError> {
        if usize::from(self.ordinal) >= MAX_PAGES
            || self.rows == 0
            || usize::from(self.rows) > MAX_PAGE_ROWS
        {
            Err(RatingError::Bound)
        } else {
            Ok(())
        }
    }
    #[must_use]
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }
    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}
record!(RatingResultRef, ResultRefWire, { id:ResultId, pages:Rows<RatingPageRef,MAX_PAGES>, cases:u16 });
impl RatingResultRef {
    pub fn from_pages(pages: &[PageContent]) -> Result<Self, RatingError> {
        let mut refs = Vec::with_capacity(pages.len().min(MAX_PAGES));
        let mut cases = 0u16;
        for page in pages {
            page.validate()?;
            let rows = u16::try_from(page.rows.0.len()).map_err(|_| RatingError::Bound)?;
            cases = cases.checked_add(rows).ok_or(RatingError::Bound)?;
            refs.push(RatingPageRef {
                ordinal: page.ordinal,
                digest: page.digest()?,
                rows,
            });
        }
        let pages = Rows::new(refs)?;
        let id = ResultId(digest(
            "tacsim.registry.result.v1",
            &pages,
            MAX_RECEIPT_BYTES,
        )?);
        let value = Self { id, pages, cases };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.pages.0.is_empty() || usize::from(self.cases) > MAX_CASES {
            return Err(RatingError::Bound);
        }
        let mut count = 0u16;
        for (i, page) in self.pages.0.iter().enumerate() {
            page.validate()?;
            if usize::from(page.ordinal) != i
                || (i + 1 < self.pages.0.len() && usize::from(page.rows) != MAX_PAGE_ROWS)
            {
                return Err(RatingError::Invalid);
            }
            count = count.checked_add(page.rows).ok_or(RatingError::Bound)?;
        }
        if count != self.cases
            || self.id.0 != digest("tacsim.registry.result.v1", &self.pages, MAX_RECEIPT_BYTES)?
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    #[must_use]
    pub const fn id(&self) -> ResultId {
        self.id
    }
    #[must_use]
    pub fn pages(&self) -> &[RatingPageRef] {
        &self.pages.0
    }
    #[must_use]
    pub const fn cases(&self) -> u16 {
        self.cases
    }
}
record!(RatingPage, PageWire, { job:CommitToken, rating:RatingId, result:ResultId, content:PageContent });
impl RatingPage {
    /// No circular hash: result derives from content-only page hashes. The
    /// actual parent receipt validates the ordered complete page set. The
    /// admitted job is the durable lookup key; the measured content identity
    /// remains independent of the execution attempt and publication clock.
    pub fn new(
        job: CommitToken,
        rating: RatingId,
        result: ResultId,
        content: PageContent,
    ) -> Result<Self, RatingError> {
        let value = Self {
            job,
            rating,
            result,
            content,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.content.validate()?;
        encoded(self, MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    #[must_use]
    pub fn key(&self) -> RatingPageKey {
        RatingPageKey {
            job: self.job,
            ordinal: self.content.ordinal,
        }
    }
    #[must_use]
    pub const fn content(&self) -> &PageContent {
        &self.content
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RatingPageKey {
    job: CommitToken,
    ordinal: u16,
}
impl RatingPageKey {
    pub fn new(job: CommitToken, ordinal: u16) -> Result<Self, RatingError> {
        if usize::from(ordinal) >= MAX_PAGES {
            return Err(RatingError::Bound);
        }
        Ok(Self { job, ordinal })
    }
    #[must_use]
    pub fn encoded(&self) -> String {
        format!("{}/{:03}", self.job, self.ordinal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasKind {
    Profile,
    Preset,
}
record!(VariantAlias,AliasWire,{kind:AliasKind,name:String,variant:u8});
impl VariantAlias {
    pub fn new(kind: AliasKind, name: String, variant: u8) -> Result<Self, RatingError> {
        let value = Self {
            kind,
            name,
            variant,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.name.is_empty()
            || self.name.len() > 64
            || self.name.trim() != self.name
            || self.name.chars().any(char::is_control)
            || self.variant >= 12
        {
            Err(RatingError::Bound)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassOutcome {
    BudgetExceeded,
    Passing,
}
record!(ClassSummary,ClassWire,{class:RatingClass,outcome:ClassOutcome,cases:u16,meter:MeterSummary,peak_case:CaseKey,cases_digest:DefinitionDigest});
impl ClassSummary {
    pub fn from_cases(class: RatingClass, rows: &[&CaseResult]) -> Result<Self, RatingError> {
        let cases = u16::try_from(rows.len()).map_err(|_| RatingError::Bound)?;
        let mut thinks = 0u64;
        let mut max_ops = 0;
        let mut max_fees = 0;
        let mut peak: Option<Peak> = None;
        let mut peak_case = None;
        let mut ops_faults = 0u64;
        let mut outcome = ClassOutcome::Passing;
        for (i, row) in rows.iter().enumerate() {
            row.validate()?;
            if row.key.class != class {
                return Err(RatingError::Invalid);
            }
            if matches!(row.key.case, CaseKind::Kernel { .. }) {
                continue;
            }
            if matches!(row.outcome, CaseOutcome::BudgetExceeded {}) {
                if i + 1 != rows.len() {
                    return Err(RatingError::Invalid);
                }
                outcome = ClassOutcome::BudgetExceeded;
            }
            thinks = thinks
                .checked_add(row.meter.thinks)
                .ok_or(RatingError::Bound)?;
            ops_faults = ops_faults
                .checked_add(row.meter.ops_faults)
                .ok_or(RatingError::Bound)?;
            max_ops = max_ops.max(row.meter.max_ops);
            max_fees = max_fees.max(row.meter.max_fees);
            if let Some(candidate) = &row.meter.peak {
                let current = peak.as_ref().map(Peak::total).transpose()?;
                if current.is_none_or(|total| candidate.total().is_ok_and(|next| next > total)) {
                    peak = Some(candidate.clone());
                    peak_case = Some(row.key.clone());
                }
            }
        }
        let meter = MeterSummary::new(thinks, max_ops, max_fees, peak, ops_faults, 0, 0, 0)?;
        let value = Self {
            class,
            outcome,
            cases,
            meter,
            peak_case: peak_case.ok_or(RatingError::Incomplete)?,
            cases_digest: digest("tacsim.registry.class-cases.v1", &rows, MAX_RESULT_BYTES)?,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.meter.validate()?;
        if self.cases == 0
            || usize::from(self.cases) > MAX_CASES
            || self.peak_case.class != self.class
            || match self.outcome {
                ClassOutcome::Passing => !self.meter.clean(),
                ClassOutcome::BudgetExceeded => !self.meter.only_budget_faults(),
            }
        {
            Err(RatingError::Invalid)
        } else {
            Ok(())
        }
    }
}
record!(CertificateContent,CertificateWire,{format:u16,subject:RatingSubject,compatibility:RatingCompatibility,plan:DefinitionDigest,aliases:Rows<VariantAlias,12>,classes:Rows<ClassSummary,3>,minimum:RatingClass,result:RatingResultRef});
impl CertificateContent {
    pub fn new(
        subject: RatingSubject,
        compatibility: RatingCompatibility,
        plan: DefinitionDigest,
        aliases: Vec<VariantAlias>,
        classes: Vec<ClassSummary>,
        minimum: RatingClass,
        result: RatingResultRef,
    ) -> Result<Self, RatingError> {
        let value = Self {
            format: FORMAT,
            subject,
            compatibility,
            plan,
            aliases: Rows::new(aliases)?,
            classes: Rows::new(classes)?,
            minimum,
            result,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.format != FORMAT || self.aliases.0.is_empty() || self.classes.0.is_empty() {
            return Err(RatingError::Invalid);
        }
        if self
            .aliases
            .0
            .windows(2)
            .any(|pair| matches!(pair,[a,b]if(a.kind,&a.name)>=(b.kind,&b.name)))
        {
            return Err(RatingError::Invalid);
        }
        let mut cases = 0u16;
        for (i, class) in self.classes.0.iter().enumerate() {
            class.validate()?;
            if usize::from(class.class.ordinal())
                != usize::from(self.subject.declared.ordinal()) + i
                || ((i + 1 == self.classes.0.len()) != (class.outcome == ClassOutcome::Passing))
            {
                return Err(RatingError::Invalid);
            }
            cases = cases.checked_add(class.cases).ok_or(RatingError::Bound)?;
        }
        if self.classes.0.last().map(|row| row.class) != Some(self.minimum)
            || cases != self.result.cases
        {
            return Err(RatingError::Invalid);
        }
        self.result.validate()?;
        encoded(self, MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    pub fn id(&self) -> Result<RatingId, RatingError> {
        Ok(RatingId(digest(
            "tacsim.registry.rating.v1",
            self,
            MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE,
        )?))
    }
}
record!(RatingPublication,PublicationWire,{job:CommitToken,issued:ClockStamp});
impl RatingPublication {
    pub fn new(job: CommitToken, issued: ClockStamp) -> Result<Self, RatingError> {
        let value = Self { job, issued };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        WorldIcSeconds::from_stamp(&self.issued).map_err(|_| RatingError::Invalid)?;
        Ok(())
    }
    #[must_use]
    pub const fn job(&self) -> CommitToken {
        self.job
    }
}
record!(RatingReceipt,ReceiptWire,{format:u16,id:RatingId,content:CertificateContent,publication:RatingPublication});
impl RatingReceipt {
    /// Structural assembly only. This constructor never installs a typed row.
    pub fn new(
        content: CertificateContent,
        publication: RatingPublication,
    ) -> Result<Self, RatingError> {
        let value = Self {
            format: FORMAT,
            id: content.id()?,
            content,
            publication,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        self.content.validate()?;
        self.publication.validate()?;
        if self.format != FORMAT || self.id != self.content.id()? {
            return Err(RatingError::Identity);
        }
        encoded(self, MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    #[must_use]
    pub const fn id(&self) -> RatingId {
        self.id
    }
    #[must_use]
    pub const fn subject(&self) -> &RatingSubject {
        &self.content.subject
    }
    #[must_use]
    pub const fn compatibility(&self) -> &RatingCompatibility {
        &self.content.compatibility
    }
    #[must_use]
    pub const fn plan(&self) -> DefinitionDigest {
        self.content.plan
    }
    #[must_use]
    pub const fn result(&self) -> &RatingResultRef {
        &self.content.result
    }
    #[must_use]
    pub const fn minimum(&self) -> RatingClass {
        self.content.minimum
    }
    #[must_use]
    pub const fn publication(&self) -> &RatingPublication {
        &self.publication
    }
    pub fn reference(&self) -> Result<RatingRef, RatingError> {
        self.validate()?;
        Ok(RatingRef {
            job: self.publication.job,
            id: self.id,
            schema: FORMAT,
            receipt_digest: digest(
                "tacsim.registry.receipt.v1",
                self,
                MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE,
            )?,
        })
    }
    pub fn verify_pages(&self, pages: &[RatingPage]) -> Result<(), RatingError> {
        self.validate()?;
        if pages.len() != self.result().pages.0.len() {
            return Err(RatingError::Incomplete);
        }
        let mut prior = None;
        let mut bytes = 0usize;
        let mut rows = Vec::with_capacity(usize::from(self.result().cases));
        for (page, wanted) in pages.iter().zip(&self.result().pages.0) {
            page.validate()?;
            bytes = bytes
                .checked_add(
                    encoded(page, MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE)?.len()
                        + TYPED_ENVELOPE_RESERVE,
                )
                .ok_or(RatingError::Bound)?;
            if bytes > MAX_RESULT_BYTES
                || page.job != self.publication.job
                || page.rating != self.id
                || page.result != self.result().id
                || page.content.ordinal != wanted.ordinal
                || page.content.digest()? != wanted.digest
                || page.content.subject != self.subject().digest()?
                || page.content.compatibility != self.compatibility().digest
                || page.content.rows.0.len() != usize::from(wanted.rows)
            {
                return Err(RatingError::Identity);
            }
            for row in &page.content.rows.0 {
                if prior.is_some_and(|p| p >= &row.key) {
                    return Err(RatingError::Invalid);
                }
                prior = Some(&row.key);
                if !matches!(row.key.case, CaseKind::Kernel { .. })
                    && row.binding.source != self.subject().revision.source()
                {
                    return Err(RatingError::Identity);
                }
                rows.push(row);
            }
        }
        for summary in &self.content.classes.0 {
            let class_rows = rows
                .iter()
                .copied()
                .filter(|row| row.key.class == summary.class)
                .collect::<Vec<_>>();
            if ClassSummary::from_cases(summary.class, &class_rows)? != *summary {
                return Err(RatingError::Identity);
            }
        }
        if rows.iter().any(|row| {
            !self
                .content
                .classes
                .0
                .iter()
                .any(|summary| summary.class == row.key.class)
        }) {
            return Err(RatingError::Invalid);
        }
        Ok(())
    }
}
// Every reference binds both the exact durable attempt and measured content.
// Identical measurements from another admission may share `id`, never `job`.
record!(RatingRef,RatingRefWire,{job:CommitToken,id:RatingId,schema:u16,receipt_digest:DefinitionDigest});
impl Copy for RatingRef {}
impl RatingRef {
    /// Exact admitted attempt. This is a structural locator, not producer authority.
    #[must_use]
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    fn validate(&self) -> Result<(), RatingError> {
        if self.schema != FORMAT {
            Err(RatingError::Invalid)
        } else {
            Ok(())
        }
    }
    #[must_use]
    pub const fn id(&self) -> RatingId {
        self.id
    }
    #[must_use]
    pub const fn schema(&self) -> u16 {
        self.schema
    }
    #[must_use]
    pub const fn receipt_digest(&self) -> DefinitionDigest {
        self.receipt_digest
    }
}

pub fn decode_receipt(key: &str, bytes: &[u8]) -> Result<RatingReceipt, RatingError> {
    if bytes.len() > MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE {
        return Err(RatingError::Bound);
    }
    let row: RatingReceipt = serde_json::from_slice(bytes).map_err(|_| RatingError::Invalid)?;
    if key != row.publication.job.to_string() {
        return Err(RatingError::Identity);
    }
    Ok(row)
}
pub fn decode_page(key: &str, bytes: &[u8]) -> Result<RatingPage, RatingError> {
    if bytes.len() > MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE {
        return Err(RatingError::Bound);
    }
    let row: RatingPage = serde_json::from_slice(bytes).map_err(|_| RatingError::Invalid)?;
    if key != row.key().encoded() {
        return Err(RatingError::Identity);
    }
    Ok(row)
}
fn encoded<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, RatingError> {
    let bytes = serde_json::to_vec(value).map_err(|_| RatingError::Invalid)?;
    if bytes.len() > max {
        return Err(RatingError::Bound);
    }
    Ok(bytes)
}
fn digest<T: Serialize>(
    domain: &str,
    value: &T,
    max: usize,
) -> Result<DefinitionDigest, RatingError> {
    let value = serde_json::to_value(value).map_err(|_| RatingError::Invalid)?;
    DefinitionDigest::for_canonical_value_with_limit(domain, &value, max)
        .map_err(|_| RatingError::Bound)
}

fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}

/// Durable closed job rows keep exact subjects rooted through interruption.
/// These states do not replace the bulk owner's actual quota/lease journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RatingJobState {
    Queued {},
    Running {
        lease_sequence: u64,
    },
    Completed {
        receipt: RatingRef,
    },
    Nonconforming {
        receipt: nonconformance::NonconformanceRef,
    },
    Failed {
        reason: RatingFailure,
    },
    Interrupted {},
    Cancelled {},
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingFailure {
    /// Actual worker failure without a trustworthy narrower classification.
    Execution,
    Budget,
    Conformance,
    Fixture,
    Source,
    Compatibility,
    Capacity,
}
record!(RatingJob,JobWire,{format:u16,job:CommitToken,subject:RatingSubject,compatibility:RatingCompatibility,plan:DefinitionDigest,state:RatingJobState});
impl RatingJob {
    pub fn new(
        job: CommitToken,
        subject: RatingSubject,
        compatibility: RatingCompatibility,
        plan: DefinitionDigest,
    ) -> Self {
        Self {
            format: FORMAT,
            job,
            subject,
            compatibility,
            plan,
            state: RatingJobState::Queued {},
        }
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        if self.format != FORMAT
            || matches!(self.state, RatingJobState::Running { lease_sequence: 0 })
        {
            return Err(RatingError::Invalid);
        }
        encoded(self, 4096 - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    #[must_use]
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    #[must_use]
    pub const fn subject(&self) -> &RatingSubject {
        &self.subject
    }
    #[must_use]
    pub const fn state(&self) -> &RatingJobState {
        &self.state
    }
    #[must_use]
    pub const fn compatibility(&self) -> &RatingCompatibility {
        &self.compatibility
    }
    #[must_use]
    pub const fn plan(&self) -> DefinitionDigest {
        self.plan
    }
    pub(crate) fn matches_receipt(&self, receipt: &RatingReceipt) -> Result<bool, RatingError> {
        Ok(receipt.publication().job() == self.job
            && receipt.subject() == &self.subject
            && receipt.compatibility() == &self.compatibility
            && receipt.plan() == self.plan
            && matches!(self.state(), RatingJobState::Completed { receipt: actual } if *actual == receipt.reference()?))
    }
    pub(crate) fn matches_nonconformance(
        &self,
        receipt: &nonconformance::RatingNonconformance,
    ) -> Result<bool, RatingError> {
        Ok(receipt.publication().job() == self.job
            && receipt.subject() == &self.subject
            && receipt.compatibility() == &self.compatibility
            && receipt.plan() == self.plan
            && matches!(self.state(), RatingJobState::Nonconforming { receipt: actual } if *actual == receipt.reference()?))
    }
    /// Closed owner mutation; the composed service still checks the real paid
    /// flight and immutable source before preparing this post-image.
    pub(crate) fn transition(&self, state: RatingJobState) -> Result<Self, RatingError> {
        let allowed = match (&self.state, &state) {
            (
                RatingJobState::Queued {},
                RatingJobState::Cancelled {}
                | RatingJobState::Interrupted {}
                | RatingJobState::Failed { .. },
            ) => true,
            (RatingJobState::Queued {}, RatingJobState::Running { lease_sequence: 1 }) => true,
            (
                RatingJobState::Running {
                    lease_sequence: old,
                },
                RatingJobState::Running {
                    lease_sequence: new,
                },
            ) => old.checked_add(1) == Some(*new),
            (
                RatingJobState::Running { .. },
                RatingJobState::Completed { .. }
                | RatingJobState::Nonconforming { .. }
                | RatingJobState::Failed { .. }
                | RatingJobState::Interrupted {}
                | RatingJobState::Cancelled {},
            ) => true,
            _ => false,
        };
        if !allowed {
            return Err(RatingError::Invalid);
        }
        let mut next = self.clone();
        next.state = state;
        next.validate()?;
        Ok(next)
    }
}
record!(RatingCompletion,CompletionWire,{format:u16,job:CommitToken,subject:DefinitionDigest,compatibility:DefinitionDigest,result:ResultId,receipt:RatingRef,execution:DefinitionDigest});
impl RatingCompletion {
    pub(crate) fn for_execution(
        row: &RatingReceipt,
        execution: &execution::RatingExecution,
    ) -> Result<Self, RatingError> {
        if execution.job() != row.publication.job
            || execution.observation().phase != execution::ExecutionPhase::Complete
        {
            return Err(RatingError::Identity);
        }
        Ok(Self {
            format: FORMAT,
            job: row.publication.job,
            subject: row.subject().digest()?,
            compatibility: row.compatibility().digest,
            result: row.result().id,
            receipt: row.reference()?,
            execution: execution.digest()?,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        if self.format != FORMAT {
            return Err(RatingError::Invalid);
        }
        encoded(self, 4096 - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    pub(crate) fn matches(
        &self,
        row: &RatingReceipt,
        execution: &execution::RatingExecution,
    ) -> Result<bool, RatingError> {
        Ok(*self == Self::for_execution(row, execution)?)
    }
    #[must_use]
    pub const fn job(&self) -> CommitToken {
        self.job
    }
    #[must_use]
    pub const fn receipt(&self) -> &RatingRef {
        &self.receipt
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RatingStanding {
    Active {},
    Recalled { operation: CommitToken },
}
record!(RatingStatus,StatusWire,{format:u16,rating:RatingRef,standing:RatingStanding});
impl RatingStatus {
    pub(crate) fn active(row: &RatingReceipt) -> Result<Self, RatingError> {
        Ok(Self {
            format: FORMAT,
            rating: row.reference()?,
            standing: RatingStanding::Active {},
        })
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        if self.format != FORMAT {
            return Err(RatingError::Invalid);
        }
        encoded(self, 4096 - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    #[must_use]
    pub const fn rating(&self) -> &RatingRef {
        &self.rating
    }
    #[must_use]
    pub const fn standing(&self) -> &RatingStanding {
        &self.standing
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;
