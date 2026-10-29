//! The in-character (IC) game clock.
//!
//! Pure arithmetic over a caller-supplied wall-clock timestamp — core logic
//! never calls `SystemTime`. The IC calendar is a fictional 365-day year
//! (no leap days) counted from the IC epoch, which is all the weather and
//! ruleset systems need: stable seasons and times of day, configurable
//! speed ([`GameClock::ic_ratio`]) and phase ([`GameClock::epoch_offset`]).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::mechanics::{CanonicalId, OwnerId};
use crate::module::{ModuleOp, ModuleWrite};

/// Seconds per IC day.
pub const SECS_PER_DAY: i64 = 86_400;

/// Seconds per IC hour.
pub const SECS_PER_HOUR: i64 = 3_600;

/// Days per IC year (fictional calendar; no leap days).
pub const DAYS_PER_YEAR: i64 = 365;

/// IC month lengths, index 0 == January. Gregorian-style with no leap day
/// (February is always 28); the twelve sum to exactly [`DAYS_PER_YEAR`], so
/// the calendar stays a clean, familiar 365-day year.
pub const MONTH_LENGTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// IC month names, index 0 == January.
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Maximum number of world-IC epochs accepted at boot.
///
/// Rate changes are expected to be rare campaign operations.  Keeping a hard
/// limit makes corrupt or hostile persisted state bounded without requiring
/// old generations to be discarded (old [`ClockStamp`] values retain their
/// meaning forever).
pub const MAX_WORLD_TIMELINE_EPOCHS: usize = 4_096;

/// The slowest world-IC rate any caller may install: **one** IC second per real
/// second (10b §7; E7 residue pass 1 independent verify fold, C9/B10; owner decision
/// F9(a) with the band `1..=3_600`).
///
/// **Zero is not a rate, it is a stopped clock**, and nothing here checked for it.
/// `WorldTimelineEpoch::ic_at` computes `ic_anchor + delta * rate`, so at zero every
/// `world_ic_now` consumer stops for ever — recovery cooldowns, production schedules,
/// journey ETAs, escalation cool-offs — while `game_clock_compatibility` stores a
/// `GameClock { ic_ratio: 0 }` whose only two readers (`mush_core::module`'s
/// `AMBIENT_IC_TIME_KEY` and `mush_economy::sweep`'s `ratio`) both apply `.max(1)`.
/// So the IC clock and calendar a player reads in the room kept advancing at 1:1
/// against a world in which nothing keyed on `world_ic_now` would ever come due
/// again, and a later rate change re-anchored from the frozen `ic_at`, discarding
/// every elapsed second as a jump. Boot was already protected —
/// [`WorldTimeline::from_game_clock`] clamps `clock.ic_ratio.max(1)` — and the
/// mutation path was not.
///
/// A deliberate freeze, if one is ever wanted, needs its own verb with explicit
/// semantics on **both** clock readers, never a rate of zero typed by accident.
pub const MIN_WORLD_TIME_RATE: u32 = 1;

/// The fastest world-IC rate any caller may install: one IC **hour** per real second
/// (F9(a)'s band).
///
/// Doc 10 S1A's *"≈ one OOC hour per IC day"* is 24 IC seconds per real second, so
/// the band is three orders of magnitude of headroom over the setting's own pace and
/// still bounded: `u32::MAX` was accepted before, and at that rate a single real
/// second advances the IC calendar by 136 years.
pub const MAX_WORLD_TIME_RATE: u32 = 3_600;

/// Persistence namespace reserved for engine-owned clock state.
pub const WORLD_TIMELINE_NAMESPACE: &str = "rustmush_kernel_time";

/// Persistence key containing the current world-IC generation.
pub const WORLD_TIMELINE_HEAD_KEY: &str = "world_ic/head";

/// Prefix for immutable per-generation world-IC epoch documents.
pub const WORLD_TIMELINE_EPOCH_PREFIX: &str = "world_ic/epoch/";

/// Persistence key containing mutable world-IC checkpoint, re-anchor, and freeze state (`W-clock`).
pub const WORLD_TIMELINE_STATE_KEY: &str = "world_ic/state";

/// A durable checkpoint of the world IC clock at a specific wall-clock instant (`W-clock`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldClockCheckpoint {
    /// Wall-clock Unix second when the checkpoint was recorded.
    pub real_unix: i64,
    /// World-IC second at `real_unix`.
    pub ic_unix: i64,
    /// Timeline generation active at `real_unix`.
    pub generation: u64,
}

/// Re-anchored `(real_anchor, ic_anchor)` for a specific timeline generation after a
/// freeze/resume or sub-threshold restart re-anchor (`W-clock`, `Q-R4`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldClockReanchor {
    /// Generation whose anchor was re-anchored.
    pub generation: u64,
    /// Re-anchored wall-clock Unix second.
    pub real_anchor: i64,
    /// Re-anchored world-IC second.
    pub ic_anchor: i64,
}

/// Why the world IC clock is currently frozen (`W-clock`, `FO-11`, `Q-R4`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorldClockFreezeKind {
    /// Deliberate staff/campaign freeze (`@tacsim/time freeze`).
    Deliberate,
    /// Automatic outage recovery countdown (`FO-11`, `Q-R4`).
    OutageCountdown {
        /// Wall-clock Unix second when the current recovery countdown started.
        countdown_started_real_unix: i64,
        /// Wall-clock Unix second when the world IC clock will unfreeze.
        unfreeze_at_real_unix: i64,
    },
}

/// Active freeze state on the shared world IC clock (`W-clock`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldClockFreeze {
    /// Wall-clock Unix second when the world became unavailable / frozen (checkpoint time
    /// for an outage freeze, or command time for a deliberate freeze).
    pub frozen_real_unix: i64,
    /// Frozen world-IC second held constant until resume.
    pub frozen_ic_unix: i64,
    /// Timeline generation held while frozen.
    pub generation: u64,
    /// Cause and countdown metadata for the freeze.
    pub kind: WorldClockFreezeKind,
}

const fn default_timeline_state_schema() -> u16 {
    WorldTimelineState::SCHEMA_VERSION
}

/// Persisted mutable state paired with the world-IC epoch log under [`WORLD_TIMELINE_STATE_KEY`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldTimelineState {
    /// Persistence schema version.
    #[serde(default = "default_timeline_state_schema")]
    pub schema_version: u16,
    /// Per-generation re-anchors, ordered by generation.
    #[serde(default)]
    pub reanchors: Vec<WorldClockReanchor>,
    /// Most recent durable checkpoint of `(real_unix, ic_unix, generation)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<WorldClockCheckpoint>,
    /// Active freeze state, if the world clock is currently frozen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze: Option<WorldClockFreeze>,
}

impl Default for WorldTimelineState {
    fn default() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            reanchors: Vec::new(),
            checkpoint: None,
            freeze: None,
        }
    }
}

impl WorldTimelineState {
    /// Current persistence schema for [`WORLD_TIMELINE_STATE_KEY`].
    pub const SCHEMA_VERSION: u16 = 1;

    /// Whether this state carries no re-anchors, checkpoint, or freeze.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reanchors.is_empty() && self.checkpoint.is_none() && self.freeze.is_none()
    }
}

/// Identity of a clock.  A generation is part of every mutable clock's
/// identity, so an old deadline can never silently be interpreted under a
/// later rate, scene, domain, or provider definition.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClockRef {
    /// Unix UTC seconds.  UTC has no mutable generation.
    Utc,
    /// The versioned world in-character timeline.
    WorldIc {
        /// Exact timeline generation.
        generation: u64,
    },
    /// A scene-local turn counter.
    SceneTurn {
        /// Stable scene identifier supplied by its provider.
        scene: u64,
        /// Exact scene-clock generation.
        generation: u64,
    },
    /// A tactical/spatial domain tick counter.  Core deliberately does not
    /// depend on a particular simulation crate's domain-id representation.
    DomainTick {
        /// Module that owns the domain clock.
        owner: OwnerId,
        /// Owner-local canonical domain identifier.
        id: CanonicalId,
        /// Exact domain-clock generation.
        generation: u64,
    },
    /// Any other registered module-owned clock.
    Registered {
        /// Clock provider.
        owner: OwnerId,
        /// Provider-local clock identifier.
        id: CanonicalId,
        /// Exact provider-clock generation.
        generation: u64,
    },
}

