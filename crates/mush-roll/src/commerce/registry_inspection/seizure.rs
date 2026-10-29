//! **The seizure's proof and record** (doc 17h cut M8d2, S2 and S4; doc 17 §10, D-UM-42): the
//! Roll/tacsim producer proves the inspection, because the economy cannot read it. The hull's
//! **newest** [`InspectionRecord`] (the highest `seq` in its ring) must ground a seizure (an
//! ungoverned core, `governed == Some(false)`, or since M8b-2 a `grey` or `heirloom` core),
//! name the core now fitted on that hull, and be no older than
//! `[tacsim.minds].seizure_inspection_max_age_ic_days`. The proof
//! ([`mush_economy::registry_filing::seizure::InspectionProof`]) is then bound into the same
//! coordinator union as the economy's closed plan, whose check compares the claim's reason to
//! the proof's key byte for byte.
//!
//! Detection is evidence only (D-UM-42): a refusal here is the operator's word, never a
//! notice to anyone else.
use super::super::records::Record;
use super::records::{InspectionRecord, Provenance, inspection_slots};
use super::*;
use mush_core::{ActionTxn, ClockStamp, CommitToken, StateFamilyHandle};
use mush_economy::registry_filing::seizure::{InspectionProof, inspection_key};

/// `[tacsim.minds].seizure_inspection_max_age_ic_days`' default: an inspection three IC days
/// old still grounds a seizure (Q-M8-6, provisional).
pub const DEFAULT_INSPECTION_MAX_AGE_IC_DAYS: u32 = 3;
/// `[tacsim.minds].seizure_hold_ic_days`' default (Q-M8-6, provisional): a seized core may be
/// forfeited three IC days after its seizure.
pub const DEFAULT_SEIZURE_HOLD_IC_DAYS: u32 = 3;
const IC_DAY: i64 = 86_400;

/// **Why a hull's newest inspection grounds no seizure.** The operator's surface maps each to
/// its sentence; nothing else hears of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSeizable {
    /// The hull has never been inspected.
    NoInspection,
    /// The newest inspection read a governed certified (or unknown) core, or no core at all.
    NothingToSeize,
    /// The newest inspection is older than the configured age, or on another clock.
    Stale,
    /// The core the inspection named is no longer the core fitted on the hull.
    CoreMoved,
}

/// **The newest of `hull`'s inspections**: the ring slot holding the highest `seq`, read in
/// the producer's own transaction.
///
/// # Errors
/// A family read the transaction refuses.
pub fn newest(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<String, InspectionRecord>,
    hull: ObjId,
) -> Result<Option<InspectionRecord>, PublicationError> {
    let mut best: Option<InspectionRecord> = None;
    for key in inspection_slots(hull) {
        if let Some(row) = txn.get(family, &key).map_err(bad)?
            && row.hull == hull
            && best.as_ref().is_none_or(|b| row.seq > b.seq)
        {
            best = Some(row.clone());
        }
    }
    Ok(best)
}

/// **The proof** (S2) over the hull's newest inspection `record`, the core asset fitted on
/// the hull now (`fitted`, the hull's `core` component's asset) and the world's IC `now`.
///
/// # Errors
/// [`NotSeizable`], in the order the operator would want to hear it: the ground first, then
/// the age, then the core.
pub fn prove(
    record: &InspectionRecord,
    fitted: Option<ObjId>,
    now: &ClockStamp,
    max_age_ic_days: u32,
) -> Result<InspectionProof, NotSeizable> {
    // The two grounds: an ungoverned core, or (doc 17h cut M8b-2, now landed) a core whose
    // provenance is `grey` or `heirloom`, governed or not. A coreless record grounds nothing.
    match (record.governed, &record.provenance) {
        (Some(false), _) | (Some(true), Provenance::Grey | Provenance::Heirloom) => {}
        (Some(true), Provenance::Certified { .. } | Provenance::Unknown) | (None, _) => {
            return Err(NotSeizable::NothingToSeize);
        }
    }
    let max_age = i64::from(max_age_ic_days)
        .checked_mul(IC_DAY)
        .ok_or(NotSeizable::Stale)?;
    let age = now
        .at
        .checked_sub(record.at_ic.at)
        .ok_or(NotSeizable::Stale)?;
    if now.clock != record.at_ic.clock || age < 0 || age > max_age {
        return Err(NotSeizable::Stale);
    }
    let core = record.core.ok_or(NotSeizable::NothingToSeize)?;
    if fitted != Some(core) {
        return Err(NotSeizable::CoreMoved);
    }
    let proof = InspectionProof {
        key: inspection_key(record.hull, record.seq),
        hull: record.hull,
        core,
        seq: record.seq,
    };
    if !proof.well_formed() {
        return Err(NotSeizable::NothingToSeize);
    }
    Ok(proof)
}

