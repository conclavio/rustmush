//! Bounded observation of the real submitted-source runner. This wrapper never
//! fabricates an answer, substitutes a callback, or changes fault/landing logic.
//! Its state is private preparation/worker state, absent from live arenas.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use mush_core::{DefinitionDigest, ObjId};
use mush_roll::commerce::rating::{MeterSummary, Peak, RatingError};
use serde::Serialize;

use crate::arena::{
    core::CoreProfile,
    mind::{MindRunner, ThinkJob, ThinkResult},
    profile_rules::BoundProgram,
    runtime_card::{ProfileKey, RuntimeCard},
};
use crate::minds::RhaiRunner;

use super::identity;

mod physical;
pub(super) use physical::PhysicalSummary;

/// Derived only from hydrated runtime admission. Comparing both the full card
/// bytes and concrete program prevents a matching nominal profile index from
/// hiding different authored terms or a different hull expansion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Binding {
    key: ProfileKey,
    card: Vec<u8>,
    program: BoundProgram,
}
impl Binding {
    pub(super) fn capture(card: &RuntimeCard, program: &BoundProgram) -> Result<Self, RatingError> {
        let bytes = card.canonical_bytes().ok_or(RatingError::Incomplete)?;
        if bytes.len() > 64 * 1024 || program.rows.len() > 6 {
            return Err(RatingError::Bound);
        }
        Ok(Self {
            key: card.key(),
            card: bytes.to_vec(),
            program: program.clone(),
        })
    }
    fn matches(&self, job: &ThinkJob<'_>) -> bool {
        self.matches_parts(job.card, job.program)
            && job.profile_token.key == self.key
            && job.profile_token.unit == job.unit
    }

    fn matches_parts(&self, card: &RuntimeCard, program: &BoundProgram) -> bool {
        card.key() == self.key
            && card.canonical_bytes() == Some(self.card.as_slice())
            && program == &self.program
    }
}

#[derive(Debug)]
struct UnitMeter {
    ordinal: u16,
    bindings: Arc<[Binding]>,
    last_sequence: Option<u64>,
    seen: Vec<u64>,
}
#[derive(Debug)]
struct State {
    units: BTreeMap<ObjId, UnitMeter>,
    thinks: u64,
    maximum_thinks: u64,
    max_ops: u64,
    max_fees: u64,
    peak: Option<Peak>,
    ops_faults: u64,
    other_faults: u64,
    dropped: u64,
    unmeasured: u64,
    witness: DefinitionDigest,
    invalid: Option<RatingError>,
    physical: Option<physical::Physical>,
}

/// The contained runner is compiled from the exact verified subject source.
/// There is intentionally no generic MindRunner constructor in this module.
#[derive(Debug)]
pub(super) struct MeteredRunner {
    runner: Arc<RhaiRunner>,
    profile: CoreProfile,
    state: Mutex<State>,
}

#[derive(Debug)]
pub(super) struct Measurement {
    pub(super) summary: MeterSummary,
    pub(super) witness: DefinitionDigest,
    /// Per-unit/per-profile counts in immutable case roster/profile order.
    pub(super) seen: Vec<Vec<u64>>,
    pub(super) physical: Option<PhysicalSummary>,
}

impl MeteredRunner {
    pub(super) fn new(
        runner: Arc<RhaiRunner>,
        profile: CoreProfile,
        units: BTreeMap<ObjId, Vec<Binding>>,
        maximum_thinks: u64,
    ) -> Result<Self, RatingError> {
        if units.is_empty() || units.len() > 33 || maximum_thinks == 0 || maximum_thinks > 1_000_000
        {
            return Err(RatingError::Bound);
        }
        let mut measured = BTreeMap::new();
        for (ordinal, (id, bindings)) in units.into_iter().enumerate() {
            if bindings.is_empty()
                || bindings.len() > 4
                || bindings.iter().enumerate().any(|(i, b)| {
                    bindings
                        .iter()
                        .take(i)
                        .any(|previous| previous.key == b.key)
                })
            {
                return Err(RatingError::Invalid);
            }
            let seen = vec![0; bindings.len()];
            measured.insert(
                id,
                UnitMeter {
                    ordinal: u16::try_from(ordinal).map_err(|_| RatingError::Bound)?,
                    bindings: bindings.into(),
                    last_sequence: None,
                    seen,
                },
            );
        }
        Ok(Self {
            runner,
            profile,
            state: Mutex::new(State {
                units: measured,
                thinks: 0,
                maximum_thinks,
                max_ops: 0,
                max_fees: 0,
                peak: None,
                ops_faults: 0,
                other_faults: 0,
                dropped: 0,
                unmeasured: 0,
                witness: identity("tacsim.registry.thinks.begin.v1", &profile, 4096)?,
                invalid: None,
                physical: None,
            }),
        })
    }

    /// Call only at a settled worker boundary after all synchronous invocations
    /// have returned. A poisoned/overflowed observation is never zero cost.
    pub(super) fn measurement(&self) -> Result<Measurement, RatingError> {
        let state = self.state.lock().map_err(|_| RatingError::Incomplete)?;
        if let Some(error) = state.invalid {
            return Err(error);
        }
        let physical = state
            .physical
            .as_ref()
            .map(physical::Physical::summary)
            .transpose()?;
        if physical.as_ref().is_some_and(|physical| {
            state
                .thinks
                .checked_add(physical.degraded_thinks)
                .and_then(|count| count.checked_add(physical.rom_thinks))
                != Some(physical.total_thinks)
        }) {
            return Err(RatingError::Incomplete);
        }
        Ok(Measurement {
            summary: MeterSummary::new(
                state.thinks,
                state.max_ops,
                state.max_fees,
                state.peak.clone(),
                state.ops_faults,
                state.other_faults,
                state.dropped,
                state.unmeasured,
            )?,
            witness: state.witness,
            seen: state.units.values().map(|unit| unit.seen.clone()).collect(),
            physical,
        })
    }

