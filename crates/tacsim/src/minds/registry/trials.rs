//! Actual combat.v1 whole matches. The only constructor consumes immutable
//! content/configuration and an exact prepared subject; it cannot wrap a live
//! arena. One native step is the largest execution turn exposed to the worker.
use super::{
    identity,
    manifest::FrozenPolicy,
    meter::MeteredRunner,
    plan::{Case, Physics},
    runtime,
    subject::PreparedSubject,
};
use crate::{
    arena::{
        Arena, ArenaMode, Channel, Controller, CustodyOut, EndReason, MatchMode, PartyId,
        author_trace::{Fault, Outcome, TraceStore},
        core::CoreClass,
        intents::{Intent, IntentKind, IntentOutcome, IntentSource, apply_intent},
        mind::SealEngine,
        profile_seal::SealedCards,
    },
    arena_task::MatchId,
    match_::{SealIdentity, SealTerms},
    minds::{RhaiRunner, STANDARD_RHAI, standard_digest},
};
use mush_core::{CanonicalId, DefinitionDigest, ObjId};
use mush_mind::{EngineLimits, Sandbox};
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, Coverage, RatingClass,
    RatingError, TrialEnd, TrialLayout, TrialOutcome,
};
use mush_sim::data::Pose;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layouts {
    format: u16,
    ground: String,
    chassis: BTreeMap<String, String>,
    scenarios: Vec<Layout>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    name: String,
    mode: String,
    red: Vec<[u16; 2]>,
    blue: Vec<[u16; 2]>,
}