/// A scalar value on one exact typed clock generation.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockStamp {
    /// Complete clock identity, including generation where mutable.
    pub clock: ClockRef,
    /// Scalar position on that clock (seconds, turns, or ticks as defined by
    /// the clock provider).
    pub at: i64,
}

impl ClockStamp {
    /// Compare two stamps only when they belong to the same exact clock
    /// generation.
    pub fn checked_cmp(&self, other: &Self) -> Result<Ordering, ClockMismatch> {
        if self.clock != other.clock {
            return Err(ClockMismatch {
                left: Box::new(self.clock.clone()),
                right: Box::new(other.clock.clone()),
            });
        }
        Ok(self.at.cmp(&other.at))
    }

    /// Checked distance on one exact clock generation.
    pub fn checked_duration_since(&self, earlier: &Self) -> Result<i64, ClockArithmeticError> {
        self.checked_cmp(earlier)?;
        self.at
            .checked_sub(earlier.at)
            .ok_or(ClockArithmeticError::Overflow)
    }
}

/// Two scalar clock values were compared without a shared identity.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("clock stamps are incomparable: {left:?} versus {right:?}")]
pub struct ClockMismatch {
    /// Left clock identity.
    pub left: Box<ClockRef>,
    /// Right clock identity.
    pub right: Box<ClockRef>,
}

/// Failure while comparing or subtracting typed clock values.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ClockArithmeticError {
    /// The clocks or generations differ.
    #[error(transparent)]
    Mismatch(#[from] ClockMismatch),
    /// The scalar difference is outside `i64`.
    #[error("clock arithmetic overflow")]
    Overflow,
}

/// One immutable generation of the world in-character timeline.
///
/// `rate` is IC seconds per real second.  Zero deliberately represents a
/// paused world clock.  Opening a new generation anchors it to the exact IC
/// value produced by the preceding generation at `real_anchor`, so a rate
/// change is continuous while earlier stamps keep their old identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldTimelineEpoch {
    /// Wall-clock Unix second at which this generation begins.
    pub real_anchor: i64,
    /// World-IC second at the anchor.
    pub ic_anchor: i64,
    /// IC seconds advanced per wall-clock second (`0` pauses).
    pub rate: u32,
    /// Monotonic generation, beginning at one.
    pub generation: u64,
}

impl WorldTimelineEpoch {
    /// Evaluate this epoch at a wall-clock Unix second with saturating
    /// arithmetic.  Callers normally evaluate only the current generation at
    /// or after its anchor; supporting earlier values makes continuity checks
    /// exact and deterministic.
    #[must_use]
    pub fn ic_at(self, real_unix: i64) -> i64 {
        let delta = i128::from(real_unix) - i128::from(self.real_anchor);
        let value = i128::from(self.ic_anchor) + delta * i128::from(self.rate);
        value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
}

/// The append-only world-IC epoch log and mutable checkpoint/freeze state held in RAM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldTimeline {
    epochs: Vec<WorldTimelineEpoch>,
    reanchors: std::collections::BTreeMap<u64, WorldClockReanchor>,
    checkpoint: Option<WorldClockCheckpoint>,
    freeze: Option<WorldClockFreeze>,
}

impl Default for WorldTimeline {
    fn default() -> Self {
        Self::from_game_clock(GameClock::default())
    }
}

impl WorldTimeline {
    /// Seed generation one from the legacy boot configuration without
    /// changing its `ic = offset + real * ratio` behavior.
    #[must_use]
    pub fn from_game_clock(clock: GameClock) -> Self {
        Self {
            epochs: vec![WorldTimelineEpoch {
                real_anchor: 0,
                ic_anchor: clock.epoch_offset,
                rate: clock.ic_ratio.max(1),
                generation: 1,
            }],
            reanchors: std::collections::BTreeMap::new(),
            checkpoint: None,
            freeze: None,
        }
    }

    /// Validate and construct a persisted contiguous epoch log.
    pub fn from_epochs(epochs: Vec<WorldTimelineEpoch>) -> Result<Self, WorldTimelineError> {
        Self::from_epochs_and_state(epochs, WorldTimelineState::default())
    }

    /// Validate and construct a persisted contiguous epoch log paired with mutable
    /// checkpoint, re-anchor, and freeze state (`W-clock`).
    pub fn from_epochs_and_state(
        epochs: Vec<WorldTimelineEpoch>,
        state: WorldTimelineState,
    ) -> Result<Self, WorldTimelineError> {
        if epochs.is_empty() {
            return Err(WorldTimelineError::Empty);
        }
        if epochs.len() > MAX_WORLD_TIMELINE_EPOCHS {
            return Err(WorldTimelineError::TooMany {
                actual: epochs.len(),
                max: MAX_WORLD_TIMELINE_EPOCHS,
            });
        }
        if state.schema_version != WorldTimelineState::SCHEMA_VERSION {
            return Err(WorldTimelineError::UnsupportedStateSchema(
                state.schema_version,
            ));
        }
        let max_gen = u64::try_from(epochs.len()).unwrap_or(u64::MAX);
        let mut reanchors = std::collections::BTreeMap::new();
        for reanchor in state.reanchors {
            if reanchor.generation == 0 || reanchor.generation > max_gen {
                return Err(WorldTimelineError::NonContiguousGeneration {
                    expected: max_gen,
                    actual: reanchor.generation,
                });
            }
            if reanchors.insert(reanchor.generation, reanchor).is_some() {
                return Err(WorldTimelineError::DuplicateReanchorGeneration(
                    reanchor.generation,
                ));
            }
        }
        let effective = |epoch: WorldTimelineEpoch| -> WorldTimelineEpoch {
            match reanchors.get(&epoch.generation) {
                Some(reanchor) => WorldTimelineEpoch {
                    real_anchor: reanchor.real_anchor,
                    ic_anchor: reanchor.ic_anchor,
                    rate: epoch.rate,
                    generation: epoch.generation,
                },
                None => epoch,
            }
        };
        for (index, epoch) in epochs.iter().copied().enumerate() {
            let expected_generation = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(WorldTimelineError::GenerationExhausted)?;
            if epoch.generation != expected_generation {
                return Err(WorldTimelineError::NonContiguousGeneration {
                    expected: expected_generation,
                    actual: epoch.generation,
                });
            }
            if let Some(reanchor) = reanchors.get(&epoch.generation)
                && reanchor.real_anchor < epoch.real_anchor
            {
                return Err(WorldTimelineError::AnchorMovedBackward {
                    previous: epoch.real_anchor,
                    proposed: reanchor.real_anchor,
                });
            }
            if let Some(previous) = index.checked_sub(1).and_then(|i| epochs.get(i).copied()) {
                validate_epoch_transition(effective(previous), epoch)?;
            }
        }
        if let Some(cp) = state.checkpoint
            && (cp.generation == 0 || cp.generation > max_gen)
        {
            return Err(WorldTimelineError::NonContiguousGeneration {
                expected: max_gen,
                actual: cp.generation,
            });
        }
        if let Some(freeze) = state.freeze
            && freeze.generation != max_gen
        {
            return Err(WorldTimelineError::NonContiguousGeneration {
                expected: max_gen,
                actual: freeze.generation,
            });
        }
        Ok(Self {
            epochs,
            reanchors,
            checkpoint: state.checkpoint,
            freeze: state.freeze,
        })
    }

    /// All immutable epochs, oldest first.
    #[must_use]
    pub fn epochs(&self) -> &[WorldTimelineEpoch] {
        &self.epochs
    }

