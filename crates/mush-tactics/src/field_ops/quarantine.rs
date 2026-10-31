//! `P-4` / `Q-H11` — Post-outage audit quarantine for in-flight field units, a consumer of
//! the kernel `W-clock` (design doc 27 `FO-11`, `Q-H11`, `P-4`).
//!
//! The shared world IC clock — its durable checkpoint, the outage evaluation at boot, the
//! recovery countdown and its broadcasts — is one implementation, the kernel's
//! (`mush_core::clock::WorldTimeline`, `mush-server`'s boot policy and world task). This
//! module holds the one thing the kernel does not: the record of a field unit whose persisted
//! state disagreed with its site books when the world came back, kept for staff audit instead
//! of being reset to stock. It is driven by the kernel's
//! [`WorldEvent::WorldClockResumed`] event — every record is stamped from that event — the way
//! the economy extends its real-time contract deadlines from the same events.

use std::collections::BTreeMap;
use std::fmt;

use mush_core::{ObjId, WorldEvent};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::FieldCustodyLedger;
use crate::field_ops::engagement::FieldSiteEngagementBook;
use crate::unit::TacticalUnit;

/// Maximum quarantined unit records retained in one [`OutageQuarantineBook`].
pub const MAX_QUARANTINED_UNITS: usize = 512;

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
    /// Real-time UTC second when the unit was quarantined (the kernel's `resumed_real_unix`).
    pub quarantined_at_real_s: i64,
    /// World-IC second when the unit was quarantined (the kernel's resumed `ic_unix`, which is
    /// the frozen IC second: the clock does not move across an outage).
    pub quarantined_at_ic_s: i64,
}

/// Post-outage audit quarantine (`Q-H11`, `P-4`), a consumer of the kernel `W-clock`.
///
/// Holds the units that failed their consistency audit when the world clock resumed after an
/// outage, each with its preserved revision, until staff release them. It keeps no clock of
/// its own: whether the world is frozen is `World::world_timeline().is_frozen()`, and the
/// moment of the audit is the kernel's [`WorldEvent::WorldClockResumed`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutageQuarantineBook {
    units: BTreeMap<ObjId, QuarantinedUnitRecord>,
}

impl OutageQuarantineBook {
    /// Construct an empty [`OutageQuarantineBook`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All units currently quarantined for post-outage audit (`Q-H11`).
    #[must_use]
    pub const fn quarantined_units(&self) -> &BTreeMap<ObjId, QuarantinedUnitRecord> {
        &self.units
    }

    /// Gate that refuses active combat or recovery actions on a unit held in post-outage
    /// audit quarantine (`Q-H11`).
    ///
    /// # Errors
    /// Returns [`QuarantineError::UnitQuarantined`] if `unit` is quarantined.
    pub fn ensure_unit_not_quarantined(&self, unit: ObjId) -> Result<(), QuarantineError> {
        if let Some(rec) = self.units.get(&unit) {
            return Err(QuarantineError::UnitQuarantined {
                unit,
                reason: rec.reason.clone(),
            });
        }
        Ok(())
    }

    /// Audit every unit of a site when the kernel resumes the world clock after an outage
    /// (`Q-H11`, `P-4`).
    ///
    /// For a [`WorldEvent::WorldClockResumed`] event, runs
    /// [`Self::audit_or_quarantine_site_unit`] over `units` with the event's
    /// `resumed_real_unix` and `ic_unix` as the stamps and returns the records quarantined.
    /// Any other event audits nothing: quarantine is decided when the world comes back, never
    /// during the countdown.
    ///
    /// # Errors
    /// Returns [`QuarantineError::MaxQuarantinedUnitsReached`] if the quarantine table fills;
    /// units audited before the full one are quarantined.
    pub fn audit_site_units_on_world_clock_resumed<'u>(
        &mut self,
        event: &WorldEvent,
        site_book: &FieldSiteEngagementBook,
        ledger: &FieldCustodyLedger,
        units: impl IntoIterator<Item = &'u TacticalUnit>,
    ) -> Result<Vec<QuarantinedUnitRecord>, QuarantineError> {
        let WorldEvent::WorldClockResumed {
            resumed_real_unix,
            ic_unix,
            ..
        } = event
        else {
            return Ok(Vec::new());
        };
        let mut quarantined = Vec::new();
        for unit in units {
            if let Some(record) = self.audit_or_quarantine_site_unit(
                site_book,
                ledger,
                unit,
                *resumed_real_unix,
                *ic_unix,
            )? {
                quarantined.push(record);
            }
        }
        Ok(quarantined)
    }

    /// Audit one active site unit after an outage against the [`FieldCustodyLedger`] and its
    /// persisted [`TacticalUnit`] revision (`Q-H11`).
    ///
    /// If the unit's revision on `site_book` or `ledger` is incompatible with
    /// `unit.revision()`, or its custody record is missing, the unit is quarantined for
    /// audited recovery rather than reset to stock (`Q-H11`). `at_real_s` and `at_ic_s` are
    /// the kernel's stamps for the moment of the audit (`WorldClockResumed`'s
    /// `resumed_real_unix` and `ic_unix`).
    ///
    /// Returns `Ok(None)` if the unit is consistent or not present on the site, or
    /// `Ok(Some(QuarantinedUnitRecord))` if it was quarantined.
    ///
    /// # Errors
    /// Returns [`QuarantineError::MaxQuarantinedUnitsReached`] if the quarantine table is full.
    pub fn audit_or_quarantine_site_unit(
        &mut self,
        site_book: &FieldSiteEngagementBook,
        ledger: &FieldCustodyLedger,
        unit: &TacticalUnit,
        at_real_s: i64,
        at_ic_s: i64,
    ) -> Result<Option<QuarantinedUnitRecord>, QuarantineError> {
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

        if !self.units.contains_key(&unit_id) && self.units.len() >= MAX_QUARANTINED_UNITS {
            return Err(QuarantineError::MaxQuarantinedUnitsReached {
                max: MAX_QUARANTINED_UNITS,
            });
        }

        let record = QuarantinedUnitRecord {
            unit: unit_id,
            site: site_book.site(),
            reason,
            preserved_revision: unit.revision(),
            quarantined_at_real_s: at_real_s,
            quarantined_at_ic_s: at_ic_s,
        };
        self.units.insert(unit_id, record.clone());
        Ok(Some(record))
    }

    /// Release a unit from post-outage audit quarantine once staff/audit reconciliation
    /// completes.
    pub fn release_quarantined_unit(&mut self, unit: ObjId) -> Option<QuarantinedUnitRecord> {
        self.units.remove(&unit)
    }
}

/// Errors returned by [`OutageQuarantineBook`] operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuarantineError {
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

impl fmt::Display for QuarantineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

impl std::error::Error for QuarantineError {}