pub(super) struct Trial {
    arena: Arena,
    case: Case,
    meter: Arc<MeteredRunner>,
    binding: CaseBinding,
    candidates: Vec<ObjId>,
    swapped: bool,
    cycle: bool,
    profiles: u8,
    next_cycle_ms: i64,
    next_profile: u8,
    wall: i64,
    refusals: u32,
}
pub(super) struct Slice {
    pub(super) simulated_ms: u64,
    pub(super) result: Option<CaseResult>,
}
impl Trial {
    pub(super) fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        let CaseKind::Trial {
            variant,
            layout,
            seed,
            swapped,
        } = case.key.kind()
        else {
            return Err(RatingError::Invalid);
        };
        let variant = subject
            .variants()
            .get(usize::from(variant))
            .ok_or(RatingError::Identity)?;
        let class = core(case.key.class());
        let (mut arena, roms) = opening(policy, layout, seed, swapped, class)?;
        let actual = Physics {
            units: u16::try_from(arena.units.len()).map_err(|_| RatingError::Bound)?,
            native_ms: u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?,
        };
        if actual != case.physics {
            return Err(RatingError::Identity);
        }
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            variant.initial,
            class,
            &roms,
            case.maximum_thinks,
        )?;
        arena.traces.set_enabled(true);
        let candidates = roms.keys().copied().collect::<Vec<_>>();
        installed.meter.enable_physical(&arena)?;
        Ok(Self {
            arena,
            case: case.clone(),
            meter: installed.meter,
            binding: installed.binding,
            candidates,
            swapped,
            cycle: variant.cycle,
            profiles: u8::try_from(variant.definition.profiles.len())
                .map_err(|_| RatingError::Bound)?,
            next_cycle_ms: 10_000,
            next_profile: 1,
            wall: 0,
            refusals: 0,
        })
    }
    /// Capture the real cadence/roster before the quota Plan is constructed.
    pub(super) fn physics(
        policy: &FrozenPolicy,
        layout: TrialLayout,
        class: CoreClass,
    ) -> Result<Physics, RatingError> {
        let (arena, _) = opening(policy, layout, 1, false, class)?;
        Ok(Physics {
            units: u16::try_from(arena.units.len()).map_err(|_| RatingError::Bound)?,
            native_ms: u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?,
        })
    }
    pub(super) const fn binding(&self) -> &CaseBinding {
        &self.binding
    }
    /// Failure-only test output is bounded by the actual candidate roster. It
    /// contains counters/flags, never source, memory, a seal, or a trace payload.
    #[cfg(test)]
    pub(super) fn failure_diagnostic(&self) {
        let meter = self.meter.measurement().map(|value| value.summary);
        eprintln!(
            "M7_REGISTRY_TRIAL_FAILURE now_ms={} wall={} ended={} meter={meter:?} refusals={}",
            self.arena.now_ms,
            self.wall,
            self.arena.ended.is_some(),
            self.refusals,
        );
        for (ordinal, id) in self.candidates.iter().enumerate() {
            if let Some(state) = self.arena.minds.get(id) {
                let rows = self.arena.traces.recent(*id, None);
                eprintln!(
                    "M7_REGISTRY_TRIAL_UNIT ordinal={ordinal} think_seq={} strikes={} dropped={:?} core_class={:?} mind_class={:?} sealed_class={:?} tested_class={:?} trace_rows={} first_seq={:?} last_seq={:?}",
                    state.think_seq,
                    state.strikes,
                    state.dropped,
                    self.arena.core_of(*id).map(|core| core.class),
                    self.arena.mind_class(*id),
                    self.arena.unit(*id).map(|unit| unit.seal.class),
                    self.case.key.class(),
                    rows.len(),
                    rows.first().map(|row| row.think_seq),
                    rows.last().map(|row| row.think_seq),
                );
                if let Some(runtime) = self.arena.profiles.get(id) {
                    eprintln!(
                        "M7_REGISTRY_TRIAL_RUNTIME ordinal={ordinal} generation={} dropped={} diagnostic_override={} cycle={} next_cycle_ms={} pending_think_seq={:?}",
                        runtime.generation,
                        runtime.dropped,
                        runtime.diagnostic_override,
                        self.cycle,
                        self.next_cycle_ms,
                        state.pending.as_ref().map(|pending| pending.think_seq),
                    );
                }
            }
        }
    }
    pub(super) fn advance(&mut self, allowance_ms: u64) -> Result<Slice, RatingError> {
        let cadence = u64::from(self.case.physics.native_ms);
        if allowance_ms < cadence || self.arena.ended.is_some() {
            return Err(RatingError::Invalid);
        }
        if self.cycle && self.profiles > 1 && self.arena.now_ms >= self.next_cycle_ms {
            for id in &self.candidates {
                // Destroyed hulls have no next thought. Surviving hulls must
                // accept the ordinary recorded profile command and cooldown.
                if !self.meter.physically_interrupted(*id)?
                    && self
                        .arena
                        .unit(*id)
                        .is_some_and(|u| !u.limits.wreck && !u.seal.closed)
                {
                    let intent = Intent {
                        unit: *id,
                        channel: Channel::Helm,
                        epoch: self.arena.epochs.current(*id, Channel::Helm),
                        kind: IntentKind::Profile {
                            index: self.next_profile,
                        },
                        seq: u64::try_from(self.next_cycle_ms).map_err(|_| RatingError::Bound)?,
                    };
                    if apply_intent(&mut self.arena, &intent, IntentSource::Seat)
                        != IntentOutcome::AppliedContinuous
                    {
                        return Err(RatingError::Incomplete);
                    }
                }
            }
            self.next_profile = (self.next_profile + 1) % self.profiles;
            self.next_cycle_ms = self
                .next_cycle_ms
                .checked_add(10_000)
                .ok_or(RatingError::Bound)?;
            // Bind the actual new source token before the next step can lose
            // its core. No pending invocation is replaced or re-executed.
            self.meter.reconcile(&self.arena)?;
        }
        let before = self.arena.now_ms;
        self.wall = self
            .wall
            .checked_add(i64::from(self.case.physics.native_ms))
            .ok_or(RatingError::Bound)?;
        // This is the same complete step/answer barrier as production proving.
        // It advances exactly one captured native step and preserves pending
        // landings, native evaluation and memory between invocations.
        let out = self.arena.step_settled(self.wall, &[], &[]);
        let simulated_ms =
            u64::try_from(self.arena.now_ms - before).map_err(|_| RatingError::Invalid)?;
        if simulated_ms > cadence {
            return Err(RatingError::Bound);
        }
        drain(&mut self.arena, &out.ledger_chunks)?;
        self.meter.reconcile(&self.arena)?;
        let measured = self.meter.measurement()?;
        let physical = measured.physical.as_ref().ok_or(RatingError::Incomplete)?;
        self.refusals = physical.refusals;
        // Lower-class exhaustion is retained separately and refuses this case;
        // it is never misreported as a budget failure at the tested class.
        if physical.degraded_ops_faults != 0 {
            return Err(RatingError::Invalid);
        }
        if !measured.summary.clean() && !measured.summary.only_budget_faults() {
            return Err(RatingError::Invalid);
        }
        let budget = measured.summary.only_budget_faults();
        if !budget && self.arena.ended.is_none() {
            if u64::try_from(self.arena.now_ms).map_err(|_| RatingError::Bound)?
                > self.case.maximum_ms + cadence
            {
                return Err(RatingError::Bound);
            }
            return Ok(Slice {
                simulated_ms,
                result: None,
            });
        }
        if self.arena.ended.is_some() {
            self.meter.require_complete()?;
        }
        let terminal = self
            .arena
            .ended
            .map(|end| {
                TrialEnd::new(
                    u64::try_from(self.arena.now_ms).map_err(|_| RatingError::Bound)?,
                    crate::arena::state_hash(&self.arena),
                    DefinitionDigest::from_bytes(self.arena.ledger.digest()),
                    outcome(end.reason, self.swapped)?,
                )
            })
            .transpose()?;
        // At least one real invocation is required for each submitted hull.
        // Full program/preset coverage is independently proved by direct cases.
        let invoked = measured
            .seen
            .iter()
            .filter(|row| row.iter().any(|n| *n > 0))
            .count();
        let coverage = Coverage::new(
            Applicability::Armed,
            u32::try_from(self.candidates.len()).map_err(|_| RatingError::Bound)?,
            u32::try_from(invoked).map_err(|_| RatingError::Bound)?,
            identity(
                "tacsim.registry.trial.coverage.v1",
                &(
                    measured.witness,
                    physical,
                    &measured.seen,
                    self.refusals,
                    self.arena.now_ms,
                    crate::arena::state_hash(&self.arena),
                ),
                16 * 1024,
            )?,
        )?;
        let result = CaseResult::new(
            self.case.key.clone(),
            self.binding.clone(),
            measured.summary,
            coverage,
            self.refusals,
            if budget {
                CaseOutcome::BudgetExceeded {}
            } else {
                CaseOutcome::Conformant {}
            },
            terminal,
        )?;
        Ok(Slice {
            simulated_ms,
            result: Some(result),
        })
    }
}