    /// Current epoch (including any checkpoint/unfreeze re-anchor on the head generation).
    /// A timeline is structurally non-empty.
    #[must_use]
    pub fn current(&self) -> WorldTimelineEpoch {
        let base = match self.epochs.last().copied() {
            Some(epoch) => epoch,
            None => WorldTimelineEpoch {
                real_anchor: 0,
                ic_anchor: 0,
                rate: 1,
                generation: 1,
            },
        };
        match self.reanchors.get(&base.generation) {
            Some(reanchor) => WorldTimelineEpoch {
                real_anchor: reanchor.real_anchor,
                ic_anchor: reanchor.ic_anchor,
                rate: base.rate,
                generation: base.generation,
            },
            None => base,
        }
    }

    /// Whether the world IC clock is currently frozen (`W-clock`).
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.freeze.is_some()
    }

    /// Active freeze state, if the world IC clock is frozen.
    #[must_use]
    pub fn freeze(&self) -> Option<WorldClockFreeze> {
        self.freeze
    }

    /// Most recent durable checkpoint on the timeline.
    #[must_use]
    pub fn checkpoint(&self) -> Option<WorldClockCheckpoint> {
        self.checkpoint
    }

    /// Snapshot of the mutable state persisted at [`WORLD_TIMELINE_STATE_KEY`].
    #[must_use]
    pub fn state(&self) -> WorldTimelineState {
        WorldTimelineState {
            schema_version: WorldTimelineState::SCHEMA_VERSION,
            reanchors: self.reanchors.values().copied().collect(),
            checkpoint: self.checkpoint,
            freeze: self.freeze,
        }
    }

    /// Current world-IC value and generation at `real_unix`.
    ///
    /// While frozen (`W-clock`), every evaluation returns the exact frozen IC
    /// timestamp on the current generation without advancing.
    #[must_use]
    pub fn stamp_at(&self, real_unix: i64) -> ClockStamp {
        let current = self.current();
        let at = match self.freeze {
            Some(freeze) => freeze.frozen_ic_unix,
            None => current.ic_at(real_unix),
        };
        ClockStamp {
            clock: ClockRef::WorldIc {
                generation: current.generation,
            },
            at,
        }
    }

    /// Record a durable checkpoint at `real_unix`.
    pub fn record_checkpoint(&mut self, real_unix: i64) -> WorldClockCheckpoint {
        let stamp = self.stamp_at(real_unix);
        let current = self.current();
        let checkpoint = WorldClockCheckpoint {
            real_unix,
            ic_unix: stamp.at,
            generation: current.generation,
        };
        self.checkpoint = Some(checkpoint);
        checkpoint
    }

    /// Deliberately freeze the shared world IC clock at `real_unix` (`W-clock`).
    pub fn freeze_deliberate(
        &mut self,
        real_unix: i64,
    ) -> Result<WorldClockFreeze, WorldTimelineError> {
        if self.freeze.is_some() {
            return Err(WorldTimelineError::AlreadyFrozen);
        }
        let current = self.current();
        if real_unix < current.real_anchor {
            return Err(WorldTimelineError::AnchorMovedBackward {
                previous: current.real_anchor,
                proposed: real_unix,
            });
        }
        let frozen_ic_unix = current.ic_at(real_unix);
        let freeze = WorldClockFreeze {
            frozen_real_unix: real_unix,
            frozen_ic_unix,
            generation: current.generation,
            kind: WorldClockFreezeKind::Deliberate,
        };
        self.freeze = Some(freeze);
        self.checkpoint = Some(WorldClockCheckpoint {
            real_unix,
            ic_unix: frozen_ic_unix,
            generation: current.generation,
        });
        Ok(freeze)
    }

    /// Enter or refresh an unplanned-outage recovery countdown freeze (`FO-11`, `Q-R4`),
    /// holding the IC clock at `(frozen_real_unix, frozen_ic_unix)` until `unfreeze_at_real_unix`.
    pub fn freeze_for_outage(
        &mut self,
        frozen_real_unix: i64,
        frozen_ic_unix: i64,
        countdown_started_real_unix: i64,
        unfreeze_at_real_unix: i64,
    ) -> WorldClockFreeze {
        let generation = self.current().generation;
        let freeze = WorldClockFreeze {
            frozen_real_unix,
            frozen_ic_unix,
            generation,
            kind: WorldClockFreezeKind::OutageCountdown {
                countdown_started_real_unix,
                unfreeze_at_real_unix,
            },
        };
        self.freeze = Some(freeze);
        self.checkpoint = Some(WorldClockCheckpoint {
            real_unix: frozen_real_unix,
            ic_unix: frozen_ic_unix,
            generation,
        });
        freeze
    }

    /// Resume the frozen world IC clock at `real_unix`, re-anchoring the current
    /// generation so `world_ic_now` continues smoothly from `frozen_ic_unix` with
    /// zero unseen catch-up (`W-clock`).
    pub fn resume_at(
        &mut self,
        real_unix: i64,
    ) -> Result<(WorldClockFreeze, WorldTimelineEpoch), WorldTimelineError> {
        let Some(freeze) = self.freeze else {
            return Err(WorldTimelineError::NotFrozen);
        };
        if real_unix < freeze.frozen_real_unix {
            return Err(WorldTimelineError::AnchorMovedBackward {
                previous: freeze.frozen_real_unix,
                proposed: real_unix,
            });
        }
        self.freeze = None;
        self.reanchors.insert(
            freeze.generation,
            WorldClockReanchor {
                generation: freeze.generation,
                real_anchor: real_unix,
                ic_anchor: freeze.frozen_ic_unix,
            },
        );
        self.checkpoint = Some(WorldClockCheckpoint {
            real_unix,
            ic_unix: freeze.frozen_ic_unix,
            generation: freeze.generation,
        });
        Ok((freeze, self.current()))
    }

    /// Silently re-anchor the world IC clock from its last durable checkpoint at
    /// `real_unix` without a countdown and without catch-up (`Q-R4`).
    ///
    /// Returns the elapsed wall-clock seconds excluded (`Some(lost_secs)`) when a
    /// prior checkpoint existed, or `None` when seeding the initial checkpoint.
    pub fn reanchor_from_checkpoint(
        &mut self,
        real_unix: i64,
    ) -> Result<Option<i64>, WorldTimelineError> {
        if self.freeze.is_some() {
            return Err(WorldTimelineError::AlreadyFrozen);
        }
        let Some(cp) = self.checkpoint else {
            self.record_checkpoint(real_unix);
            return Ok(None);
        };
        if real_unix < cp.real_unix {
            return Err(WorldTimelineError::AnchorMovedBackward {
                previous: cp.real_unix,
                proposed: real_unix,
            });
        }
        let lost_secs = real_unix.saturating_sub(cp.real_unix);
        let generation = self.current().generation;
        self.reanchors.insert(
            generation,
            WorldClockReanchor {
                generation,
                real_anchor: real_unix,
                ic_anchor: cp.ic_unix,
            },
        );
        self.checkpoint = Some(WorldClockCheckpoint {
            real_unix,
            ic_unix: cp.ic_unix,
            generation,
        });
        Ok(Some(lost_secs))
    }

    /// Prepare a continuous rate change without mutating the log.
    pub fn next_rate_epoch(
        &self,
        real_anchor: i64,
        rate: u32,
    ) -> Result<WorldTimelineEpoch, WorldTimelineError> {
        if self.freeze.is_some() {
            return Err(WorldTimelineError::ClockFrozen);
        }
        let current = self.current();
        // **The band, in the kernel** (independent verify fold, C9/B10; owner decision
        // F9(a)): the verb was not the only caller, and a rate is not something a
        // caller may be trusted to have checked. `rate == 0` stops every
        // `world_ic_now` mechanism for ever; anything past the band is a campaign act
        // nobody meant to make. Refused, never clamped — a silently clamped time rate
        // is a decision the staffer did not take.
        if !(MIN_WORLD_TIME_RATE..=MAX_WORLD_TIME_RATE).contains(&rate) {
            return Err(WorldTimelineError::RateOutOfBand {
                rate,
                min: MIN_WORLD_TIME_RATE,
                max: MAX_WORLD_TIME_RATE,
            });
        }
        if real_anchor < current.real_anchor {
            return Err(WorldTimelineError::AnchorMovedBackward {
                previous: current.real_anchor,
                proposed: real_anchor,
            });
        }
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(WorldTimelineError::GenerationExhausted)?;
        if self.epochs.len() >= MAX_WORLD_TIMELINE_EPOCHS {
            return Err(WorldTimelineError::TooMany {
                actual: self.epochs.len() + 1,
                max: MAX_WORLD_TIMELINE_EPOCHS,
            });
        }
        Ok(WorldTimelineEpoch {
            real_anchor,
            ic_anchor: current.ic_at(real_anchor),
            rate,
            generation,
        })
    }

    /// Append one exact prepared epoch.  Re-validating the transition keeps
    /// persisted hydration and live mutation on the same invariant path.
    pub fn append(&mut self, epoch: WorldTimelineEpoch) -> Result<(), WorldTimelineError> {
        if self.freeze.is_some() {
            return Err(WorldTimelineError::ClockFrozen);
        }
        if self.epochs.len() >= MAX_WORLD_TIMELINE_EPOCHS {
            return Err(WorldTimelineError::TooMany {
                actual: self.epochs.len() + 1,
                max: MAX_WORLD_TIMELINE_EPOCHS,
            });
        }
        validate_epoch_transition(self.current(), epoch)?;
        self.epochs.push(epoch);
        if self.checkpoint.is_some() {
            self.checkpoint = Some(WorldClockCheckpoint {
                real_unix: epoch.real_anchor,
                ic_unix: epoch.ic_anchor,
                generation: epoch.generation,
            });
        }
        Ok(())
    }
}

