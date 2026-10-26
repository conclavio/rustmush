//! Trial-only evidence, authenticated at the ordinary settled arena boundary.
//! No caller supplies a damage permit. Source/ROM answers are returned unchanged.
use super::*;
use crate::arena::{
    Arena,
    author_trace::{Cancelled, Fault, Outcome, RomDrop, THINKS, ThinkDiagnostics, ThinkTrace},
    conditions::ConditionTable,
    core::{ClassTable, CoreClass, CoreHardware},
    mind::{DropCause, SortieId},
    profile_runtime::ProfileToken,
};

const MAX_PENDING_PER_UNIT: usize = THINKS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Kind {
    Tested,
    Degraded,
    Rom,
}

#[derive(Debug)]
struct Invocation {
    class: CoreClass,
    kind: Kind,
    t_ms: i64,
    lands_ms: i64,
    counts: Option<(u64, u64)>,
    fault: Option<Fault>,
    captured_rom: bool,
    checked: bool,
    token: ProfileToken,
}

#[derive(Debug)]
struct Unit {
    core: Option<CoreHardware>,
    sealed_core: CoreClass,
    sortie: SortieId,
    rom: Binding,
    bindings: Arc<[Binding]>,
    next_call: u64,
    next_trace: u64,
    pending: BTreeMap<u64, Invocation>,
    ops_faults: u64,
    interrupted: bool,
    token: ProfileToken,
}

/// These counters never stand in for tested-source coverage. Their complete
/// bounded identity enters the trial's coverage witness and the manifest binds
/// this observer implementation. They are not a new receipt wire format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::minds::registry) struct PhysicalSummary {
    pub(in crate::minds::registry) total_thinks: u64,
    pub(in crate::minds::registry) degraded_thinks: u64,
    pub(in crate::minds::registry) degraded_ops_faults: u64,
    pub(in crate::minds::registry) degraded_max_ops: u64,
    pub(in crate::minds::registry) degraded_max_fees: u64,
    pub(in crate::minds::registry) rom_thinks: u64,
    pub(in crate::minds::registry) refusals: u32,
    pub(in crate::minds::registry) cancelled_uncalled: u64,
    pub(in crate::minds::registry) cancelled_pending: u64,
    witness: DefinitionDigest,
}

#[derive(Debug)]
pub(super) struct Physical {
    units: BTreeMap<ObjId, Unit>,
    classes: ClassTable,
    tested: CoreClass,
    summary: PhysicalSummary,
    dirty: bool,
    terminal: bool,
}

