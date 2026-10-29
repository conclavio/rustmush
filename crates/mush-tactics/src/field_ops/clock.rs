//! `W-clock` — Global outage freeze, 5-minute recovery countdown, sub-threshold
//! silent re-anchor, incompatible-state quarantine, and real-time salvage contest
//! deadline adjustment (design doc 27 `FO-11`, `Q-H11`, `Q-F3`, `Q-F4`, `Q-R4`, `P-4`).

use std::collections::BTreeMap;
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::FieldCustodyLedger;
use crate::field_ops::engagement::FieldSiteEngagementBook;
use crate::field_ops::salvage::{SalvageClaimBook, SalvageClaimId, SalvageSubject};
use crate::unit::TacticalUnit;

/// Default lost wall-clock threshold in seconds (`30s`) below which a hot restart
/// re-anchors silently with no countdown (`Q-R4`).
pub const DEFAULT_OUTAGE_FREEZE_THRESHOLD_REAL_S: i64 = 30;

/// Production default post-outage recovery countdown in real seconds (`300s` = 5 minutes, `FO-11`, `Q-H11`, `Q-R4`).
pub const DEFAULT_OUTAGE_COUNTDOWN_REAL_S: i64 = 300;

/// Maximum quarantined unit records retained in one [`FieldClockCoordinator`].
pub const MAX_QUARANTINED_UNITS: usize = 512;

/// Configuration knobs for `W-clock` restart evaluation and countdown duration (`Q-R4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutageFreezeConfig {
    /// Minimum lost wall-clock seconds (`>= 1`) that triggers an outage freeze and
    /// countdown. Lost wall time below this threshold re-anchors silently (`Q-R4`).
    pub freeze_threshold_real_s: i64,
    /// Duration of the post-outage recovery countdown in real seconds (`>= 0`, production
    /// default `300s` = 5 minutes, `FO-11`, `Q-H11`, `Q-R4`).
    pub countdown_duration_real_s: i64,
}

impl Default for OutageFreezeConfig {
    fn default() -> Self {
        Self {
            freeze_threshold_real_s: DEFAULT_OUTAGE_FREEZE_THRESHOLD_REAL_S,
            countdown_duration_real_s: DEFAULT_OUTAGE_COUNTDOWN_REAL_S,
        }
    }
}

impl OutageFreezeConfig {
    /// Validate that `freeze_threshold_real_s >= 1` and `countdown_duration_real_s >= 0`.
    ///
    /// # Errors
    /// Returns [`ClockFreezeError::InvalidConfig`] if either parameter is out of range.
    pub const fn validate(&self) -> Result<(), ClockFreezeError> {
        if self.freeze_threshold_real_s < 1 || self.countdown_duration_real_s < 0 {
            return Err(ClockFreezeError::InvalidConfig {
                freeze_threshold_real_s: self.freeze_threshold_real_s,
                countdown_duration_real_s: self.countdown_duration_real_s,
            });
        }
        Ok(())
    }
}

/// Broadcast milestone emitted during a post-outage recovery countdown (`FO-11`, `Q-H11`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "milestone", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutageBroadcastMilestone {
    /// Broadcast at the 5-minute (`> 120s` remaining) countdown mark (`FO-11`, `Q-H11`).
    FiveMinutes {
        /// Remaining real seconds until unfreeze.
        remaining_real_s: i64,
    },
    /// Broadcast at the 2-minute (`<= 120s` and `> 60s` remaining) countdown mark (`FO-11`, `Q-H11`).
    TwoMinutes {
        /// Remaining real seconds until unfreeze.
        remaining_real_s: i64,
    },
    /// Broadcast at the 1-minute (`<= 60s` and `> 0s` remaining) countdown mark (`FO-11`, `Q-H11`).
    OneMinute {
        /// Remaining real seconds until unfreeze.
        remaining_real_s: i64,
    },
    /// Broadcast when the recovery countdown reaches zero and the world clock unfreezes (`FO-11`, `Q-H11`).
    Unfrozen,
}

/// Status banner delivered to a player who reconnects while the world clock is frozen (`Q-H11`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutageReconnectBanner {
    /// Real-time UTC second when the world clock froze.
    pub frozen_at_real_s: i64,
    /// Frozen IC timestamp held constant during the freeze.
    pub frozen_ic_s: i64,
    /// Remaining countdown seconds if frozen for an outage recovery countdown (`None` for a deliberate freeze).
    pub remaining_countdown_real_s: Option<i64>,
    /// Scheduled real-time UTC second when the countdown will unfreeze (`None` for a deliberate freeze).
    pub unfreeze_at_real_s: Option<i64>,
}