fn validate_epoch_transition(
    previous: WorldTimelineEpoch,
    next: WorldTimelineEpoch,
) -> Result<(), WorldTimelineError> {
    let expected = previous
        .generation
        .checked_add(1)
        .ok_or(WorldTimelineError::GenerationExhausted)?;
    if next.generation != expected {
        return Err(WorldTimelineError::NonContiguousGeneration {
            expected,
            actual: next.generation,
        });
    }
    if next.real_anchor < previous.real_anchor {
        return Err(WorldTimelineError::AnchorMovedBackward {
            previous: previous.real_anchor,
            proposed: next.real_anchor,
        });
    }
    let expected_ic = previous.ic_at(next.real_anchor);
    if next.ic_anchor != expected_ic {
        return Err(WorldTimelineError::DiscontinuousAnchor {
            generation: next.generation,
            expected: expected_ic,
            actual: next.ic_anchor,
        });
    }
    Ok(())
}

/// Invalid world-IC epoch history or transition.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorldTimelineError {
    /// A timeline must contain generation one.
    #[error("world timeline is empty")]
    Empty,
    /// Persisted or proposed history exceeds the boot/work bound.
    #[error("world timeline contains {actual} epochs; maximum is {max}")]
    TooMany {
        /// Observed/proposed count.
        actual: usize,
        /// Hard limit.
        max: usize,
    },
    /// Generations must be exactly `1, 2, ...`.
    #[error("world timeline generation is not contiguous: expected {expected}, got {actual}")]
    NonContiguousGeneration {
        /// Required next generation.
        expected: u64,
        /// Supplied generation.
        actual: u64,
    },
    /// Rate changes cannot be installed before the current generation began.
    #[error("world timeline anchor moved backward from {previous} to {proposed}")]
    AnchorMovedBackward {
        /// Current epoch anchor.
        previous: i64,
        /// Proposed epoch anchor.
        proposed: i64,
    },
    /// A new rate generation must preserve the preceding IC value at its
    /// anchor.
    #[error(
        "world timeline generation {generation} is discontinuous: expected IC {expected}, got {actual}"
    )]
    DiscontinuousAnchor {
        /// Proposed generation.
        generation: u64,
        /// Value derived from the previous epoch.
        expected: i64,
        /// Supplied anchor value.
        actual: i64,
    },
    /// No further generation can be represented.
    #[error("world timeline generation exhausted")]
    GenerationExhausted,
    /// The proposed rate is outside [`MIN_WORLD_TIME_RATE`]..=[`MAX_WORLD_TIME_RATE`]
    /// (independent verify fold, C9/B10; owner decision F9(a)). A rate of **zero** is
    /// the load-bearing case: it stops every `world_ic_now` mechanism for ever while
    /// the compatibility `GameClock`'s two `.max(1)` readers keep the clock a player
    /// reads in the room advancing at 1:1.
    #[error("world time rate {rate} is outside the band {min}..={max}")]
    RateOutOfBand {
        /// The rate that was proposed.
        rate: u32,
        /// The slowest legal rate.
        min: u32,
        /// The fastest legal rate.
        max: u32,
    },
    /// The world IC clock is already frozen (`W-clock`).
    #[error("world timeline is already frozen")]
    AlreadyFrozen,
    /// The world IC clock is not frozen (`W-clock`).
    #[error("world timeline is not frozen")]
    NotFrozen,
    /// Rate changes are refused while the world IC clock is frozen (`W-clock`).
    #[error("world timeline is frozen; resume the clock before changing its rate")]
    ClockFrozen,
    /// Persisted state schema version is unsupported.
    #[error("unsupported world timeline state schema {0}")]
    UnsupportedStateSchema(u16),
    /// Persisted state contained duplicate re-anchor entries for one generation.
    #[error("duplicate world timeline re-anchor for generation {0}")]
    DuplicateReanchorGeneration(u64),
}

/// Small materialized pointer paired atomically with every immutable epoch
/// document.  Boot verifies that it names the last contiguous generation; it
/// is an index, never a substitute for the epoch log.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldTimelineHead {
    schema_version: u16,
    generation: u64,
}

impl WorldTimelineHead {
    /// Current persistence schema.
    pub const SCHEMA_VERSION: u16 = 1;

    /// Build a head for an installed epoch.
    #[must_use]
    pub const fn new(generation: u64) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            generation,
        }
    }

    /// Validate and return the named generation.
    pub fn validated_generation(self) -> Result<u64, WorldTimelinePersistenceError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(WorldTimelinePersistenceError::UnsupportedHeadSchema(
                self.schema_version,
            ));
        }
        if self.generation == 0 {
            return Err(WorldTimelinePersistenceError::ZeroHeadGeneration);
        }
        Ok(self.generation)
    }
}

/// Canonical immutable persistence key for one timeline generation.
#[must_use]
pub fn world_timeline_epoch_key(generation: u64) -> SmolStr {
    SmolStr::new(format!("{WORLD_TIMELINE_EPOCH_PREFIX}{generation:020}"))
}

/// Exact two-document append representation for a world-time generation.
///
/// Per-generation keys are never reused by the kernel.  Retrying an ambiguous
/// legacy-store acknowledgement writes the same bytes to the same two keys,
/// while the store transaction keeps epoch and head atomic.  Boot validates
/// the whole contiguous log before exposing it to modules.
pub fn world_timeline_persistence_ops(
    epoch: WorldTimelineEpoch,
) -> Result<[ModuleOp; 2], WorldTimelinePersistenceError> {
    let epoch_value = serde_json::to_value(epoch)?;
    let head_value = serde_json::to_value(WorldTimelineHead::new(epoch.generation))?;
    Ok([
        ModuleOp {
            ns: SmolStr::new_static(WORLD_TIMELINE_NAMESPACE),
            op: ModuleWrite::JsonSet {
                key: world_timeline_epoch_key(epoch.generation),
                value: epoch_value,
            },
        },
        ModuleOp {
            ns: SmolStr::new_static(WORLD_TIMELINE_NAMESPACE),
            op: ModuleWrite::JsonSet {
                key: SmolStr::new_static(WORLD_TIMELINE_HEAD_KEY),
                value: head_value,
            },
        },
    ])
}

/// Persistence operation for the mutable checkpoint, re-anchor, and freeze state
/// under [`WORLD_TIMELINE_STATE_KEY`] (`W-clock`).
pub fn world_timeline_state_op(
    timeline: &WorldTimeline,
) -> Result<ModuleOp, WorldTimelinePersistenceError> {
    let value = serde_json::to_value(timeline.state())?;
    Ok(ModuleOp {
        ns: SmolStr::new_static(WORLD_TIMELINE_NAMESPACE),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new_static(WORLD_TIMELINE_STATE_KEY),
            value,
        },
    })
}