impl Physical {
    pub(super) fn capture(
        arena: &Arena,
        meters: &BTreeMap<ObjId, UnitMeter>,
        profile: CoreProfile,
    ) -> Result<Self, RatingError> {
        if meters.is_empty()
            || meters.len() > 2
            || arena.classes.row(profile.class) != profile
            || profile.class == CoreClass::Zero
        {
            #[cfg(test)]
            eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
            return Err(RatingError::Identity);
        }
        let mut units = BTreeMap::new();
        for (id, meter) in meters {
            let own = arena.unit(*id).ok_or(RatingError::Incomplete)?;
            let mind = arena.minds.get(id).ok_or(RatingError::Incomplete)?;
            let core = arena.core_of(*id).ok_or(RatingError::Identity)?;
            let runtime = arena.profiles.get(id).ok_or(RatingError::Incomplete)?;
            if own.seal.class != profile.class
                || core.class < profile.class
                || mind.think_seq != 0
                || mind.pending.is_some()
                || mind.dropped.is_some()
                || mind.strikes != 0
                || runtime.dropped
                || runtime.diagnostic_override
                || runtime.unit != *id
                || own.seal.rom != runtime.rom.key()
                || !meter
                    .bindings
                    .iter()
                    .any(|binding| binding.matches_parts(&runtime.card, &runtime.program))
                || !arena.traces.recent(*id, Some(THINKS)).is_empty()
            {
                #[cfg(test)]
                eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                return Err(RatingError::Identity);
            }
            // Derive the exact ROM binding through the actual immutable runtime
            // preparation, on an uninstalled clone, without changing the arena.
            let mut rom = runtime.clone();
            let prepared = rom
                .prepare_rom(
                    *id,
                    &arena.classes.row(CoreClass::Zero),
                    &runtime.weapons,
                    &runtime.channels,
                    arena.odds_floor_permille,
                )
                .map_err(|_| RatingError::Identity)?;
            rom.commit(&mut ConditionTable::default(), prepared)
                .map_err(|_| RatingError::Identity)?;
            let rom = Binding::capture(&rom.card, &rom.program)?;
            if serde_json::to_vec(&rom)
                .map_err(|_| RatingError::Invalid)?
                .len()
                > 128 * 1024
            {
                return Err(RatingError::Bound);
            }
            units.insert(
                *id,
                Unit {
                    core: Some(core),
                    sealed_core: own.seal.core_class,
                    sortie: own.seal.sortie,
                    rom,
                    bindings: Arc::clone(&meter.bindings),
                    next_call: 0,
                    next_trace: 0,
                    pending: BTreeMap::new(),
                    ops_faults: 0,
                    interrupted: false,
                    token: runtime.token(),
                },
            );
        }
        Ok(Self {
            units,
            classes: arena.classes,
            tested: profile.class,
            dirty: false,
            terminal: false,
            summary: PhysicalSummary {
                total_thinks: 0,
                degraded_thinks: 0,
                degraded_ops_faults: 0,
                degraded_max_ops: 0,
                degraded_max_fees: 0,
                rom_thinks: 0,
                refusals: 0,
                cancelled_uncalled: 0,
                cancelled_pending: 0,
                witness: identity(
                    "tacsim.registry.physical.begin.v1",
                    &(profile, arena.classes),
                    4096,
                )?,
            },
        })
    }

    pub(super) fn record(
        &mut self,
        state: &mut State,
        expected: &CoreProfile,
        job: &ThinkJob<'_>,
        result: &ThinkResult,
    ) -> Result<(), RatingError> {
        if self.terminal
            || self.summary.total_thinks >= state.maximum_thinks
            || job.unit.0 != job.view.unit
            || *job.profile != self.classes.row(job.profile.class)
            || job.profile.class > self.tested
        {
            #[cfg(test)]
            eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
            #[cfg(test)]
            eprintln!(
                "M7_PHYSICAL_RECORD total={} maximum={} sequence={} class={:?} tested={:?} dropped={} exact_unit={} exact_profile={}",
                self.summary.total_thinks,
                state.maximum_thinks,
                job.think_seq,
                job.profile.class,
                self.tested,
                job.dropped,
                job.unit.0 == job.view.unit,
                *job.profile == self.classes.row(job.profile.class),
            );
            return Err(RatingError::Identity);
        }
        let unit = self.units.get_mut(&job.unit).ok_or(RatingError::Identity)?;
        if job.think_seq != unit.next_call || unit.pending.len() >= MAX_PENDING_PER_UNIT {
            return Err(RatingError::Incomplete);
        }
        let source = unit.bindings.iter().any(|binding| binding.matches(job));
        let kind = if job.dropped
            && job.profile.class == CoreClass::Zero
            && (source || unit.rom.matches(job))
        {
            Kind::Rom
        } else if !job.dropped && source && job.profile.class != CoreClass::Zero {
            if job.profile == expected {
                Kind::Tested
            } else {
                Kind::Degraded
            }
        } else {
            #[cfg(test)]
            eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
            return Err(RatingError::Identity);
        };
        let counts = result.diagnostics.as_ref().and_then(|d| d.counts());
        let fault = result.fault.as_ref().map(|f| Fault::from_cause(&f.cause));
        match kind {
            Kind::Tested => super::record(state, expected, job, result)?,
            Kind::Degraded => {
                let (ops, fees) = counts.ok_or(RatingError::Incomplete)?;
                if fault.is_none()
                    && ops.checked_add(fees).ok_or(RatingError::Bound)? > job.profile.ops
                {
                    return Err(RatingError::Invalid);
                }
                self.summary.degraded_thinks = self
                    .summary
                    .degraded_thinks
                    .checked_add(1)
                    .ok_or(RatingError::Bound)?;
                self.summary.degraded_max_ops = self.summary.degraded_max_ops.max(ops);
                self.summary.degraded_max_fees = self.summary.degraded_max_fees.max(fees);
                if fault == Some(Fault::Ops) {
                    self.summary.degraded_ops_faults = self
                        .summary
                        .degraded_ops_faults
                        .checked_add(1)
                        .ok_or(RatingError::Bound)?;
                }
            }
            Kind::Rom => {
                if counts.is_some() || fault.is_some() {
                    return Err(RatingError::Invalid);
                }
                self.summary.rom_thinks = self
                    .summary
                    .rom_thinks
                    .checked_add(1)
                    .ok_or(RatingError::Bound)?;
            }
        }
        if fault == Some(Fault::Ops) {
            unit.ops_faults = unit.ops_faults.checked_add(1).ok_or(RatingError::Bound)?;
        }
        unit.pending.insert(
            job.think_seq,
            Invocation {
                class: job.profile.class,
                kind,
                t_ms: job.t_ms,
                lands_ms: job.lands_ms,
                counts,
                fault,
                captured_rom: unit.rom.matches(job),
                checked: false,
                token: job.profile_token,
            },
        );
        unit.next_call = unit.next_call.checked_add(1).ok_or(RatingError::Bound)?;
        self.summary.total_thinks = self
            .summary
            .total_thinks
            .checked_add(1)
            .ok_or(RatingError::Bound)?;
        self.summary.witness = identity(
            "tacsim.registry.physical.call.v1",
            &(
                self.summary.witness,
                state
                    .units
                    .get(&job.unit)
                    .ok_or(RatingError::Identity)?
                    .ordinal,
                job.think_seq,
                job.t_ms,
                job.view.t_ms,
                job.lands_ms,
                job.profile,
                job.profile_token,
                job.program.digest,
                kind,
                source,
                counts,
                fault,
            ),
            4096,
        )?;
        self.dirty = true;
        Ok(())
    }