    /// Whole trials alone admit authenticated physical interruption. Direct
    /// profile/kernel cases retain their original strict runner contract.
    pub(super) fn enable_physical(&self, arena: &crate::arena::Arena) -> Result<(), RatingError> {
        let mut state = self.state.lock().map_err(|_| RatingError::Incomplete)?;
        if state.thinks != 0 || state.physical.is_some() || state.invalid.is_some() {
            return Err(RatingError::Invalid);
        }
        state.physical = Some(physical::Physical::capture(
            arena,
            &state.units,
            self.profile,
        )?);
        Ok(())
    }

    pub(super) fn reconcile(&self, arena: &crate::arena::Arena) -> Result<(), RatingError> {
        let mut state = self.state.lock().map_err(|_| RatingError::Incomplete)?;
        if let Some(error) = state.invalid {
            return Err(error);
        }
        let result = state
            .physical
            .as_mut()
            .ok_or(RatingError::Incomplete)?
            .reconcile(arena);
        if let Err(error) = result {
            state.invalid = Some(error);
        }
        result
    }

    pub(super) fn require_complete(&self) -> Result<(), RatingError> {
        let state = self.state.lock().map_err(|_| RatingError::Incomplete)?;
        if let Some(error) = state.invalid {
            return Err(error);
        }
        state
            .physical
            .as_ref()
            .ok_or(RatingError::Incomplete)?
            .require_complete()
    }

    pub(super) fn physically_interrupted(&self, unit: ObjId) -> Result<bool, RatingError> {
        let state = self.state.lock().map_err(|_| RatingError::Incomplete)?;
        if let Some(error) = state.invalid {
            return Err(error);
        }
        state
            .physical
            .as_ref()
            .ok_or(RatingError::Incomplete)?
            .interrupted(unit)
    }
}

impl MindRunner for MeteredRunner {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.runner.think(job);
        // Return the actual answer unchanged even after an evidence failure.
        // The case owner rejects the measurement; ordinary settlement still
        // sees exactly what the submitted source produced.
        if let Ok(mut state) = self.state.lock()
            && state.invalid.is_none()
        {
            let recorded = if let Some(mut physical) = state.physical.take() {
                let recorded = physical.record(&mut state, &self.profile, job, &result);
                state.physical = Some(physical);
                recorded
            } else {
                record(&mut state, &self.profile, job, &result)
            };
            if let Err(error) = recorded {
                state.invalid = Some(error);
            }
        }
        result
    }
}

fn record(
    state: &mut State,
    expected: &CoreProfile,
    job: &ThinkJob<'_>,
    result: &ThinkResult,
) -> Result<(), RatingError> {
    if job.profile != expected
        || state.thinks >= state.maximum_thinks
        || job.unit.0 != job.view.unit
    {
        return Err(RatingError::Identity);
    }
    let unit = state
        .units
        .get_mut(&job.unit)
        .ok_or(RatingError::Identity)?;
    if unit
        .last_sequence
        .is_some_and(|sequence| job.think_seq <= sequence)
    {
        return Err(RatingError::Identity);
    }
    let index = unit
        .bindings
        .iter()
        .position(|binding| binding.matches(job))
        .ok_or(RatingError::Identity)?;
    let count = unit.seen.get_mut(index).ok_or(RatingError::Incomplete)?;
    *count = count.checked_add(1).ok_or(RatingError::Bound)?;
    unit.last_sequence = Some(job.think_seq);
    state.thinks = state.thinks.checked_add(1).ok_or(RatingError::Bound)?;
    if job.dropped {
        state.dropped = state.dropped.checked_add(1).ok_or(RatingError::Bound)?;
    }
    let counts = result.diagnostics.as_ref().and_then(|d| d.counts());
    if let Some((ops, fees)) = counts {
        let peak = Peak::new(ops, fees, unit.ordinal, job.think_seq)?;
        let total = peak.total()?;
        if result.fault.is_none() && total > expected.ops {
            return Err(RatingError::Invalid);
        }
        state.max_ops = state.max_ops.max(ops);
        state.max_fees = state.max_fees.max(fees);
        if state
            .peak
            .as_ref()
            .map(Peak::total)
            .transpose()?
            .is_none_or(|old| total > old)
        {
            state.peak = Some(peak);
        }
    } else {
        state.unmeasured = state.unmeasured.checked_add(1).ok_or(RatingError::Bound)?;
    }
    if let Some(fault) = &result.fault {
        let counter = if fault.cause == "ops" {
            &mut state.ops_faults
        } else {
            &mut state.other_faults
        };
        *counter = counter.checked_add(1).ok_or(RatingError::Bound)?;
    }
    // Bounded rolling identity, not a retained transcript or copied private
    // memory. Include both clocks; the public picture may be off the timer grid.
    state.witness = identity(
        "tacsim.registry.think-link.v1",
        &(
            state.witness,
            unit.ordinal,
            job.think_seq,
            job.t_ms,
            job.view.t_ms,
            job.lands_ms,
            job.card.key(),
            job.program.digest,
            job.profile_token,
            counts,
            result.fault.as_ref().map(|f| f.cause.as_str()),
            job.dropped,
        ),
        4096,
    )?;
    Ok(())
}