/// Corrupt or unrepresentable timeline persistence metadata.
#[derive(Debug, thiserror::Error)]
pub enum WorldTimelinePersistenceError {
    /// Epoch/head serialization failed before any mutation.
    #[error("world timeline serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    /// A head document belongs to a future incompatible schema.
    #[error("unsupported world timeline head schema {0}")]
    UnsupportedHeadSchema(u16),
    /// Generation zero is reserved for "no persisted timeline".
    #[error("world timeline head generation must be positive")]
    ZeroHeadGeneration,
}

impl crate::cmd::Ctx<'_> {
    /// Typed world-IC `now`, available to commands, hooks, ticks, observers,
    /// and module boot callbacks through the ordinary module context.
    #[must_use]
    pub fn world_ic_now(&self) -> ClockStamp {
        self.w.world_ic_now()
    }

    /// Open and persist a continuous world-IC rate generation.
    ///
    /// This is mechanism, not policy: the campaign/staff layer decides who may
    /// invoke it.  The handler should perform every other fallible validation
    /// first and return success after this call.  Confirmation is held behind
    /// the durable write acknowledgement; a store failure freezes mutations
    /// under the existing persistence-integrity policy.
    pub fn change_world_time_rate(
        &mut self,
        rate: u32,
    ) -> Result<WorldTimelineEpoch, WorldTimelineChangeError> {
        let epoch = self.w.next_world_timeline_epoch(rate)?;
        let ops = world_timeline_persistence_ops(epoch)?;
        for op in ops {
            // The world timeline's namespace is engine-owned — reserved, and
            // refused as a module name at composition — so this is the engine
            // writing through whichever module's command asked, not that
            // module reaching into someone else's namespace (WS03-A1).
            self.persist_engine_owned(op);
        }
        self.gate_on_durable_ack_strict();
        self.w.append_world_timeline_epoch(epoch)?;
        if !self.w.world_timeline().state().is_empty() {
            let state_op = world_timeline_state_op(self.w.world_timeline())?;
            self.persist_engine_owned(state_op);
        }
        Ok(epoch)
    }

    /// Deliberately freeze the shared world-IC clock at the current wall-clock
    /// second (`W-clock`).
    ///
    /// Holds both `world_ic_now` and the compatibility `GameClock` readers at the
    /// frozen IC timestamp until [`Ctx::resume_world_time`] is invoked, and emits
    /// [`crate::module::WorldEvent::WorldClockFrozen`] so real-time deadline owners
    /// pause their windows (`Q-F3`, `P-4`).
    pub fn freeze_world_time(&mut self) -> Result<WorldClockFreeze, WorldTimelineChangeError> {
        let mut next = self.w.world_timeline().clone();
        let freeze = next.freeze_deliberate(self.w.now())?;
        let state_op = world_timeline_state_op(&next)?;
        self.persist_engine_owned(state_op);
        self.gate_on_durable_ack_strict();
        self.w.set_world_timeline(next);
        self.emit_world_event(&crate::module::WorldEvent::WorldClockFrozen {
            real_unix: freeze.frozen_real_unix,
            ic_unix: freeze.frozen_ic_unix,
        });
        Ok(freeze)
    }

    /// Resume the shared world-IC clock from its frozen checkpoint at the current
    /// wall-clock second (`W-clock`).
    ///
    /// Re-anchors the current timeline generation so `world_ic_now` and `GameClock`
    /// resume smoothly from the frozen IC timestamp with zero unseen catch-up, and
    /// emits [`crate::module::WorldEvent::WorldClockResumed`] so real-time deadline
    /// owners extend their deadlines by the unavailable duration (`Q-F3`, `P-4`).
    pub fn resume_world_time(
        &mut self,
    ) -> Result<(WorldClockFreeze, WorldTimelineEpoch), WorldTimelineChangeError> {
        let mut next = self.w.world_timeline().clone();
        let now = self.w.now();
        let (freeze, epoch) = next.resume_at(now)?;
        let state_op = world_timeline_state_op(&next)?;
        self.persist_engine_owned(state_op);
        self.gate_on_durable_ack_strict();
        self.w.set_world_timeline(next);
        let unavailable_secs = now.saturating_sub(freeze.frozen_real_unix);
        self.emit_world_event(&crate::module::WorldEvent::WorldClockResumed {
            frozen_real_unix: freeze.frozen_real_unix,
            resumed_real_unix: now,
            unavailable_secs,
            ic_unix: epoch.ic_anchor,
        });
        Ok((freeze, epoch))
    }

    /// Record and persist a durable world-IC clock checkpoint at the current
    /// wall-clock second (`W-clock` / `Q-R4`).
    pub fn checkpoint_world_time(
        &mut self,
    ) -> Result<WorldClockCheckpoint, WorldTimelineChangeError> {
        let mut next = self.w.world_timeline().clone();
        let cp = next.record_checkpoint(self.w.now());
        let state_op = world_timeline_state_op(&next)?;
        self.persist_engine_owned(state_op);
        self.w.set_world_timeline(next);
        Ok(cp)
    }
}

/// Failure to prepare or queue a live world-time rate generation.
#[derive(Debug, thiserror::Error)]
pub enum WorldTimelineChangeError {
    /// Invalid transition or exhausted history.
    #[error(transparent)]
    Timeline(#[from] WorldTimelineError),
    /// Epoch/head encoding failed before live installation.
    #[error(transparent)]
    Persistence(#[from] WorldTimelinePersistenceError),
}

/// Converts real (OOC) unix time into IC time and derives season and
/// time-of-day from it.
///
/// `ic = epoch_offset + now * ic_ratio` — an `ic_ratio` of 4 makes a full
/// IC day pass in 6 real hours; `epoch_offset` (IC seconds) shifts the
/// calendar so a game can open in whatever season it likes.
///
/// ```
/// use mush_core::clock::{GameClock, Season};
/// let clock = GameClock { ic_ratio: 1, epoch_offset: 200 * 86_400 };
/// assert_eq!(clock.season(0), Season::Summer); // IC day 200
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameClock {
    /// How many IC seconds pass per real second (0 is treated as 1).
    pub ic_ratio: u32,
    /// IC seconds added after scaling; sets the IC calendar's phase.
    pub epoch_offset: i64,
}

impl Default for GameClock {
    fn default() -> Self {
        GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        }
    }
}

/// A decomposed IC civil date-time on the fictional 365-day calendar.
///
/// Round-trips with [`CivilDate::from_ic_seconds`] /
/// [`CivilDate::to_ic_seconds`], and [`CivilDate::parse`] reads the config
/// `ic_epoch` string into one. `month` and `day` are 1-based; a negative
/// `year` is a date before IC timestamp 0. Its [`Display`](std::fmt::Display)
/// is the human-facing form the `time` command shows
/// (`"August 28, 2075, 02:57"`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CivilDate {
    /// IC year (year 0 begins at IC timestamp 0; negative before it).
    pub year: i64,
    /// Month of year, `1..=12`.
    pub month: u32,
    /// Day of month, `1..=` the month's length.
    pub day: u32,
    /// Hour of day, `0..24`.
    pub hour: u32,
    /// Minute of hour, `0..60`.
    pub minute: u32,
    /// Second of minute, `0..60`.
    pub second: u32,
}

impl CivilDate {
    /// Decompose an IC unix-style timestamp into civil fields.
    #[must_use]
    pub fn from_ic_seconds(ic: i64) -> CivilDate {
        let day_secs = ic.rem_euclid(SECS_PER_DAY);
        let total_days = ic.div_euclid(SECS_PER_DAY);
        let year = total_days.div_euclid(DAYS_PER_YEAR);
        let mut doy = total_days.rem_euclid(DAYS_PER_YEAR) as u32;
        let mut month = 12u32;
        for (i, &len) in MONTH_LENGTHS.iter().enumerate() {
            if doy < len {
                month = i as u32 + 1;
                break;
            }
            doy -= len;
        }
        CivilDate {
            year,
            month,
            day: doy + 1,
            hour: (day_secs / SECS_PER_HOUR) as u32,
            minute: ((day_secs % SECS_PER_HOUR) / 60) as u32,
            second: (day_secs % 60) as u32,
        }
    }