/// **Whether a seizure made at IC second `seized_at` may be forfeited at `now`** (S5): the
/// hold (`[tacsim.minds].seizure_hold_ic_days`) has fully run. On another clock it has not.
#[must_use]
pub fn hold_elapsed(seized_at: &ClockStamp, now: &ClockStamp, hold_ic_days: u32) -> bool {
    let Some(hold) = i64::from(hold_ic_days).checked_mul(IC_DAY) else {
        return false;
    };
    now.clock == seized_at.clock
        && seized_at
            .at
            .checked_add(hold)
            .is_some_and(|until| now.at >= until)
}

/// The seizure record family.
pub const SEIZURE_FAMILY: &str = "roll_registry_seizures";

/// **One seizure** (doc 17h cut M8d2 S4/S5), keyed by the inspection that grounded it
/// (`hull/<8 hex>/inspection/<seq>`): who seized the core fitted on `hull`, when, the economy
/// seizure's own commit, and when the hold ends; a forfeit fills the three `forfeit` fields and
/// nothing else. The core stays fitted (the lead's "impounded in place"); the seal reads it as
/// no core from its title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeizureRecord {
    pub format: u16,
    /// The grounding inspection's commit, hull and sequence.
    pub inspection: CommitToken,
    pub hull: ObjId,
    pub seq: u64,
    /// The core asset seized (the inspection's fitted core).
    pub core: ObjId,
    /// The seizure's commit: the economy seizure receipt's key.
    pub seizure: CommitToken,
    /// The seizure's World IC second, and the first second a forfeit may run.
    pub at_ic: i64,
    pub forfeit_after_ic: i64,
    pub seized_by: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forfeit: Option<CommitToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forfeited_at_ic: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forfeited_by: Option<ObjId>,
}

impl SeizureRecord {
    /// The record as seized: the forfeit fields empty.
    #[must_use]
    pub fn seized(&self) -> Self {
        Self {
            forfeit: None,
            forfeited_at_ic: None,
            forfeited_by: None,
            ..self.clone()
        }
    }
    /// Whether the core was forfeited.
    #[must_use]
    pub const fn forfeited(&self) -> bool {
        self.forfeited_by.is_some()
    }
}

impl Record for SeizureRecord {
    const FAMILY: &'static str = SEIZURE_FAMILY;
    const MAX_BYTES: usize = 2048;
    fn key(&self) -> String {
        inspection_key(self.hull, self.seq)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        let forfeit_ok = match (self.forfeit, self.forfeited_at_ic, self.forfeited_by) {
            (None, None, None) => true,
            (Some(commit), Some(at), Some(by)) => {
                commit != self.seizure && at >= self.forfeit_after_ic && by.0 != 0
            }
            _ => false,
        };
        if self.format != 1
            || self.hull.0 == 0
            || self.core.0 == 0
            || self.hull == self.core
            || self.seq == 0
            || self.seized_by.0 == 0
            || self.seizure == self.inspection
            || self.forfeit_after_ic < self.at_ic
            || !forfeit_ok
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