/// Published deadline adjustment for a pending [`SalvageClaim`](crate::field_ops::salvage::SalvageClaim)
/// when the world clock unfreezes (`Q-F3`, `P-4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustedSalvageClaimDeadline {
    /// Adjusted salvage claim id.
    pub claim_id: SalvageClaimId,
    /// Claimed hull or field-stripped component.
    pub subject: SalvageSubject,
    /// Recoverer / claimant object id.
    pub claimant: ObjId,
    /// Prior title holder object id.
    pub prior_owner: ObjId,
    /// Deadline in real-time UTC seconds before this outage adjustment.
    pub previous_deadline_real_s: i64,
    /// Adjusted deadline in real-time UTC seconds after adding the unavailable duration (`Q-F3`).
    pub adjusted_deadline_real_s: i64,
    /// Remaining available real seconds in the 48-hour contest window at unfreeze (`Q-F3`).
    pub remaining_available_real_s: i64,
    /// Cumulative real seconds paused across all outages for this claim (`Q-F3`).
    pub total_paused_real_s: i64,
}

/// Audit record for an active field unit quarantined during outage recovery due to
/// incompatible state, preserving its exact state for staff/audit recovery rather than
/// resetting the unit to stock (`Q-H11`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinedUnitRecord {
    /// Quarantined unit object id.
    pub unit: ObjId,
    /// Frontier site where the unit was active.
    pub site: ObjId,
    /// Reason the unit failed post-outage consistency validation.
    pub reason: SmolStr,
    /// Preserved `TacticalUnit::revision()` at quarantine (never reset to stock).
    pub preserved_revision: u64,
    /// Real-time UTC second when the unit was quarantined.
    pub quarantined_at_real_s: i64,
    /// Frozen/current IC timestamp when the unit was quarantined.
    pub quarantined_at_ic_s: i64,
}

/// Active freeze state managed by [`FieldClockCoordinator`] (`W-clock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActiveFieldFreeze {
    /// Deliberate staff/campaign freeze.
    Deliberate {
        /// Real-time UTC second when the freeze began.
        frozen_at_real_s: i64,
        /// Frozen IC timestamp held constant.
        frozen_ic_s: i64,
    },
    /// Automatic post-outage recovery countdown (`FO-11`, `Q-H11`, `Q-R4`).
    OutageCountdown {
        /// Real-time UTC second when the outage began (last durable checkpoint before outage).
        frozen_at_real_s: i64,
        /// Frozen IC timestamp at `frozen_at_real_s`.
        frozen_ic_s: i64,
        /// Real-time UTC second when the current recovery countdown started.
        countdown_started_real_s: i64,
        /// Real-time UTC second when the countdown completes and the world unfreezes.
        unfreeze_at_real_s: i64,
        /// Whether the 5-minute broadcast milestone has been emitted.
        announced_5m: bool,
        /// Whether the 2-minute broadcast milestone has been emitted.
        announced_2m: bool,
        /// Whether the 1-minute broadcast milestone has been emitted.
        announced_1m: bool,
    },
}

impl ActiveFieldFreeze {
    /// Real-time UTC second when the world became unavailable / frozen.
    #[must_use]
    pub const fn frozen_at_real_s(&self) -> i64 {
        match self {
            Self::Deliberate {
                frozen_at_real_s, ..
            }
            | Self::OutageCountdown {
                frozen_at_real_s, ..
            } => *frozen_at_real_s,
        }
    }

    /// Frozen IC timestamp held constant while frozen.
    #[must_use]
    pub const fn frozen_ic_s(&self) -> i64 {
        match self {
            Self::Deliberate { frozen_ic_s, .. } | Self::OutageCountdown { frozen_ic_s, .. } => {
                *frozen_ic_s
            }
        }
    }
}