/// A bounded diagnostic join, not another execution engine. Every terminal
/// trace row is consumed in sequence once; a ring gap fails the fixture rather
/// than silently undercounting refusals. No source/memory/trace text is retained.
pub(super) struct Landings {
    next: BTreeMap<ObjId, u64>,
    refusals: u32,
}
impl Landings {
    pub(super) fn new(units: &[ObjId]) -> Self {
        Self {
            next: units.iter().map(|id| (*id, 0)).collect(),
            refusals: 0,
        }
    }
    pub(super) fn refusals(&self) -> u32 {
        self.refusals
    }
    pub(super) fn collect(&mut self, traces: &TraceStore) -> Result<(), RatingError> {
        for (id, next) in &mut self.next {
            for row in traces.recent(*id, None) {
                if row.think_seq < *next {
                    continue;
                }
                if row.think_seq != *next {
                    return Err(RatingError::Incomplete);
                }
                if !row.settled || matches!(row.outcome, Outcome::Pending) {
                    break;
                }
                if row.rom || row.rom_drop.is_some() || row.fault.is_some_and(|f| f != Fault::Ops) {
                    return Err(RatingError::Invalid);
                }
                if row.diagnostics.omitted_intents() != 0 {
                    return Err(RatingError::Bound);
                }
                let refused = u32::try_from(
                    row.intents
                        .iter()
                        .filter(|i| matches!(i.outcome, Outcome::Refused { .. }))
                        .count(),
                )
                .map_err(|_| RatingError::Bound)?;
                self.refusals = self
                    .refusals
                    .checked_add(refused)
                    .ok_or(RatingError::Bound)?;
                *next = next.checked_add(1).ok_or(RatingError::Bound)?;
            }
        }
        Ok(())
    }
}
fn opening(
    policy: &FrozenPolicy,
    layout: TrialLayout,
    seed: u8,
    swapped: bool,
    class: CoreClass,
) -> Result<(Arena, BTreeMap<ObjId, SealedCards>), RatingError> {
    let content: Layouts =
        serde_json::from_str(include_str!("../../../content/proving/scenarios.json"))
            .map_err(|_| RatingError::Invalid)?;
    if content.format != 1
        || content.ground != policy.ground.id
        || content.scenarios.len() != 3
        || !matches!(seed, 1 | 2)
    {
        return Err(RatingError::Identity);
    }
    let name = match layout {
        TrialLayout::Duel => "duel",
        TrialLayout::Muster => "muster",
        TrialLayout::Hold => "hold",
    };
    let layout = content
        .scenarios
        .iter()
        .find(|r| r.name == name)
        .ok_or(RatingError::Incomplete)?;
    let mode = match layout.mode.as_str() {
        "skirmish" => MatchMode::Skirmish,
        "hold"
            if policy
                .ground
                .modes
                .iter()
                .any(|m| matches!(m, ArenaMode::Hold(_))) =>
        {
            MatchMode::Hold
        }
        _ => return Err(RatingError::Invalid),
    };
    if (!layout.red.is_empty() && layout.red.len() != 6)
        || (!layout.blue.is_empty() && layout.blue.len() != 6)
    {
        return Err(RatingError::Invalid);
    }
    let chassis = id(content
        .chassis
        .get(class.as_str())
        .ok_or(RatingError::Incomplete)?)?;
    let catalog = &policy.catalog;
    let spec = catalog
        .specs
        .get(&chassis)
        .ok_or(RatingError::Unavailable)?;
    let binding = catalog
        .bindings
        .get(&chassis)
        .ok_or(RatingError::Unavailable)?;
    let authored = catalog
        .arenas
        .get(&id(&content.ground)?)
        .ok_or(RatingError::Unavailable)?;
    let resolution = catalog
        .resolutions
        .get(&id(crate::definitions::content::RESOLUTION)?)
        .cloned()
        .ok_or(RatingError::Unavailable)?;
    let mut units = Vec::new();
    let mut occupied = Vec::new();
    let mut roms = BTreeMap::new();
    let mut opponents = Vec::new();
    let per_side: u8 = if name == "duel" { 1 } else { 2 };
    for team in [PartyId::RED, PartyId::BLUE] {
        let candidate = (team == PartyId::RED) != swapped;
        let terms = terms(policy, candidate);
        let positions = if team == PartyId::RED {
            &layout.red
        } else {
            &layout.blue
        };
        let authored_slots = authored
            .tanks
            .iter()
            .filter(|u| u.team == team)
            .collect::<Vec<_>>();
        for n in 0..per_side {
            let object = ObjId((if team == PartyId::RED { 100 } else { 200 }) + u32::from(n));
            let [x, y] = if positions.is_empty() {
                let slot = authored_slots
                    .get(usize::from(n))
                    .ok_or(RatingError::Incomplete)?;
                [slot.x, slot.y]
            } else {
                *positions
                    .get(usize::from(n))
                    .ok_or(RatingError::Incomplete)?
            };
            let cell = i64::from(policy.ground.terrain.cell_cm());
            let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
                .map_err(|_| RatingError::Invalid)?;
            let mut proposed = Pose::at(
                i64::from(x) * cell + cell / 2,
                i64::from(y) * cell + cell / 2,
                0,
                posture,
            );
            proposed.heading_mdeg = if team == PartyId::RED {
                90_000
            } else {
                270_000
            };
            let pose = crate::match_::opening_pose(
                &policy.ground,
                team,
                Some(proposed),
                &mut occupied,
                crate::arena::state::CostRow::Tracked,
            )
            .ok_or(RatingError::Invalid)?;
            let cells = crate::definitions::to_tactical_unit(spec, binding, object)
                .map_err(|_| RatingError::Invalid)?;
            let unit = crate::match_::custody_unit(
                catalog,
                cells,
                object,
                team,
                pose,
                1,
                Controller::Mind,
                n,
                &terms,
            )
            .map_err(|_| RatingError::Invalid)?;
            if candidate {
                roms.insert(object, unit.seal.rom.clone());
            } else {
                opponents.push(object);
            }
            units.push(unit);
        }
    }
    let mut arena = Arena::new_with_minds(
        CustodyOut {
            definition: (*policy.ground).clone(),
            units,
            pilots: Vec::new(),
            resolution,
            seed: u64::from(seed),
            mode,
            host: crate::arena::FieldHost::all_hostile(),
        },
        &policy.cfg.arena,
        &policy.cfg.match_,
        &policy.cfg.sensors,
        &policy.cfg.minds,
    )
    .map_err(|_| RatingError::Invalid)?;
    arena.clock.reanchor(0);
    // Standard's exact current source executes in this isolated cache. Its
    // declared Class I and captured Line/ROM policy are unchanged.
    let sandbox = Arc::new(Sandbox::new(
        &EngineLimits::default(),
        crate::minds::register_api,
    ));
    let standard = Arc::new(
        RhaiRunner::on(sandbox, &policy.cfg.arena, STANDARD_RHAI, standard_digest())
            .map_err(|_| RatingError::Invalid)?,
    );
    for object in opponents {
        arena.runners.insert(object, standard.clone());
    }
    Ok((arena, roms))
}
fn terms(policy: &FrozenPolicy, candidate: bool) -> SealTerms {
    SealTerms {
        authored: None,
        selected_profiles: Vec::new(),
        classes: policy.cfg.minds.classes,
        rom_cfg: policy.cfg.arena.reflex.clone(),
        match_id: MatchId(1),
        principal: None,
        card: crate::arena::card::line(&policy.cfg.arena.reflex),
        odds_floor_permille: policy.cfg.minds.odds_floor_permille,
        engine_for_minds: if candidate {
            SealEngine::Native
        } else {
            SealEngine::Scripted
        },
        rated: if candidate {
            CoreClass::Zero
        } else {
            CoreClass::I
        },
        class_cap: if candidate {
            CoreClass::Zero
        } else {
            CoreClass::I
        },
        identity: SealIdentity::doctrine(),
        profiles: Vec::new(),
        fallback: None,
    }
}
fn core(class: RatingClass) -> CoreClass {
    match class {
        RatingClass::I => CoreClass::I,
        RatingClass::II => CoreClass::II,
        RatingClass::III => CoreClass::III,
    }
}
fn id(word: &str) -> Result<CanonicalId, RatingError> {
    CanonicalId::new(word).map_err(|_| RatingError::Invalid)
}
pub(super) fn drain(
    arena: &mut Arena,
    chunks: &[crate::arena::ledger::LedgerChunk],
) -> Result<(), RatingError> {
    arena.ledger.flush();
    let tail = arena.ledger.take_chunks();
    if let Some(last) = chunks.iter().chain(&tail).map(|c| c.seq).max() {
        arena.ledger.mark_flushed(last);
    }
    if arena.ledger.overflowed() {
        return Err(RatingError::Bound);
    }
    Ok(())
}
fn outcome(reason: EndReason, swapped: bool) -> Result<TrialOutcome, RatingError> {
    let focus = if swapped { PartyId::BLUE } else { PartyId::RED };
    let winner = match reason {
        EndReason::Elimination { winner } => winner,
        EndReason::Hold { winner } => Some(winner),
        EndReason::Conceded { winner } => winner,
        EndReason::Spent => None,
        EndReason::Timeout | EndReason::Unattended => return Ok(TrialOutcome::Timeout),
        EndReason::Abort | EndReason::Void | EndReason::LedgerOverflow => {
            return Err(RatingError::Invalid);
        }
    };
    Ok(match winner {
        Some(team) if team == focus => TrialOutcome::Win,
        Some(_) => TrialOutcome::Loss,
        None => TrialOutcome::Draw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    fn case(policy: &FrozenPolicy, class: RatingClass, variant: u8) -> Result<Case, RatingError> {
        Ok(Case {
            key: mush_roll::commerce::rating::CaseKey::new(
                class,
                CaseKind::Trial {
                    variant,
                    layout: TrialLayout::Duel,
                    seed: 1,
                    swapped: false,
                },
            )?,
            physics: Trial::physics(policy, TrialLayout::Duel, core(class))?,
            maximum_ms: super::super::plan::TRIAL_LIMIT_MS,
            maximum_thinks: 2402,
        })
    }
    #[test]
    fn higher_class_trial_preserves_declared_card_and_lands_actual_source() -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { stock::rules(); this.calls = if this.contains(\"calls\") { this.calls+1 } else { 1 }; }";
        let revision = super::super::tests::revision(source)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        let case = case(&policy, RatingClass::II, 0)?;
        let mut trial = Trial::open(&policy, &subject, &case)?;
        let id = *trial.candidates.first().ok_or("candidate")?;
        let runtime = trial.arena.profiles.get(&id).ok_or("runtime")?;
        let original = subject.variants().first().ok_or("variant")?;
        check_eq!(original.definition.declared_class, CoreClass::I);
        check_eq!(
            trial.arena.unit(id).ok_or("hull")?.seal.class,
            CoreClass::II
        );
        check_eq!(
            runtime.card.rules(),
            Some(
                original
                    .definition
                    .profiles
                    .first()
                    .ok_or("profile")?
                    .rules
                    .as_slice()
            )
        );
        check!(!runtime.program.rows.is_empty());
        let mut landed = false;
        for _ in 0..80 {
            let slice = trial.advance(u64::from(case.physics.native_ms))?;
            check!(slice.simulated_ms <= u64::from(case.physics.native_ms));
            check!(
                slice.result.is_none(),
                "short real run has no invented completion"
            );
            let state = trial.arena.minds.get(&id).ok_or("mind")?;
            if !state.memory.is_empty() {
                let memory: serde_json::Value = serde_json::from_slice(&state.memory)?;
                if memory
                    .get("calls")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|n| n >= 3)
                {
                    check_eq!(state.strikes, 0);
                    check!(state.dropped.is_none());
                    landed = true;
                    break;
                }
            }
        }
        check!(
            landed,
            "real pending answers reach native landing with fresh evolving memory"
        );
        let measured = trial.meter.measurement()?;
        check!(measured.summary.clean());
        check!(measured.summary.max_ops() > 0 && measured.summary.max_fees() > 0);
        check_eq!(trial.binding.source(), revision.source());
        Ok(())
    }
    #[test]
    fn landing_evidence_counts_the_actual_zero_sequence_and_refuses_a_gap() -> TestResult {
        use crate::arena::{intents::Refusal, mind::Landed};
        let own = ObjId(1);
        let mut traces = TraceStore::default();
        check!(traces.record(
            own,
            0,
            0,
            false,
            None,
            &[],
            &[IntentKind::Lock {
                contact: crate::arena::ContactId(99)
            }],
            None
        ));
        check!(traces.land(own, 0, 250, &[Landed::Refused(Refusal::NoContact)]));
        let mut landings = Landings::new(&[own]);
        landings.collect(&traces)?;
        check_eq!(landings.refusals(), 1);
        landings.collect(&traces)?;
        check_eq!(
            landings.refusals(),
            1,
            "the same actual row is not counted twice"
        );
        let mut missing = TraceStore::default();
        check!(missing.record(own, 1, 1000, false, None, &[], &[], None));
        check!(missing.land(own, 1, 1250, &[]));
        check!(
            Landings::new(&[own]).collect(&missing).is_err(),
            "missing first invocation cannot be silent"
        );
        Ok(())
    }
    #[test]
    fn actual_operations_exhaustion_finishes_only_a_budget_failure_row() -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { while true {} }";
        let revision = super::super::tests::revision(source)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        let case = case(&policy, RatingClass::I, 0)?;
        let mut trial = Trial::open(&policy, &subject, &case)?;
        let mut result = None;
        for _ in 0..80 {
            result = trial.advance(u64::from(case.physics.native_ms))?.result;
            if result.is_some() {
                break;
            }
        }
        let result = result.ok_or("the bounded real invocation must exhaust its budget")?;
        check!(matches!(result.outcome(), CaseOutcome::BudgetExceeded {}));
        check!(result.meter().only_budget_faults());
        check_eq!(result.meter().thinks(), 1);
        check_eq!(result.binding().source(), revision.source());
        Ok(())
    }
}

#[cfg(test)]
mod house_tests;
#[cfg(test)]
mod physical_tests;