    pub(super) fn reconcile(&mut self, arena: &Arena) -> Result<(), RatingError> {
        if self.terminal || arena.classes != self.classes {
            #[cfg(test)]
            eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
            return Err(RatingError::Identity);
        }
        for (ordinal, (id, unit)) in self.units.iter_mut().enumerate() {
            let own = arena.unit(*id).ok_or(RatingError::Incomplete)?;
            let mind = arena.minds.get(id).ok_or(RatingError::Incomplete)?;
            let runtime = arena.profiles.get(id).ok_or(RatingError::Incomplete)?;
            let core = arena.core_of(*id);
            let previous = unit
                .core
                .as_ref()
                .map_or(CoreClass::Zero, |c| c.class)
                .min(self.tested);
            let current = core
                .as_ref()
                .map_or(CoreClass::Zero, |c| c.class)
                .min(self.tested);
            if own.seal.class != self.tested
                || own.seal.core_class != unit.sealed_core
                || runtime.diagnostic_override
                || (arena.ended.is_none() && mind.think_seq != unit.next_call)
                || u64::from(mind.strikes) > unit.ops_faults
                || matches!(mind.dropped, Some(DropCause::Strikes | DropCause::Abi))
            {
                #[cfg(test)]
                eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                #[cfg(test)]
                eprintln!(
                    "M7_PHYSICAL_STATE ordinal={ordinal} now_ms={} ended={} next_call={} next_trace={} think_seq={} pending={} strikes={} measured_ops_faults={} drop={:?} old_class={previous:?} current_class={current:?} previous_generation={} current_generation={} runtime_dropped={} diagnostic_override={} seal_matches={} core_seal_matches={}",
                    arena.now_ms,
                    arena.ended.is_some(),
                    unit.next_call,
                    unit.next_trace,
                    mind.think_seq,
                    unit.pending.len(),
                    mind.strikes,
                    unit.ops_faults,
                    mind.dropped,
                    unit.token.generation,
                    runtime.token().generation,
                    runtime.dropped,
                    runtime.diagnostic_override,
                    own.seal.class == self.tested,
                    own.seal.core_class == unit.sealed_core,
                );
                return Err(RatingError::Identity);
            }
            match (&unit.core, &core) {
                (None, Some(_)) => return Err(RatingError::Identity),
                (Some(old), Some(now))
                    if old.component != now.component
                        || old.governed != now.governed
                        || now.class > old.class =>
                {
                    #[cfg(test)]
                    eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                    return Err(RatingError::Identity);
                }
                _ => {}
            }
            if runtime.rom.key() != unit.rom.key
                || runtime.rom.canonical_bytes() != Some(unit.rom.card.as_slice())
            {
                #[cfg(test)]
                eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                return Err(RatingError::Identity);
            }
            if mind.dropped == Some(DropCause::Core) {
                if core.is_some()
                    || !runtime.dropped
                    || !unit.rom.matches_parts(&runtime.card, &runtime.program)
                {
                    #[cfg(test)]
                    eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                    return Err(RatingError::Identity);
                }
            } else if runtime.dropped
                || !unit
                    .bindings
                    .iter()
                    .any(|binding| binding.matches_parts(&runtime.card, &runtime.program))
            {
                #[cfg(test)]
                eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                return Err(RatingError::Identity);
            }
            for invocation in unit.pending.values_mut().filter(|call| !call.checked) {
                // A captured source answer may precede this step's physical
                // loss. The previous/current hardware interval preserves it.
                if invocation.class > previous
                    || invocation.class < current
                    || (invocation.kind == Kind::Rom && current != CoreClass::Zero)
                    || (invocation.token != unit.token && invocation.token != runtime.token())
                {
                    #[cfg(test)]
                    eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                    #[cfg(test)]
                    eprintln!(
                        "M7_PHYSICAL_INVOCATION ordinal={ordinal} class={:?} kind={:?} previous={previous:?} current={current:?} call_generation={} previous_generation={} current_generation={} captured_rom={}",
                        invocation.class,
                        invocation.kind,
                        invocation.token.generation,
                        unit.token.generation,
                        runtime.token().generation,
                        invocation.captured_rom,
                    );
                    return Err(RatingError::Identity);
                }
                if invocation.kind == Kind::Rom {
                    if invocation.captured_rom {
                        if core.is_some() || mind.dropped != Some(DropCause::Core) {
                            #[cfg(test)]
                            eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                            return Err(RatingError::Identity);
                        }
                    } else if !core.as_ref().is_some_and(|c| c.class == CoreClass::Zero)
                        && !unit
                            .core
                            .as_ref()
                            .is_some_and(|c| c.class == CoreClass::Zero)
                    {
                        #[cfg(test)]
                        eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                        return Err(RatingError::Identity);
                    }
                }
                if invocation.fault.is_some_and(|fault| fault != Fault::Ops) {
                    return Err(RatingError::Invalid);
                }
                invocation.checked = true;
            }
            let traces = arena.traces.recent(*id, Some(THINKS));
            if arena.ended.is_some() {
                Self::cancel_uncalled(&mut self.summary, arena, ordinal, *id, unit, &traces)?;
            }
            self.summary.witness = identity(
                "tacsim.registry.physical.boundary.v1",
                &(
                    self.summary.witness,
                    ordinal,
                    arena.now_ms,
                    unit.core
                        .as_ref()
                        .map(|c| (c.class, &c.component, c.governed)),
                    core.as_ref().map(|c| (c.class, &c.component, c.governed)),
                    mind.dropped,
                    runtime.token(),
                    runtime.dropped,
                ),
                4096,
            )?;
            unit.core = core;
            unit.token = runtime.token();
            unit.interrupted = current == CoreClass::Zero;
            for row in &traces {
                if row.think_seq < unit.next_trace {
                    continue;
                }
                if row.think_seq != unit.next_trace {
                    return Err(RatingError::Incomplete);
                }
                // The authenticated terminal suffix was never called. It is
                // separate from both call and completed-trace sequence counts.
                if arena.ended.is_some() && row.think_seq >= unit.next_call {
                    break;
                }
                let call = unit
                    .pending
                    .get(&row.think_seq)
                    .ok_or(RatingError::Incomplete)?;
                if !call.checked || row.t_ms != call.t_ms || row.rom != (call.kind == Kind::Rom) {
                    #[cfg(test)]
                    eprintln!(
                        "M7_PHYSICAL_TRACE ordinal={ordinal} seq={} checked={} row_ms={} call_ms={} row_rom={} kind={:?} settled={} pending={}",
                        row.think_seq,
                        call.checked,
                        row.t_ms,
                        call.t_ms,
                        row.rom,
                        call.kind,
                        row.settled,
                        matches!(row.outcome, Outcome::Pending),
                    );
                    #[cfg(test)]
                    eprintln!("M7_PHYSICAL_IDENTITY file={} line={}", file!(), line!());
                    return Err(RatingError::Identity);
                }
                if !row.settled {
                    if arena.ended.is_some() {
                        return Err(RatingError::Incomplete);
                    }
                    break;
                }
                let terminal_pending =
                    arena.ended.is_some() && matches!(row.outcome, Outcome::Pending);
                if matches!(row.outcome, Outcome::Pending) && !terminal_pending {
                    break;
                }
                if row.fault != call.fault
                    || row.diagnostics.counts() != call.counts
                    || row.rom_drop.is_some_and(|drop| {
                        drop != RomDrop::Core
                            || unit.core.is_some()
                            || mind.dropped != Some(DropCause::Core)
                    })
                {
                    return Err(RatingError::Invalid);
                }
                if row.diagnostics.omitted_intents() != 0 {
                    return Err(RatingError::Bound);
                }
                if terminal_pending {
                    let pending = mind.pending.as_ref().ok_or(RatingError::Incomplete)?;
                    if pending.think_seq != row.think_seq
                        || pending.t_ms != call.t_ms
                        || pending.lands_ms != call.lands_ms
                        || pending.profile_token != call.token
                        || pending.profile_class != call.class
                        || arena.awaiting.contains(id)
                        || row.fault.is_some()
                        || row.rom_drop.is_some()
                        || row.landed_ms.is_some()
                        || row
                            .intents
                            .iter()
                            .any(|intent| !matches!(intent.outcome, Outcome::Pending))
                    {
                        return Err(RatingError::Identity);
                    }
                    self.summary.cancelled_pending = self
                        .summary
                        .cancelled_pending
                        .checked_add(1)
                        .ok_or(RatingError::Bound)?;
                    self.summary.witness = identity(
                        "tacsim.registry.physical.cancelled_call.v1",
                        &(
                            self.summary.witness,
                            ordinal,
                            row.think_seq,
                            call.t_ms,
                            call.lands_ms,
                            call.token,
                            call.kind,
                            call.counts,
                        ),
                        4096,
                    )?;
                    // The actual invocation already contributed its costs and
                    // fault evidence. No landing/refusal/coverage is invented.
                    unit.pending.remove(&row.think_seq);
                    unit.next_trace = unit.next_trace.checked_add(1).ok_or(RatingError::Bound)?;
                    continue;
                }
                let refused = u32::try_from(
                    row.intents
                        .iter()
                        .filter(|i| matches!(i.outcome, Outcome::Refused { .. }))
                        .count(),
                )
                .map_err(|_| RatingError::Bound)?;
                self.summary.refusals = self
                    .summary
                    .refusals
                    .checked_add(refused)
                    .ok_or(RatingError::Bound)?;
                self.summary.witness = identity(
                    "tacsim.registry.physical.landing.v1",
                    &(
                        self.summary.witness,
                        ordinal,
                        row.think_seq,
                        row.landed_ms,
                        row.rom_drop,
                        &row.outcome,
                        refused,
                    ),
                    4096,
                )?;
                unit.pending.remove(&row.think_seq);
                unit.next_trace = unit.next_trace.checked_add(1).ok_or(RatingError::Bound)?;
            }
            // A vanished trace is not an empty/healthy ring. Pending landing
            // evidence must remain present until its terminal join is consumed.
            if unit
                .pending
                .keys()
                .any(|seq| !traces.iter().any(|row| row.think_seq == *seq))
            {
                return Err(RatingError::Incomplete);
            }
        }
        if let Some(end) = arena.ended {
            if end.t_ms > arena.now_ms {
                return Err(RatingError::Identity);
            }
            self.summary.witness = identity(
                "tacsim.registry.physical.terminal.v1",
                &(
                    self.summary.witness,
                    end,
                    arena.now_ms,
                    crate::arena::state_hash(arena),
                    arena.ledger.digest(),
                ),
                4096,
            )?;
            self.terminal = true;
        }
        self.dirty = false;
        Ok(())
    }