/// Outcome of evaluating a server restart / boot timestamp against the last durable
/// checkpoint (`W-clock`, `FO-11`, `Q-H11`, `Q-R4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldOutageResolution {
    /// Initial checkpoint seeded on first boot.
    InitialCheckpointSeeded {
        /// Seeded real-time UTC second.
        real_s: i64,
        /// Seeded IC timestamp.
        ic_s: i64,
    },
    /// Lost wall time was below [`OutageFreezeConfig::freeze_threshold_real_s`]; the clock
    /// re-anchored silently to `anchored_real_s` with zero unseen catch-up and no countdown (`Q-R4`).
    SilentReanchor {
        /// Excluded wall-clock gap in seconds.
        lost_real_s: i64,
        /// New real-time UTC anchor.
        anchored_real_s: i64,
        /// Preserved IC anchor from the checkpoint.
        anchored_ic_s: i64,
    },
    /// Lost wall time met or exceeded the threshold; the world is frozen for the recovery
    /// countdown and pending salvage contest windows are paused (`FO-11`, `Q-H11`, `Q-F3`, `Q-R4`).
    OutageCountdownStarted {
        /// Total wall-clock seconds elapsed since the pre-outage checkpoint.
        lost_real_s: i64,
        /// Real-time UTC second when the world became unavailable (pre-outage checkpoint).
        frozen_at_real_s: i64,
        /// Frozen IC timestamp held constant during the countdown.
        frozen_ic_s: i64,
        /// Real-time UTC second when this countdown started.
        countdown_started_real_s: i64,
        /// Real-time UTC second when the world will unfreeze.
        unfreeze_at_real_s: i64,
        /// Initial broadcast milestones emitted immediately at countdown start (e.g. `FiveMinutes`).
        initial_milestones: Vec<OutageBroadcastMilestone>,
    },
    /// World was already under a deliberate staff freeze prior to restart; remains frozen.
    DeliberateFreezePreserved {
        /// Original freeze real-time UTC second.
        frozen_at_real_s: i64,
        /// Preserved frozen IC timestamp.
        frozen_ic_s: i64,
    },
}

/// Summary produced when an outage countdown or deliberate freeze completes and the
/// world clock resumes (`FO-11`, `Q-H11`, `Q-F3`, `P-4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutageUnfreezeReport {
    /// Real-time UTC second when the world originally froze.
    pub frozen_at_real_s: i64,
    /// Real-time UTC second when the world unfroze and resumed.
    pub unfrozen_at_real_s: i64,
    /// Total unavailable + countdown real seconds excluded from deadlines (`unfrozen_at_real_s - frozen_at_real_s`).
    pub unavailable_real_s: i64,
    /// Re-anchored IC timestamp from which campaign/surface progression resumes with zero unseen catch-up.
    pub resumed_ic_s: i64,
    /// Published deadline adjustments for all pending salvage claims (`Q-F3`).
    pub adjusted_salvage_claims: Vec<AdjustedSalvageClaimDeadline>,
    /// Units currently held in post-outage audit quarantine (`Q-H11`).
    pub quarantined_units: Vec<ObjId>,
}

/// Coordinator for `W-clock` outage freezes, 5-minute recovery countdowns, checkpoint
/// re-anchoring, salvage contest deadline extensions, and post-outage unit quarantine
/// (`FO-11`, `Q-H11`, `Q-F3`, `Q-F4`, `Q-R4`, `P-4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldClockCoordinator {
    config: OutageFreezeConfig,
    ic_rate: u32,
    checkpoint_real_s: Option<i64>,
    checkpoint_ic_s: Option<i64>,
    active_freeze: Option<ActiveFieldFreeze>,
    quarantined_units: BTreeMap<ObjId, QuarantinedUnitRecord>,
}

impl FieldClockCoordinator {
    /// Construct a [`FieldClockCoordinator`] with validated `config` and `ic_rate` (`>= 1`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if `config` is invalid or `ic_rate == 0`.
    pub fn new(config: OutageFreezeConfig, ic_rate: u32) -> Result<Self, ClockFreezeError> {
        config.validate()?;
        if ic_rate == 0 {
            return Err(ClockFreezeError::ZeroIcRate);
        }
        Ok(Self {
            config,
            ic_rate,
            checkpoint_real_s: None,
            checkpoint_ic_s: None,
            active_freeze: None,
            quarantined_units: BTreeMap::new(),
        })
    }

    /// Current outage freeze configuration.
    #[must_use]
    pub const fn config(&self) -> OutageFreezeConfig {
        self.config
    }

    /// Configured IC rate (IC seconds per real second).
    #[must_use]
    pub const fn ic_rate(&self) -> u32 {
        self.ic_rate
    }