    /// Compose civil fields back into an IC unix-style timestamp. Out-of-range
    /// `month`/`day` are clamped into the calendar so a config typo still
    /// boots (rather than panicking on an index).
    #[must_use]
    pub fn to_ic_seconds(&self) -> i64 {
        let month = self.month.clamp(1, 12) as usize;
        let mut doy: i64 = MONTH_LENGTHS[..month - 1]
            .iter()
            .map(|&l| i64::from(l))
            .sum();
        let day = self.day.clamp(1, MONTH_LENGTHS[month - 1]);
        doy += i64::from(day - 1);
        let total_days = self.year.saturating_mul(DAYS_PER_YEAR).saturating_add(doy);
        total_days.saturating_mul(SECS_PER_DAY)
            + i64::from(self.hour) * SECS_PER_HOUR
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    /// Parse an `ic_epoch` config value: `"YYYY-MM-DD"` or
    /// `"YYYY-MM-DD HH:MM[:SS]"` (a `T` may separate the date and time, as in
    /// RFC 3339). The year must be non-negative; every field is
    /// range-checked. Returns `None` on any malformed input.
    #[must_use]
    pub fn parse(s: &str) -> Option<CivilDate> {
        let s = s.trim();
        let (date, time) = match s.split_once([' ', 'T']) {
            Some((d, t)) => (d, Some(t.trim())),
            None => (s, None),
        };
        let mut dp = date.split('-');
        let year: i64 = dp.next()?.trim().parse().ok()?;
        let month: u32 = dp.next()?.trim().parse().ok()?;
        let day: u32 = dp.next()?.trim().parse().ok()?;
        if dp.next().is_some() || year < 0 || !(1..=12).contains(&month) {
            return None;
        }
        if day < 1 || day > MONTH_LENGTHS[month as usize - 1] {
            return None;
        }
        let (mut hour, mut minute, mut second) = (0u32, 0u32, 0u32);
        if let Some(t) = time.filter(|t| !t.is_empty()) {
            let mut tp = t.split(':');
            hour = tp.next()?.trim().parse().ok()?;
            minute = tp.next()?.trim().parse().ok()?;
            if let Some(sec) = tp.next() {
                second = sec.trim().parse().ok()?;
            }
            if tp.next().is_some() || hour >= 24 || minute >= 60 || second >= 60 {
                return None;
            }
        }
        Some(CivilDate {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// This date's month name (`"January"`..`"December"`).
    #[must_use]
    pub fn month_name(&self) -> &'static str {
        MONTH_NAMES[(self.month.clamp(1, 12) - 1) as usize]
    }
}

impl std::fmt::Display for CivilDate {
    /// The human-facing IC datetime: `"August 28, 2075, 02:57"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}, {}, {:02}:{:02}",
            self.month_name(),
            self.day,
            self.year,
            self.hour,
            self.minute
        )
    }
}

/// The four seasons, by IC day-of-year (meteorological-style quarters).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Season {
    /// IC days 334..365 and 0..59.
    Winter,
    /// IC days 59..151.
    Spring,
    /// IC days 151..243.
    Summer,
    /// IC days 243..334.
    Autumn,
}

impl std::fmt::Display for Season {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Season::Winter => "winter",
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
        })
    }
}

/// Coarse IC time-of-day buckets, by IC hour.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeOfDay {
    /// 00:00–05:59.
    Night,
    /// 06:00–07:59.
    Dawn,
    /// 08:00–11:59.
    Morning,
    /// 12:00–17:59.
    Afternoon,
    /// 18:00–19:59.
    Dusk,
    /// 20:00–23:59.
    Evening,
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TimeOfDay::Night => "night",
            TimeOfDay::Dawn => "dawn",
            TimeOfDay::Morning => "morning",
            TimeOfDay::Afternoon => "afternoon",
            TimeOfDay::Dusk => "dusk",
            TimeOfDay::Evening => "evening",
        })
    }
}

impl GameClock {
    /// IC unix-style timestamp for a real (OOC) unix timestamp.
    #[must_use]
    pub fn ic_unix(&self, now_unix: i64) -> i64 {
        let ratio = i64::from(self.ic_ratio.max(1));
        self.epoch_offset
            .saturating_add(now_unix.saturating_mul(ratio))
    }

    /// IC hour of day, `0..24`.
    #[must_use]
    pub fn ic_hour(&self, now_unix: i64) -> u32 {
        let secs = self.ic_unix(now_unix).rem_euclid(SECS_PER_DAY);
        (secs / 3_600) as u32
    }

    /// IC day of year, `0..365`.
    #[must_use]
    pub fn ic_day_of_year(&self, now_unix: i64) -> u32 {
        let days = self.ic_unix(now_unix).div_euclid(SECS_PER_DAY);
        days.rem_euclid(DAYS_PER_YEAR) as u32
    }

    /// IC year number (year 0 begins at IC timestamp 0; negative before).
    #[must_use]
    pub fn ic_year(&self, now_unix: i64) -> i64 {
        self.ic_unix(now_unix)
            .div_euclid(SECS_PER_DAY * DAYS_PER_YEAR)
    }

