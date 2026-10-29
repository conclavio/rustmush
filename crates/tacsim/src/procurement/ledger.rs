//! **The procurement ledger** (doc 17h §3 M8f P7; `tacsim.procurement_ledger`): one row per
//! House purchase, the budget's memory. Keyed `{org:020}/{year}/{month:02}/{ordinal:06}` (the
//! organisation, the IC month, the purchase's ordinal in it), so one organisation's rows are
//! contiguous and a prefix scan never reaches a neighbour's. Written only by the
//! `tacsim_procurement` action ([`super::provider`]), as the House's agent.
use mush_core::module::{ModError, Registrar};
use mush_core::{CommitToken, ObjId, StateFamilyHandle};
use mush_org::OrgId;
use mush_roll::commerce::{CharterId, LicenceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::PROCUREMENT_FAMILY;

/// **An IC calendar month** (P3): the civil date's year and month of an IC instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Period {
    pub year: i64,
    pub month: u32,
}
impl Period {
    /// The month of IC second `ic` (`mush_core::clock::CivilDate`).
    #[must_use]
    pub fn of_ic(ic: i64) -> Self {
        let date = mush_core::clock::CivilDate::from_ic_seconds(ic);
        Self {
            year: date.year,
            month: date.month,
        }
    }
    /// The month before.
    #[must_use]
    pub fn previous(self) -> Self {
        if self.month <= 1 {
            Self {
                year: self.year.saturating_sub(1),
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }
    fn check(self) -> bool {
        (1..=12).contains(&self.month)
    }
}

/// Where a recorded purchase stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowPhase {
    /// Paid into escrow; counts against the month.
    Funded,
    /// The licence granted; counts.
    Granted,
    /// Refunded; counts for nothing.
    Refunded,
}

/// **One House purchase** (P7: `{ policy org, period, licence, listing, price_minor }`, with
/// its ordinal in the month, the purchase's operation, the charter the cap counts, the phase and
/// the acting agent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcurementRow {
    pub format: u16,
    pub org: OrgId,
    pub period: Period,
    pub ordinal: u32,
    pub operation: CommitToken,
    pub listing: CommitToken,
    pub charter: CharterId,
    pub licence: LicenceId,
    pub price_minor: i64,
    pub phase: RowPhase,
    pub agent: ObjId,
}
impl ProcurementRow {
    /// The row's own bounds: format 1, a month 1 to 12, a positive price, a real agent.
    ///
    /// # Errors
    ///
    /// The one sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.format != 1
            || !self.period.check()
            || self.price_minor <= 0
            || self.agent == ObjId(0)
            || self.agent == mush_core::GOD
        {
            return Err("procurement row out of bounds".into());
        }
        Ok(())
    }
    #[must_use]
    pub const fn key(&self) -> ProcurementKey {
        ProcurementKey {
            org: self.org,
            period: self.period,
            ordinal: self.ordinal,
        }
    }
    /// Whether the row counts against its month's budget (every phase but `Refunded`).
    #[must_use]
    pub fn counts(&self) -> bool {
        self.phase != RowPhase::Refunded
    }
    /// The row with `phase`, every other field kept.
    #[must_use]
    pub fn with_phase(&self, phase: RowPhase) -> Self {
        Self {
            phase,
            ..self.clone()
        }
    }
}

/// A ledger row's key: the organisation, the month and the purchase's ordinal in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProcurementKey {
    pub org: OrgId,
    pub period: Period,
    pub ordinal: u32,
}
impl ProcurementKey {
    /// `{org:020}/{year}/{month:02}/{ordinal:06}`.
    #[must_use]
    pub fn key_text(&self) -> String {
        format!(
            "{}{}/{:02}/{:06}",
            Self::prefix(self.org),
            self.period.year,
            self.period.month,
            self.ordinal
        )
    }
    /// The organisation's prefix, `{org:020}/`, which sorts before every one of its rows.
    #[must_use]
    pub fn prefix(org: OrgId) -> String {
        format!("{:020}/", org.0)
    }
    /// One month's prefix, `{org:020}/{year}/{month:02}/`, before every row of that month.
    #[must_use]
    pub fn period_prefix(org: OrgId, period: Period) -> String {
        format!("{}{}/{:02}/", Self::prefix(org), period.year, period.month)
    }
}

/// Register the family (`tacsim.procurement`).
pub(super) fn register(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<ProcurementKey, ProcurementRow>, ModError> {
    reg.state_family::<ProcurementKey, ProcurementRow, _, _, _, _>(
        crate::port::families::family_descriptor(
            PROCUREMENT_FAMILY,
            "tacsim.procurement_ledger_key.v1",
            "tacsim.procurement_ledger",
        )?,
        |key: &ProcurementKey| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        ProcurementRow::check,
        |value: &ProcurementRow| serde_json::to_value(value).map_err(|e| e.to_string()),
    )
}