    /// Most recent durable `(real_s, ic_s)` checkpoint, if recorded.
    #[must_use]
    pub const fn checkpoint(&self) -> Option<(i64, i64)> {
        match (self.checkpoint_real_s, self.checkpoint_ic_s) {
            (Some(r), Some(ic)) => Some((r, ic)),
            _ => None,
        }
    }

    /// Active freeze state, if the world clock is frozen.
    #[must_use]
    pub const fn active_freeze(&self) -> Option<&ActiveFieldFreeze> {
        self.active_freeze.as_ref()
    }

    /// Whether the world clock is currently frozen (`Deliberate` or `OutageCountdown`).
    #[must_use]
    pub const fn is_frozen(&self) -> bool {
        self.active_freeze.is_some()
    }

    /// All units currently quarantined for post-outage audit (`Q-H11`).
    #[must_use]
    pub const fn quarantined_units(&self) -> &BTreeMap<ObjId, QuarantinedUnitRecord> {
        &self.quarantined_units
    }

    /// Gate that refuses gameplay mutations, combat ticks, towing, loading, and surface
    /// motion while the world clock is frozen (`FO-11`, `Q-H11`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError::WorldClockFrozen`] if a freeze is active.
    pub fn ensure_unfrozen(&self) -> Result<(), ClockFreezeError> {
        if let Some(freeze) = &self.active_freeze {
            return Err(ClockFreezeError::WorldClockFrozen {
                frozen_at_real_s: freeze.frozen_at_real_s(),
                frozen_ic_s: freeze.frozen_ic_s(),
            });
        }
        Ok(())
    }

    /// Gate that refuses active combat or recovery actions on a unit held in post-outage
    /// audit quarantine (`Q-H11`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError::UnitQuarantined`] if `unit` is quarantined.
    pub fn ensure_unit_not_quarantined(&self, unit: ObjId) -> Result<(), ClockFreezeError> {
        if let Some(rec) = self.quarantined_units.get(&unit) {
            return Err(ClockFreezeError::UnitQuarantined {
                unit,
                reason: rec.reason.clone(),
            });
        }
        Ok(())
    }