    /// Season for the given real timestamp.
    #[must_use]
    pub fn season(&self, now_unix: i64) -> Season {
        match self.ic_day_of_year(now_unix) {
            59..=150 => Season::Spring,
            151..=242 => Season::Summer,
            243..=333 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    /// Time-of-day bucket for the given real timestamp.
    #[must_use]
    pub fn time_of_day(&self, now_unix: i64) -> TimeOfDay {
        match self.ic_hour(now_unix) {
            0..=5 => TimeOfDay::Night,
            6..=7 => TimeOfDay::Dawn,
            8..=11 => TimeOfDay::Morning,
            12..=17 => TimeOfDay::Afternoon,
            18..=19 => TimeOfDay::Dusk,
            _ => TimeOfDay::Evening,
        }
    }

    /// IC minute of the hour, `0..60`.
    #[must_use]
    pub fn ic_minute(&self, now_unix: i64) -> u32 {
        ((self.ic_unix(now_unix).rem_euclid(SECS_PER_HOUR)) / 60) as u32
    }

    /// The full civil breakdown (year/month/day/time) at `now_unix`.
    #[must_use]
    pub fn civil(&self, now_unix: i64) -> CivilDate {
        CivilDate::from_ic_seconds(self.ic_unix(now_unix))
    }

    /// IC month of the year, `1..=12`.
    #[must_use]
    pub fn ic_month(&self, now_unix: i64) -> u32 {
        self.civil(now_unix).month
    }

    /// IC day of the **month**, `1..=` the month's length (contrast
    /// [`GameClock::ic_day_of_year`]).
    #[must_use]
    pub fn ic_day(&self, now_unix: i64) -> u32 {
        self.civil(now_unix).day
    }

    /// The IC month name at `now_unix` (`"January"`..`"December"`).
    #[must_use]
    pub fn ic_month_name(&self, now_unix: i64) -> &'static str {
        MONTH_NAMES[(self.ic_month(now_unix) - 1) as usize]
    }

    /// The human-facing IC datetime string (`"August 28, 2075, 02:57"`) —
    /// what the `time` command and web clock read.
    #[must_use]
    pub fn ic_datetime(&self, now_unix: i64) -> String {
        self.civil(now_unix).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_clock_maps_day_and_hour() {
        let c = GameClock::default();
        // Day 200, 13:00.
        let now = 200 * SECS_PER_DAY + 13 * 3_600;
        assert_eq!(c.ic_unix(now), now);
        assert_eq!(c.ic_day_of_year(now), 200);
        assert_eq!(c.ic_hour(now), 13);
        assert_eq!(c.season(now), Season::Summer);
        assert_eq!(c.time_of_day(now), TimeOfDay::Afternoon);
        assert_eq!(c.ic_year(now), 0);
    }

    #[test]
    fn season_boundaries() {
        let c = GameClock::default();
        let at_day = |d: i64| d * SECS_PER_DAY;
        assert_eq!(c.season(at_day(0)), Season::Winter);
        assert_eq!(c.season(at_day(58)), Season::Winter);
        assert_eq!(c.season(at_day(59)), Season::Spring);
        assert_eq!(c.season(at_day(150)), Season::Spring);
        assert_eq!(c.season(at_day(151)), Season::Summer);
        assert_eq!(c.season(at_day(242)), Season::Summer);
        assert_eq!(c.season(at_day(243)), Season::Autumn);
        assert_eq!(c.season(at_day(333)), Season::Autumn);
        assert_eq!(c.season(at_day(334)), Season::Winter);
        assert_eq!(c.season(at_day(364)), Season::Winter);
        // Wraps into the next year.
        assert_eq!(c.season(at_day(365 + 100)), Season::Spring);
    }

    #[test]
    fn time_of_day_buckets() {
        let c = GameClock::default();
        let at_hour = |h: i64| h * 3_600;
        assert_eq!(c.time_of_day(at_hour(0)), TimeOfDay::Night);
        assert_eq!(c.time_of_day(at_hour(5)), TimeOfDay::Night);
        assert_eq!(c.time_of_day(at_hour(6)), TimeOfDay::Dawn);
        assert_eq!(c.time_of_day(at_hour(8)), TimeOfDay::Morning);
        assert_eq!(c.time_of_day(at_hour(12)), TimeOfDay::Afternoon);
        assert_eq!(c.time_of_day(at_hour(18)), TimeOfDay::Dusk);
        assert_eq!(c.time_of_day(at_hour(20)), TimeOfDay::Evening);
        assert_eq!(c.time_of_day(at_hour(23)), TimeOfDay::Evening);
    }

    #[test]
    fn ratio_speeds_up_ic_time() {
        // 24x: one real hour = one IC day.
        let c = GameClock {
            ic_ratio: 24,
            epoch_offset: 0,
        };
        assert_eq!(c.ic_day_of_year(SECS_PER_DAY / 24), 1);
        assert_eq!(c.ic_hour(3_600 / 24), 1);
        // Ratio 0 behaves as 1 (documented safeguard).
        let c0 = GameClock {
            ic_ratio: 0,
            epoch_offset: 0,
        };
        assert_eq!(c0.ic_unix(42), 42);
    }

    #[test]
    fn offset_shifts_calendar_and_negatives_wrap() {
        let c = GameClock {
            ic_ratio: 1,
            epoch_offset: 250 * SECS_PER_DAY,
        };
        assert_eq!(c.ic_day_of_year(0), 250);
        assert_eq!(c.season(0), Season::Autumn);

        // Before the epoch: hours and days still wrap into range.
        let c = GameClock::default();
        assert_eq!(c.ic_hour(-3_600), 23);
        assert_eq!(c.ic_day_of_year(-SECS_PER_DAY), 364);
        assert_eq!(c.ic_year(-1), -1);
    }

    #[test]
    fn month_lengths_sum_to_a_full_year() {
        let sum: u32 = MONTH_LENGTHS.iter().sum();
        assert_eq!(i64::from(sum), DAYS_PER_YEAR);
    }

    #[test]
    fn civil_decomposes_calendar_fields() {
        // Day-of-year 0 is Jan 1; 31 is Feb 1; 59 is Mar 1 (28-day Feb).
        assert_eq!(CivilDate::from_ic_seconds(0).month, 1);
        assert_eq!(CivilDate::from_ic_seconds(0).day, 1);
        assert_eq!(CivilDate::from_ic_seconds(31 * SECS_PER_DAY).month, 2);
        assert_eq!(CivilDate::from_ic_seconds(31 * SECS_PER_DAY).day, 1);
        let mar1 = CivilDate::from_ic_seconds(59 * SECS_PER_DAY);
        assert_eq!((mar1.month, mar1.day), (3, 1));
        // Last day of the year is Dec 31.
        let dec31 = CivilDate::from_ic_seconds(364 * SECS_PER_DAY);
        assert_eq!(
            (dec31.month, dec31.day, dec31.month_name()),
            (12, 31, "December")
        );
        // Time-of-day fields.
        let t = CivilDate::from_ic_seconds(13 * SECS_PER_HOUR + 45 * 60 + 7);
        assert_eq!((t.hour, t.minute, t.second), (13, 45, 7));
    }

    #[test]
    fn civil_roundtrips_through_ic_seconds() {
        for ic in [
            0,
            59 * SECS_PER_DAY + 3_661,
            200 * SECS_PER_DAY + 13 * SECS_PER_HOUR,
            2020 * DAYS_PER_YEAR * SECS_PER_DAY + 86_399,
            364 * SECS_PER_DAY + 23 * SECS_PER_HOUR + 59 * 60 + 59,
        ] {
            let cv = CivilDate::from_ic_seconds(ic);
            assert_eq!(cv.to_ic_seconds(), ic, "roundtrip {ic}");
        }
    }

    #[test]
    fn parse_accepts_date_and_datetime_forms() {
        assert_eq!(
            CivilDate::parse("2020-01-01"),
            Some(CivilDate {
                year: 2020,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            })
        );
        // Space- and T-separated times; optional seconds.
        assert_eq!(CivilDate::parse("2020-06-15 14:30").unwrap().hour, 14);
        assert_eq!(CivilDate::parse("2020-06-15T14:30:45").unwrap().second, 45);
        // "2020-01-01 00:00" composes to exactly 2020 whole years of seconds.
        assert_eq!(
            CivilDate::parse("2020-01-01 00:00")
                .unwrap()
                .to_ic_seconds(),
            2020 * DAYS_PER_YEAR * SECS_PER_DAY
        );
    }

    #[test]
    fn parse_rejects_out_of_range_and_garbage() {
        assert_eq!(CivilDate::parse(""), None);
        assert_eq!(CivilDate::parse("2020-13-01"), None); // month 13
        assert_eq!(CivilDate::parse("2020-02-29"), None); // Feb has 28 days
        assert_eq!(CivilDate::parse("2020-00-10"), None); // month 0
        assert_eq!(CivilDate::parse("2020-01-00"), None); // day 0
        assert_eq!(CivilDate::parse("2020-01-01 24:00"), None); // hour 24
        assert_eq!(CivilDate::parse("2020-01-01 12:60"), None); // minute 60
        assert_eq!(CivilDate::parse("-5-01-01"), None); // negative year
        assert_eq!(CivilDate::parse("not-a-date"), None);
    }

    #[test]
    fn ic_datetime_renders_month_day_year_time() {
        // ic_ratio 1, epoch at a fixed civil datetime: at OOC time 0 the clock
        // reads exactly the epoch.
        let epoch = CivilDate::parse("2075-08-28 02:57").unwrap();
        let c = GameClock {
            ic_ratio: 1,
            epoch_offset: epoch.to_ic_seconds(),
        };
        assert_eq!(c.ic_datetime(0), "August 28, 2075, 02:57");
        assert_eq!((c.ic_month(0), c.ic_day(0), c.ic_year(0)), (8, 28, 2075));
        assert_eq!(c.ic_month_name(0), "August");
        assert_eq!(c.ic_minute(0), 57);
    }

    #[test]
    fn ratio_advances_month_and_datetime() {
        // ratio 4: four IC hours per OOC hour. Start at midnight Jan 1.
        let c = GameClock {
            ic_ratio: 4,
            epoch_offset: 0,
        };
        // One OOC hour -> four IC hours.
        assert_eq!(c.ic_hour(SECS_PER_HOUR), 4);
        // 6 OOC hours -> IC 24h -> Jan 2.
        let one_day_ooc = 6 * SECS_PER_HOUR;
        assert_eq!(c.ic_day(one_day_ooc), 2);
        assert_eq!(c.ic_month(one_day_ooc), 1);
    }

    /// A rate change opens a **continuous** new generation, and two generations are
    /// incomparable.
    ///
    /// *Rewritten in place by the E7 residue pass 1 independent verify fold (C9/B10,
    /// owner decision F9(a)).* This test used to install `rate = 0` and assert that
    /// the IC clock then stood still for ever — which is precisely the state
    /// [`MIN_WORLD_TIME_RATE`] now refuses, because it stops every `world_ic_now`
    /// mechanism in the world while the compatibility [`GameClock`]'s two `.max(1)`
    /// readers keep the clock a player reads in the room advancing at 1:1. Both
    /// properties it was really pinning — the anchor's continuity and the two
    /// generations' incomparability — hold at any legal rate, so they are asserted at
    /// a **slower** one instead, and the refusal is asserted beside them.
    #[test]
    fn rate_change_opens_a_continuous_new_generation() {
        let mut timeline = WorldTimeline::from_game_clock(GameClock {
            ic_ratio: 4,
            epoch_offset: 100,
        });
        let before = timeline.stamp_at(50);
        assert_eq!(before.at, 300);
        assert_eq!(before.clock, ClockRef::WorldIc { generation: 1 });

        let slowed = timeline.next_rate_epoch(50, 1).unwrap();
        assert_eq!(
            slowed.ic_anchor, before.at,
            "the new generation begins exactly where the old one stood"
        );
        timeline.append(slowed).unwrap();
        let after = timeline.stamp_at(5_000);
        assert_eq!(
            after.at,
            before.at + 4_950,
            "and advances at the NEW rate from there"
        );
        assert_eq!(after.clock, ClockRef::WorldIc { generation: 2 });

        assert!(matches!(
            before.checked_cmp(&after),
            Err(ClockMismatch { .. })
        ));
    }

    /// **A stopped clock is not a legal rate, and neither is one past the band**
    /// (independent verify fold, C9/B10; owner decision F9(a), band `1..=3_600`).
    ///
    /// The band is enforced on the **mutation** path only. Hydration
    /// ([`WorldTimeline::from_epochs`]) deliberately still accepts any persisted
    /// epoch: a world that installed an out-of-band rate before this landed must
    /// still boot and be repairable, and refusing its history would strand it. The
    /// two ways in are `next_rate_epoch` — every live caller — and
    /// `from_game_clock`, which already clamped.
    #[test]
    fn vf_c9_the_rate_band_is_refused_at_every_mutation_and_the_edges_are_legal() {
        assert_eq!(MIN_WORLD_TIME_RATE, 1);
        assert_eq!(MAX_WORLD_TIME_RATE, 3_600);
        const {
            assert!(MIN_WORLD_TIME_RATE >= 1);
            assert!(MAX_WORLD_TIME_RATE > MIN_WORLD_TIME_RATE);
        }
        let timeline = WorldTimeline::from_game_clock(GameClock {
            ic_ratio: 4,
            epoch_offset: 0,
        });
        for rate in [0, MAX_WORLD_TIME_RATE + 1, u32::MAX] {
            assert!(
                matches!(
                    timeline.next_rate_epoch(10, rate),
                    Err(WorldTimelineError::RateOutOfBand { .. })
                ),
                "rate {rate} must be refused, naming the band"
            );
        }
        for rate in [MIN_WORLD_TIME_RATE, 24, MAX_WORLD_TIME_RATE] {
            assert!(
                timeline.next_rate_epoch(10, rate).is_ok(),
                "rate {rate} is inside the band"
            );
        }
        // Boot was always protected, and still is.
        assert_eq!(
            WorldTimeline::from_game_clock(GameClock {
                ic_ratio: 0,
                epoch_offset: 0,
            })
            .current()
            .rate,
            1,
            "`GameClock`'s own `0 is treated as 1` sentinel is honoured at boot"
        );
    }

    #[test]
    fn persisted_timeline_rejects_gaps_backwards_time_and_discontinuity() {
        let first = WorldTimeline::default().current();
        let gap = WorldTimelineEpoch {
            real_anchor: 1,
            ic_anchor: 1,
            rate: 1,
            generation: 3,
        };
        assert!(matches!(
            WorldTimeline::from_epochs(vec![first, gap]),
            Err(WorldTimelineError::NonContiguousGeneration { .. })
        ));

        let backwards = WorldTimelineEpoch {
            real_anchor: -1,
            ic_anchor: -1,
            rate: 1,
            generation: 2,
        };
        assert!(matches!(
            WorldTimeline::from_epochs(vec![first, backwards]),
            Err(WorldTimelineError::AnchorMovedBackward { .. })
        ));

        let discontinuous = WorldTimelineEpoch {
            real_anchor: 1,
            ic_anchor: 99,
            rate: 1,
            generation: 2,
        };
        assert!(matches!(
            WorldTimeline::from_epochs(vec![first, discontinuous]),
            Err(WorldTimelineError::DiscontinuousAnchor { .. })
        ));
    }

    #[test]
    fn clock_stamp_wire_shape_is_strict_and_cross_clock_math_fails() {
        let world = ClockStamp {
            clock: ClockRef::WorldIc { generation: 7 },
            at: 42,
        };
        let utc = ClockStamp {
            clock: ClockRef::Utc,
            at: 42,
        };
        assert!(matches!(
            world.checked_duration_since(&utc),
            Err(ClockArithmeticError::Mismatch(_))
        ));
        assert!(
            serde_json::from_str::<ClockStamp>(
                r#"{"clock":{"kind":"world_ic","generation":7},"at":42,"extra":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn w_clock_freeze_and_resume_hold_ic_time_and_reanchor_within_same_generation() {
        let mut timeline = WorldTimeline::from_game_clock(GameClock {
            ic_ratio: 4,
            epoch_offset: 1_000,
        });
        assert_eq!(
            timeline.stamp_at(50),
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 1_200,
            }
        );

        let freeze = timeline.freeze_deliberate(50).unwrap();
        assert_eq!(freeze.frozen_real_unix, 50);
        assert_eq!(freeze.frozen_ic_unix, 1_200);
        assert_eq!(freeze.generation, 1);
        assert!(timeline.is_frozen());
        assert_eq!(
            timeline.freeze_deliberate(60),
            Err(WorldTimelineError::AlreadyFrozen)
        );
        assert_eq!(
            timeline.next_rate_epoch(60, 2),
            Err(WorldTimelineError::ClockFrozen)
        );

        // While frozen, real time advancing does not advance IC time.
        assert_eq!(timeline.stamp_at(50).at, 1_200);
        assert_eq!(timeline.stamp_at(500).at, 1_200);

        // Resume at real=500 re-anchors generation 1 to (real=500, ic=1_200).
        let (resumed_freeze, current) = timeline.resume_at(500).unwrap();
        assert_eq!(resumed_freeze, freeze);
        assert!(!timeline.is_frozen());
        assert_eq!(current.generation, 1);
        assert_eq!(current.real_anchor, 500);
        assert_eq!(current.ic_anchor, 1_200);
        assert_eq!(current.rate, 4);
        assert_eq!(timeline.stamp_at(500).at, 1_200);
        assert_eq!(timeline.stamp_at(510).at, 1_240);

        // A later rate change opens generation 2 continuously from the re-anchored generation 1,
        // and round-trips through `from_epochs_and_state`.
        let gen2 = timeline.next_rate_epoch(510, 2).unwrap();
        assert_eq!(gen2.ic_anchor, 1_240);
        timeline.append(gen2).unwrap();
        let hydrated =
            WorldTimeline::from_epochs_and_state(timeline.epochs().to_vec(), timeline.state())
                .unwrap();
        assert_eq!(hydrated, timeline);
        assert_eq!(hydrated.stamp_at(520).at, 1_260);
    }

    #[test]
    fn w_clock_reanchor_from_checkpoint_excludes_lost_wall_time_without_catch_up() {
        let mut timeline = WorldTimeline::from_game_clock(GameClock {
            ic_ratio: 24,
            epoch_offset: 10_000,
        });
        let cp = timeline.record_checkpoint(100);
        assert_eq!(cp.ic_unix, 12_400);

        // Restart 15 real seconds later (at real=115): re-anchors silently to cp.ic_unix.
        let lost = timeline.reanchor_from_checkpoint(115).unwrap();
        assert_eq!(lost, Some(15));
        assert_eq!(timeline.stamp_at(115).at, 12_400);
        assert_eq!(timeline.stamp_at(116).at, 12_424);
    }
}