    /// `Arena::step` suppresses its worker batch after a real end, while the
    /// last substep can already have collected a job. Authenticate that exact
    /// retained suffix; do not pretend a runner was called or a think landed.
    fn cancel_uncalled(
        summary: &mut PhysicalSummary,
        arena: &Arena,
        ordinal: usize,
        id: ObjId,
        unit: &Unit,
        traces: &[ThinkTrace],
    ) -> Result<(), RatingError> {
        let end = arena.ended.ok_or(RatingError::Identity)?;
        let mind = arena.minds.get(&id).ok_or(RatingError::Incomplete)?;
        let runtime = arena.profiles.get(&id).ok_or(RatingError::Incomplete)?;
        let core = arena.core_of(id);
        let tested = arena.unit(id).ok_or(RatingError::Incomplete)?.seal.class;
        let previous = unit
            .core
            .as_ref()
            .map_or(CoreClass::Zero, |c| c.class)
            .min(tested);
        let current = core
            .as_ref()
            .map_or(CoreClass::Zero, |c| c.class)
            .min(tested);
        let mut count = 0_usize;
        for think in arena
            .thinks_outstanding()
            .iter()
            .filter(|think| think.unit == id)
        {
            if count >= MAX_PENDING_PER_UNIT {
                return Err(RatingError::Bound);
            }
            let sequence = unit
                .next_call
                .checked_add(u64::try_from(count).map_err(|_| RatingError::Bound)?)
                .ok_or(RatingError::Bound)?;
            let row = traces
                .iter()
                .find(|row| row.think_seq == sequence)
                .ok_or(RatingError::Incomplete)?;
            let job = think.job();
            let source = unit.bindings.iter().any(|binding| binding.matches(&job));
            let captured_rom = unit.rom.matches(&job);
            // A same-substep impact can destroy this hull after collection.
            // Its real destruction path cancels the trace, clears pending and
            // awaiting, and retains the uncalled job. The cancellation is not
            // a runner result and must never add source/cost/coverage credit.
            let wreck_cancelled = matches!(
                row.outcome,
                Outcome::Cancelled {
                    reason: Cancelled::Wreck
                }
            );
            if wreck_cancelled {
                let own = arena.unit(id).ok_or(RatingError::Incomplete)?;
                if !own.limits.wreck
                    || !crate::arena::damage::is_wreck(&own.unit)
                    || !own.seal.closed
                    || own.seal.sortie != unit.sortie
                    || mind.pending.is_some()
                    || arena.awaiting.contains(&id)
                    || row.landed_ms != Some(arena.now_ms)
                    || think.view.t_ms != arena.now_ms
                {
                    return Err(RatingError::Identity);
                }
            } else {
                let pending = mind.pending.as_ref().ok_or(RatingError::Incomplete)?;
                if pending.think_seq != sequence
                    || pending.t_ms != think.t_ms
                    || pending.lands_ms != think.lands_ms
                    || pending.epochs != think.epochs
                    || pending.profile_token != think.profile_token
                    || pending.profile_class != think.profile.class
                    || pending.period_ms != think.profile.think_ms
                    || pending.rule_sample.is_some()
                    || !pending.intents.is_empty()
                    || !pending.memory.is_empty()
                    || !arena.awaiting.contains(&id)
                    || row.landed_ms.is_some()
                    || !matches!(row.outcome, Outcome::Pending)
                {
                    return Err(RatingError::Identity);
                }
            }
            if think.think_seq != sequence
                || job.view.unit != id.0
                || think.seed != arena.seed
                || !Arc::ptr_eq(&think.map, &arena.definition)
                || think.t_ms > think.view.t_ms
                || think.view.t_ms > arena.now_ms
                || think.lands_ms != think.t_ms.saturating_add(arena.substep_len_ms())
                || !think
                    .substep_hash
                    .is_some_and(|hash| arena.substep_hashes.contains(&hash))
                || think.profile != arena.classes.row(think.profile.class)
                || think.profile.class > previous
                || think.profile.class < current
                || (think.profile_token != unit.token && think.profile_token != runtime.token())
                || row.settled
                || row.t_ms != think.t_ms
                || row.rom != think.dropped
                || row.fault.is_some()
                || row.rom_drop.is_some()
                || row.diagnostics != ThinkDiagnostics::default()
                || !row.author.is_empty()
                || !row.intents.is_empty()
            {
                return Err(RatingError::Identity);
            }
            let kind = if think.dropped
                && think.profile.class == CoreClass::Zero
                && current == CoreClass::Zero
                && (source || captured_rom)
            {
                if captured_rom {
                    if core.is_some() || mind.dropped != Some(DropCause::Core) {
                        return Err(RatingError::Identity);
                    }
                } else if !core.as_ref().is_some_and(|c| c.class == CoreClass::Zero)
                    && !unit
                        .core
                        .as_ref()
                        .is_some_and(|c| c.class == CoreClass::Zero)
                {
                    return Err(RatingError::Identity);
                }
                Kind::Rom
            } else if !think.dropped && source && think.profile.class != CoreClass::Zero {
                if think.profile.class == tested {
                    Kind::Tested
                } else {
                    Kind::Degraded
                }
            } else {
                return Err(RatingError::Identity);
            };
            summary.witness = identity(
                "tacsim.registry.physical.cancelled_uncalled.v1",
                &(
                    summary.witness,
                    ordinal,
                    sequence,
                    think.t_ms,
                    think.view.t_ms,
                    think.lands_ms,
                    think.epochs,
                    think.profile,
                    think.profile_token,
                    think.program.digest,
                    think.substep_hash,
                    kind,
                    wreck_cancelled,
                    unit.sortie,
                    row.landed_ms,
                    end,
                ),
                4096,
            )?;
            count = count.checked_add(1).ok_or(RatingError::Bound)?;
        }
        let count_u64 = u64::try_from(count).map_err(|_| RatingError::Bound)?;
        if mind.think_seq
            != unit
                .next_call
                .checked_add(count_u64)
                .ok_or(RatingError::Bound)?
            || traces
                .iter()
                .filter(|row| row.think_seq >= unit.next_call)
                .count()
                != count
        {
            return Err(RatingError::Incomplete);
        }
        summary.cancelled_uncalled = summary
            .cancelled_uncalled
            .checked_add(count_u64)
            .ok_or(RatingError::Bound)?;
        Ok(())
    }

    pub(super) fn summary(&self) -> Result<PhysicalSummary, RatingError> {
        if self.dirty {
            return Err(RatingError::Incomplete);
        }
        Ok(self.summary.clone())
    }
    pub(super) fn require_complete(&self) -> Result<(), RatingError> {
        if self.dirty
            || self
                .units
                .values()
                .any(|unit| !unit.pending.is_empty() || unit.next_trace != unit.next_call)
        {
            return Err(RatingError::Incomplete);
        }
        Ok(())
    }
    pub(super) fn interrupted(&self, id: ObjId) -> Result<bool, RatingError> {
        if self.dirty {
            return Err(RatingError::Incomplete);
        }
        Ok(self
            .units
            .get(&id)
            .ok_or(RatingError::Identity)?
            .interrupted)
    }
}

#[cfg(test)]
mod tests;