    /// Evaluate the current IC timestamp at `now_real_s`.
    ///
    /// While frozen, returns the exact `frozen_ic_s` without advancing (`Q-H11`, `P-4`).
    /// While unfrozen, advances from the last durable checkpoint at `ic_rate`.
    #[must_use]
    pub fn ic_at(&self, now_real_s: i64) -> Option<i64> {
        if let Some(freeze) = &self.active_freeze {
            return Some(freeze.frozen_ic_s());
        }
        let (cp_real, cp_ic) = self.checkpoint()?;
        let delta_real = i128::from(now_real_s) - i128::from(cp_real);
        let value = i128::from(cp_ic) + delta_real * i128::from(self.ic_rate);
        Some(value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
    }

    /// Record a durable campaign/tactical clock checkpoint at `(real_s, ic_s)` (`Q-H11`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if the clock is currently frozen or `real_s` moves backward.
    pub fn record_checkpoint(&mut self, real_s: i64, ic_s: i64) -> Result<(), ClockFreezeError> {
        self.ensure_unfrozen()?;
        if let Some(prev_real) = self.checkpoint_real_s
            && real_s < prev_real
        {
            return Err(ClockFreezeError::TimestampRegression {
                previous: prev_real,
                proposed: real_s,
            });
        }
        self.checkpoint_real_s = Some(real_s);
        self.checkpoint_ic_s = Some(ic_s);
        Ok(())
    }

    /// Build the status banner sent to a player reconnecting at `now_real_s` while the
    /// world clock is frozen (`Q-H11`).
    #[must_use]
    pub fn reconnect_banner(&self, now_real_s: i64) -> Option<OutageReconnectBanner> {
        let freeze = self.active_freeze.as_ref()?;
        match freeze {
            ActiveFieldFreeze::Deliberate {
                frozen_at_real_s,
                frozen_ic_s,
            } => Some(OutageReconnectBanner {
                frozen_at_real_s: *frozen_at_real_s,
                frozen_ic_s: *frozen_ic_s,
                remaining_countdown_real_s: None,
                unfreeze_at_real_s: None,
            }),
            ActiveFieldFreeze::OutageCountdown {
                frozen_at_real_s,
                frozen_ic_s,
                unfreeze_at_real_s,
                ..
            } => Some(OutageReconnectBanner {
                frozen_at_real_s: *frozen_at_real_s,
                frozen_ic_s: *frozen_ic_s,
                remaining_countdown_real_s: Some(
                    unfreeze_at_real_s.saturating_sub(now_real_s).max(0),
                ),
                unfreeze_at_real_s: Some(*unfreeze_at_real_s),
            }),
        }
    }

    /// Deliberately freeze the world clock at `now_real_s`, pausing all pending salvage
    /// claim contest windows (`W-clock`, `Q-F3`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if already frozen or `now_real_s` precedes the checkpoint.
    pub fn freeze_deliberate(
        &mut self,
        now_real_s: i64,
        salvage_book: &mut SalvageClaimBook,
    ) -> Result<ActiveFieldFreeze, ClockFreezeError> {
        self.ensure_unfrozen()?;
        if let Some(prev_real) = self.checkpoint_real_s
            && now_real_s < prev_real
        {
            return Err(ClockFreezeError::TimestampRegression {
                previous: prev_real,
                proposed: now_real_s,
            });
        }
        let frozen_ic_s = self.ic_at(now_real_s).unwrap_or(0);
        let freeze = ActiveFieldFreeze::Deliberate {
            frozen_at_real_s: now_real_s,
            frozen_ic_s,
        };
        self.active_freeze = Some(freeze);
        self.checkpoint_real_s = Some(now_real_s);
        self.checkpoint_ic_s = Some(frozen_ic_s);
        salvage_book.pause_claims_for_outage(now_real_s);
        Ok(freeze)
    }

    /// Evaluate a server restart at `boot_real_s` against the last durable checkpoint (`FO-11`, `Q-H11`, `Q-R4`).
    ///
    /// - If no prior checkpoint exists, seeds `(boot_real_s, 0)`.
    /// - If an `OutageCountdown` was already active and a further outage (`>= freeze_threshold_real_s`)
    ///   occurred before unfreeze, starts a **fresh countdown** from `boot_real_s` while
    ///   preserving the original `frozen_at_real_s` and `frozen_ic_s` (`Q-H11`).
    /// - If lost wall time is `< freeze_threshold_real_s`, silently re-anchors the checkpoint
    ///   to `boot_real_s` with zero unseen catch-up and no countdown (`Q-R4`).
    /// - If lost wall time is `>= freeze_threshold_real_s`, enters `OutageCountdown`, pauses
    ///   all pending salvage contest windows from the pre-outage checkpoint (`Q-F3`), and
    ///   emits the initial countdown broadcast milestone (`FiveMinutes`, `TwoMinutes`, or `OneMinute`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError::TimestampRegression`] if `boot_real_s` is earlier than the checkpoint.
    pub fn evaluate_restart_gap(
        &mut self,
        boot_real_s: i64,
        salvage_book: &mut SalvageClaimBook,
    ) -> Result<FieldOutageResolution, ClockFreezeError> {
        if let Some(existing) = self.active_freeze {
            return match existing {
                ActiveFieldFreeze::Deliberate {
                    frozen_at_real_s,
                    frozen_ic_s,
                } => Ok(FieldOutageResolution::DeliberateFreezePreserved {
                    frozen_at_real_s,
                    frozen_ic_s,
                }),
                ActiveFieldFreeze::OutageCountdown {
                    frozen_at_real_s,
                    frozen_ic_s,
                    countdown_started_real_s,
                    ..
                } => {
                    if boot_real_s < countdown_started_real_s {
                        return Err(ClockFreezeError::TimestampRegression {
                            previous: countdown_started_real_s,
                            proposed: boot_real_s,
                        });
                    }
                    let unfreeze_at_real_s =
                        boot_real_s.saturating_add(self.config.countdown_duration_real_s);
                    let mut initial_milestones = Vec::new();
                    let remaining = self.config.countdown_duration_real_s;
                    let announced_5m = if remaining > 120 {
                        initial_milestones.push(OutageBroadcastMilestone::FiveMinutes {
                            remaining_real_s: remaining,
                        });
                        true
                    } else {
                        false
                    };
                    let announced_2m = if remaining <= 120 && remaining > 60 {
                        initial_milestones.push(OutageBroadcastMilestone::TwoMinutes {
                            remaining_real_s: remaining,
                        });
                        true
                    } else {
                        false
                    };
                    let announced_1m = if remaining <= 60 && remaining > 0 {
                        initial_milestones.push(OutageBroadcastMilestone::OneMinute {
                            remaining_real_s: remaining,
                        });
                        true
                    } else {
                        false
                    };

                    self.active_freeze = Some(ActiveFieldFreeze::OutageCountdown {
                        frozen_at_real_s,
                        frozen_ic_s,
                        countdown_started_real_s: boot_real_s,
                        unfreeze_at_real_s,
                        announced_5m,
                        announced_2m,
                        announced_1m,
                    });
                    salvage_book.pause_claims_for_outage(frozen_at_real_s);
                    Ok(FieldOutageResolution::OutageCountdownStarted {
                        lost_real_s: boot_real_s.saturating_sub(frozen_at_real_s),
                        frozen_at_real_s,
                        frozen_ic_s,
                        countdown_started_real_s: boot_real_s,
                        unfreeze_at_real_s,
                        initial_milestones,
                    })
                }
            };
        }

        let Some((cp_real, cp_ic)) = self.checkpoint() else {
            self.checkpoint_real_s = Some(boot_real_s);
            self.checkpoint_ic_s = Some(0);
            return Ok(FieldOutageResolution::InitialCheckpointSeeded {
                real_s: boot_real_s,
                ic_s: 0,
            });
        };

        if boot_real_s < cp_real {
            return Err(ClockFreezeError::TimestampRegression {
                previous: cp_real,
                proposed: boot_real_s,
            });
        }

        let lost_real_s = boot_real_s.saturating_sub(cp_real);
        if lost_real_s < self.config.freeze_threshold_real_s
            || self.config.countdown_duration_real_s == 0
        {
            self.checkpoint_real_s = Some(boot_real_s);
            self.checkpoint_ic_s = Some(cp_ic);
            return Ok(FieldOutageResolution::SilentReanchor {
                lost_real_s,
                anchored_real_s: boot_real_s,
                anchored_ic_s: cp_ic,
            });
        }

        let unfreeze_at_real_s = boot_real_s.saturating_add(self.config.countdown_duration_real_s);
        let remaining = self.config.countdown_duration_real_s;
        let mut initial_milestones = Vec::new();
        let announced_5m = if remaining > 120 {
            initial_milestones.push(OutageBroadcastMilestone::FiveMinutes {
                remaining_real_s: remaining,
            });
            true
        } else {
            false
        };
        let announced_2m = if remaining <= 120 && remaining > 60 {
            initial_milestones.push(OutageBroadcastMilestone::TwoMinutes {
                remaining_real_s: remaining,
            });
            true
        } else {
            false
        };
        let announced_1m = if remaining <= 60 && remaining > 0 {
            initial_milestones.push(OutageBroadcastMilestone::OneMinute {
                remaining_real_s: remaining,
            });
            true
        } else {
            false
        };

        self.active_freeze = Some(ActiveFieldFreeze::OutageCountdown {
            frozen_at_real_s: cp_real,
            frozen_ic_s: cp_ic,
            countdown_started_real_s: boot_real_s,
            unfreeze_at_real_s,
            announced_5m,
            announced_2m,
            announced_1m,
        });
        salvage_book.pause_claims_for_outage(cp_real);

        Ok(FieldOutageResolution::OutageCountdownStarted {
            lost_real_s,
            frozen_at_real_s: cp_real,
            frozen_ic_s: cp_ic,
            countdown_started_real_s: boot_real_s,
            unfreeze_at_real_s,
            initial_milestones,
        })
    }

    /// Advance an active outage recovery countdown to `now_real_s`, emitting any newly
    /// crossed broadcast milestones (`5m`, `2m`, `1m`, `Unfrozen`) and unfreezing the
    /// world clock (and extending salvage contest deadlines) once `now_real_s >= unfreeze_at_real_s`
    /// (`FO-11`, `Q-H11`, `Q-F3`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if no outage countdown is active or `now_real_s` regresses.
    pub fn step_countdown(
        &mut self,
        now_real_s: i64,
        salvage_book: &mut SalvageClaimBook,
    ) -> Result<(Vec<OutageBroadcastMilestone>, Option<OutageUnfreezeReport>), ClockFreezeError>
    {
        let Some(ActiveFieldFreeze::OutageCountdown {
            frozen_at_real_s,
            frozen_ic_s,
            countdown_started_real_s,
            unfreeze_at_real_s,
            mut announced_5m,
            mut announced_2m,
            mut announced_1m,
        }) = self.active_freeze
        else {
            return Err(ClockFreezeError::NotInOutageCountdown);
        };

        if now_real_s < countdown_started_real_s {
            return Err(ClockFreezeError::TimestampRegression {
                previous: countdown_started_real_s,
                proposed: now_real_s,
            });
        }

        let remaining = unfreeze_at_real_s.saturating_sub(now_real_s);
        let mut milestones = Vec::new();

        if remaining <= 0 {
            self.active_freeze = None;
            self.checkpoint_real_s = Some(now_real_s);
            self.checkpoint_ic_s = Some(frozen_ic_s);
            let adjusted_salvage_claims = salvage_book.resume_claims_after_outage(now_real_s);
            milestones.push(OutageBroadcastMilestone::Unfrozen);
            let report = OutageUnfreezeReport {
                frozen_at_real_s,
                unfrozen_at_real_s: now_real_s,
                unavailable_real_s: now_real_s.saturating_sub(frozen_at_real_s),
                resumed_ic_s: frozen_ic_s,
                adjusted_salvage_claims,
                quarantined_units: self.quarantined_units.keys().copied().collect(),
            };
            return Ok((milestones, Some(report)));
        }

        if !announced_5m && remaining > 120 {
            announced_5m = true;
            milestones.push(OutageBroadcastMilestone::FiveMinutes {
                remaining_real_s: remaining,
            });
        }
        if !announced_2m && remaining <= 120 && remaining > 60 {
            announced_2m = true;
            milestones.push(OutageBroadcastMilestone::TwoMinutes {
                remaining_real_s: remaining,
            });
        }
        if !announced_1m && remaining <= 60 {
            announced_1m = true;
            milestones.push(OutageBroadcastMilestone::OneMinute {
                remaining_real_s: remaining,
            });
        }

        self.active_freeze = Some(ActiveFieldFreeze::OutageCountdown {
            frozen_at_real_s,
            frozen_ic_s,
            countdown_started_real_s,
            unfreeze_at_real_s,
            announced_5m,
            announced_2m,
            announced_1m,
        });

        Ok((milestones, None))
    }

    /// Resume the world clock from a deliberate freeze at `now_real_s`, re-anchoring the
    /// checkpoint and extending paused salvage contest windows (`W-clock`, `Q-F3`).
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if the clock is not frozen or `now_real_s` regresses.
    pub fn resume_freeze(
        &mut self,
        now_real_s: i64,
        salvage_book: &mut SalvageClaimBook,
    ) -> Result<OutageUnfreezeReport, ClockFreezeError> {
        let Some(freeze) = self.active_freeze else {
            return Err(ClockFreezeError::NotFrozen);
        };
        let frozen_at_real_s = freeze.frozen_at_real_s();
        let frozen_ic_s = freeze.frozen_ic_s();
        if now_real_s < frozen_at_real_s {
            return Err(ClockFreezeError::TimestampRegression {
                previous: frozen_at_real_s,
                proposed: now_real_s,
            });
        }
        self.active_freeze = None;
        self.checkpoint_real_s = Some(now_real_s);
        self.checkpoint_ic_s = Some(frozen_ic_s);
        let adjusted_salvage_claims = salvage_book.resume_claims_after_outage(now_real_s);
        Ok(OutageUnfreezeReport {
            frozen_at_real_s,
            unfrozen_at_real_s: now_real_s,
            unavailable_real_s: now_real_s.saturating_sub(frozen_at_real_s),
            resumed_ic_s: frozen_ic_s,
            adjusted_salvage_claims,
            quarantined_units: self.quarantined_units.keys().copied().collect(),
        })
    }

    /// Audit an active site unit after an outage against [`FieldCustodyLedger`] and its
    /// persisted [`TacticalUnit`] revision (`Q-H11`).
    ///
    /// If the unit's revision on `site_book` or `ledger` is incompatible with `unit.revision()`,
    /// or its custody record is missing, the unit is quarantined into `quarantined_units`
    /// for audited recovery rather than resetting the unit to stock (`Q-H11`).
    ///
    /// Returns `Ok(None)` if the unit is consistent, or `Ok(Some(QuarantinedUnitRecord))`
    /// if it was quarantined.
    ///
    /// # Errors
    /// Returns [`ClockFreezeError`] if the quarantine table is full.
    pub fn audit_or_quarantine_site_unit(
        &mut self,
        site_book: &FieldSiteEngagementBook,
        ledger: &FieldCustodyLedger,
        unit: &TacticalUnit,
        now_real_s: i64,
    ) -> Result<Option<QuarantinedUnitRecord>, ClockFreezeError> {
        let unit_id = unit.object();
        let Some(presence) = site_book.unit_presence(unit_id) else {
            return Ok(None);
        };

        let mismatch_reason = match ledger.get(unit_id) {
            None => Some(SmolStr::new("missing_field_custody_record")),
            Some(custody_rec) => {
                if custody_rec.unit_revision() != unit.revision() {
                    Some(SmolStr::new("custody_revision_mismatch"))
                } else if presence.assessment.unit_revision != unit.revision() {
                    Some(SmolStr::new("site_presence_revision_mismatch"))
                } else {
                    None
                }
            }
        };

        let Some(reason) = mismatch_reason else {
            return Ok(None);
        };

        if !self.quarantined_units.contains_key(&unit_id)
            && self.quarantined_units.len() >= MAX_QUARANTINED_UNITS
        {
            return Err(ClockFreezeError::MaxQuarantinedUnitsReached {
                max: MAX_QUARANTINED_UNITS,
            });
        }

        let ic_s = self.ic_at(now_real_s).unwrap_or(0);
        let record = QuarantinedUnitRecord {
            unit: unit_id,
            site: site_book.site(),
            reason,
            preserved_revision: unit.revision(),
            quarantined_at_real_s: now_real_s,
            quarantined_at_ic_s: ic_s,
        };
        self.quarantined_units.insert(unit_id, record.clone());
        Ok(Some(record))
    }

    /// Release a unit from post-outage audit quarantine once staff/audit reconciliation completes.
    pub fn release_quarantined_unit(&mut self, unit: ObjId) -> Option<QuarantinedUnitRecord> {
        self.quarantined_units.remove(&unit)
    }
}

/// Errors returned by `W-clock` [`FieldClockCoordinator`] operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockFreezeError {
    /// Invalid [`OutageFreezeConfig`] thresholds.
    InvalidConfig {
        /// Supplied freeze threshold in seconds.
        freeze_threshold_real_s: i64,
        /// Supplied countdown duration in seconds.
        countdown_duration_real_s: i64,
    },
    /// IC clock rate must be `>= 1`.
    ZeroIcRate,
    /// Action refused because the world clock is currently frozen (`FO-11`, `Q-H11`).
    WorldClockFrozen {
        /// Real-time UTC second when the freeze began.
        frozen_at_real_s: i64,
        /// Frozen IC timestamp.
        frozen_ic_s: i64,
    },
    /// World clock is not currently frozen.
    NotFrozen,
    /// World clock is not in an active outage countdown.
    NotInOutageCountdown,
    /// Wall-clock timestamp regressed before the previous anchor/checkpoint.
    TimestampRegression {
        /// Previous real-time UTC second.
        previous: i64,
        /// Proposed real-time UTC second.
        proposed: i64,
    },
    /// Action refused because the unit is held in post-outage audit quarantine (`Q-H11`).
    UnitQuarantined {
        /// Quarantined unit object id.
        unit: ObjId,
        /// Audit quarantine reason.
        reason: SmolStr,
    },
    /// Quarantine table capacity reached.
    MaxQuarantinedUnitsReached {
        /// Maximum quarantined units.
        max: usize,
    },
}

impl fmt::Display for ClockFreezeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig {
                freeze_threshold_real_s,
                countdown_duration_real_s,
            } => write!(
                f,
                "invalid outage freeze config (threshold={freeze_threshold_real_s}s, countdown={countdown_duration_real_s}s)"
            ),
            Self::ZeroIcRate => write!(f, "world IC clock rate must be >= 1"),
            Self::WorldClockFrozen {
                frozen_at_real_s,
                frozen_ic_s,
            } => write!(
                f,
                "world clock is frozen since real={frozen_at_real_s}s at ic={frozen_ic_s}s"
            ),
            Self::NotFrozen => write!(f, "world clock is not frozen"),
            Self::NotInOutageCountdown => write!(f, "world clock is not in an outage countdown"),
            Self::TimestampRegression { previous, proposed } => write!(
                f,
                "real-clock timestamp regressed from {previous}s to {proposed}s"
            ),
            Self::UnitQuarantined { unit, reason } => write!(
                f,
                "unit {unit} is quarantined for post-outage audit ({reason})"
            ),
            Self::MaxQuarantinedUnitsReached { max } => {
                write!(f, "maximum quarantined units ({max}) reached")
            }
        }
    }
}

impl std::error::Error for ClockFreezeError {}
