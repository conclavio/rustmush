//! E7c3 B2/B3/B4/B6 — the records, their caps, and their `TryFrom` wires.
//!
//! Every persisted type below deserializes through a wire struct that **re-runs
//! the constructor invariants** (§4 rule 1, D-E7-33): a derived `Deserialize`
//! would make every cap in this file advisory on load, which is the E5a finding
//! this programme is not allowed to re-learn.
//!
//! # What is *not* here
//!
//! No reserve scalar, no book, no second quantity carrier: a deposit's
//! unextracted commodity is **cut 2's pool at the deposit's own endpoint**
//! (`OPEN 1`, D-E7-3), so `mush-industry` keeps no private stock number.
//! [`Grade`] is cut 2's and is used, never redefined. `ExternalRef`,
//! `ContractId` and `Encumbrance` are the economy's.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, ObjId, OwnerId};
// The economic principal, not `mush_core::PrincipalId`: an issuer is a government
// (`Organization`/`System`) and a holder may be an org. Every economy seam cut 3
// calls — `issue_title`, `accounts_of`, `Encumbrance` — is typed in this one.
use mush_economy::{ContractId, ExternalRef, principal_token};
use mush_inventory::{EndpointRef, Grade};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use thiserror::Error;

/// This crate's owner namespace. One `OwnerId`, one fact namespace, one custody
/// provider name.
pub const NS: &str = "industry";

/// The crate's `OwnerId`.
///
/// # Panics
/// Never: `industry` is a canonical owner id, checked once here.
#[must_use]
pub fn owner() -> OwnerId {
    OwnerId::new(NS).expect("static industry owner id is canonical")
}

// ---- E7c3: caps (§7, plus this cut's own) ----

/// Largest number of ore bodies one place may carry.
///
/// **Never `MAX_DEPOSITS`** — `mush-economy`'s `MAX_DEPOSITS` is 1 and counts
/// *custody* deposits per contract plan (`crates/mush-economy/src/contract.rs`,
/// E4b2 escrow). 10b §3.3's "neither name may drift into the other crate" is
/// discharged by this rename, not by a comment.
pub const MAX_ORE_DEPOSITS_PER_PLACE: usize = 8;
/// §7: rows in one deposit's per-principal survey disclosure map.
pub const MAX_SURVEY_BANDS: usize = 8;
/// §7: live claims over one subject.
pub const MAX_CLAIMS_PER_SUBJECT: usize = 8;
/// Points in one [`YieldCurve`]. Bounded so evaluation is a bounded walk.
pub const MAX_CURVE_POINTS: usize = 16;
/// Accessibility is a percentage of the curve's yield, `0..=100`.
pub const MAX_ACCESSIBILITY: u8 = 100;
/// Largest authored initial reserve. Inside cut 2's `MAX_POOL_QUANTITY`.
pub const MAX_ORE_RESERVE: u64 = 1_000_000_000;
/// Largest per-cycle yield a curve point may name. Inside `MAX_LOT_QUANTITY`.
pub const MAX_CYCLE_YIELD: u64 = 1_000_000;
/// Largest cycle ordinal one run may reach.
pub const MAX_EXTRACTION_CYCLES: u64 = 1_000_000;
/// Largest band token, in bytes.
pub const MAX_BAND_BYTES: usize = 32;
/// Largest claim priority.
pub const MAX_CLAIM_PRIORITY: u32 = 1_000;
/// Largest dispute/seizure reason, in bytes.
pub const MAX_REASON_BYTES: usize = 240;
/// Records one bounded page visits.
pub const INDUSTRY_PAGE: usize = 32;
/// Pages one bounded listing walk visits (D-LBK-5: bounded on records
/// **visited**, never on records matched).
pub const MAX_LIST_PAGES: usize = 128;
/// Rows one listing renders.
pub const MAX_LIST_ROWS: usize = 64;
/// Applied markers the RAM mirror keeps for the fact provider.
pub const MAX_APPLIED_MARKERS: usize = 4_096;
/// Pages the marker mirror's own walk visits.
pub const MAX_MARKER_PAGES: usize = 256;
/// Largest applied-marker id, in bytes.
pub const MAX_MARKER_ID_BYTES: usize = 96;

/// The provider name this crate registers its `CustodyRef::Registered` backings
/// under. E3's rules are unchanged: exactly one claiming provider.
pub const CUSTODY_PROVIDER: &str = NS;
/// `industry.extraction.cycled` (§10, D-E7-30).
pub const FACT_EXTRACTION_CYCLED: &str = "industry.extraction.cycled";
/// `industry.workorder.completed` — **cut 4's arm on the same provider**. Named
/// here so cut 4 adds no second namespace, and answered `None` today.
pub const FACT_WORKORDER_COMPLETED: &str = "industry.workorder.completed";

// ---- E7c3: identities ----

macro_rules! industry_id {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($prefix, "#{:016x}"), self.0)
            }
        }

        impl $name {
            /// Parse the rendered form (`prefix#<16 hex>`), or a bare hex id.
            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                let body = text
                    .trim()
                    .strip_prefix(concat!($prefix, "#"))
                    .unwrap_or_else(|| text.trim());
                (body.len() == 16)
                    .then(|| u64::from_str_radix(body, 16).ok())
                    .flatten()
                    .map(Self)
            }
        }
    };
}

industry_id!(
    /// One ore body. **Never `DepositId`**: `mush-economy` owns the word
    /// "deposit" for E4b2 custody deposits.
    OreDepositId,
    "deposit"
);
industry_id!(
    /// One tenure record.
    ClaimId,
    "claim"
);

/// A disclosure band: an opaque recorded class, never a quantity.
///
/// **Knowledge, not truth.** A survey copies the deposit's *recorded*
/// `grade_class`/`reserve_class` verbatim, so repeated surveys cannot narrow a
/// band and two principals' bands are the same two `BandId`s. The band table
/// itself is the ruleset's, authored in a pack.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BandId(SmolStr);

#[derive(Deserialize)]
#[serde(transparent)]
struct BandIdWire(SmolStr);

impl<'de> Deserialize<'de> for BandId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BandIdWire::deserialize(deserializer)?;
        Self::new(wire.0.as_str()).map_err(serde::de::Error::custom)
    }
}

impl BandId {
    /// Build a band id: `1..=`[`MAX_BAND_BYTES`] bytes of `[a-z0-9_.-]`.
    ///
    /// # Errors
    /// [`IndustryRecordError::Band`] for an empty, oversized or non-canonical
    /// token.
    pub fn new(text: &str) -> Result<Self, IndustryRecordError> {
        if text.is_empty() || text.len() > MAX_BAND_BYTES {
            return Err(IndustryRecordError::Band);
        }
        if !text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        }) {
            return Err(IndustryRecordError::Band);
        }
        Ok(Self(SmolStr::new(text)))
    }

    /// The token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for BandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// How reachable the ore body is, as a percentage of the curve's yield.
///
/// Moved only by an **attributed journaled campaign event** and only for the
/// *next* cycle: a completed cycle is never re-evaluated (D-E7-6). Zero is the
/// collapsed/suspended state — the cycle refuses rather than yielding nothing,
/// so an operator is told.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Accessibility(u8);

#[derive(Deserialize)]
#[serde(transparent)]
struct AccessibilityWire(u8);

impl<'de> Deserialize<'de> for Accessibility {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AccessibilityWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl Accessibility {
    /// Build an accessibility.
    ///
    /// # Errors
    /// [`IndustryRecordError::Accessibility`] above [`MAX_ACCESSIBILITY`].
    pub const fn new(value: u8) -> Result<Self, IndustryRecordError> {
        if value > MAX_ACCESSIBILITY {
            return Err(IndustryRecordError::Accessibility {
                max: MAX_ACCESSIBILITY,
            });
        }
        Ok(Self(value))
    }

    /// The percentage.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Whether the body can be worked at all right now.
    #[must_use]
    pub const fn is_workable(self) -> bool {
        self.0 > 0
    }
}

/// One step of a [`YieldCurve`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CurvePoint {
    /// Cumulative extraction at or above which this step applies.
    pub cumulative: u64,
    /// Yield per cycle in that band, before accessibility.
    pub per_cycle: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurvePointWire {
    cumulative: u64,
    per_cycle: u64,
}

impl<'de> Deserialize<'de> for CurvePoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CurvePointWire::deserialize(deserializer)?;
        if wire.per_cycle > MAX_CYCLE_YIELD {
            return Err(serde::de::Error::custom(IndustryRecordError::CycleYield {
                max: MAX_CYCLE_YIELD,
            }));
        }
        Ok(Self {
            cumulative: wire.cumulative,
            per_cycle: wire.per_cycle,
        })
    }
}

/// A bounded monotone table mapping **cumulative extraction** to per-cycle yield.
///
/// Evaluated by a pure function with checked integer arithmetic in a fixed
/// order: **no clock, no world, no RNG**, so two evaluations always agree and a
/// replay is a HIT. Depletion is arithmetic, never a roll (D-E7-6, Q1A).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct YieldCurve {
    points: Vec<CurvePoint>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct YieldCurveWire(Vec<CurvePoint>);

impl<'de> Deserialize<'de> for YieldCurve {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = YieldCurveWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl YieldCurve {
    /// Build a curve.
    ///
    /// # Errors
    /// [`IndustryRecordError::Curve`] for an empty or oversized table, a first
    /// point that does not start at zero, a non-increasing `cumulative`, a
    /// yield that rises with depletion, or a yield past [`MAX_CYCLE_YIELD`].
    pub fn new(points: Vec<CurvePoint>) -> Result<Self, IndustryRecordError> {
        if points.is_empty() || points.len() > MAX_CURVE_POINTS {
            return Err(IndustryRecordError::Curve);
        }
        if points[0].cumulative != 0 {
            return Err(IndustryRecordError::Curve);
        }
        let mut previous: Option<CurvePoint> = None;
        for point in &points {
            if point.per_cycle > MAX_CYCLE_YIELD {
                return Err(IndustryRecordError::CycleYield {
                    max: MAX_CYCLE_YIELD,
                });
            }
            if let Some(previous) = previous {
                // Strictly increasing thresholds, and yield never *rises* as the
                // body depletes: monotone is what makes "the curve" a statement
                // rather than a table anyone can read a jackpot out of.
                if point.cumulative <= previous.cumulative || point.per_cycle > previous.per_cycle {
                    return Err(IndustryRecordError::Curve);
                }
            }
            previous = Some(*point);
        }
        Ok(Self { points })
    }

    /// The table.
    #[must_use]
    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// Per-cycle yield at `extracted` cumulative units, before accessibility.
    ///
    /// A bounded walk over at most [`MAX_CURVE_POINTS`] entries in a fixed
    /// order. Pure: same inputs, same answer, for ever.
    #[must_use]
    pub fn at(&self, extracted: u64) -> u64 {
        let mut yielded = 0;
        for point in &self.points {
            if point.cumulative > extracted {
                break;
            }
            yielded = point.per_cycle;
        }
        yielded
    }
}

// ---- E7c3 B2: the deposit ----

/// One ore body: the **definition half** plus the endpoint its remaining
/// quantity lives at.
///
/// The remaining reserve is **not a field**. It is cut 2's `Pool` at
/// [`OreDeposit::endpoint`], so there is exactly one quantity authority and no
/// arithmetic of this crate's own (D-E7-3, OPEN 1). `initial_reserve` is the
/// *authored* figure the pool was seeded with and is what makes cumulative
/// extraction derivable — `initial_reserve - pool.quantity` — without a second
/// counter; it is staff-only and never rendered to a surveyor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OreDeposit {
    id: OreDepositId,
    place: ObjId,
    commodity: CanonicalId,
    grade: Grade,
    accessibility: Accessibility,
    grade_class: BandId,
    reserve_class: BandId,
    curve: YieldCurve,
    endpoint: EndpointRef,
    initial_reserve: u64,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OreDepositWire {
    id: OreDepositId,
    place: ObjId,
    commodity: CanonicalId,
    grade: Grade,
    accessibility: Accessibility,
    grade_class: BandId,
    reserve_class: BandId,
    curve: YieldCurve,
    endpoint: EndpointRef,
    initial_reserve: u64,
    revision: u64,
}

impl<'de> Deserialize<'de> for OreDeposit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = OreDepositWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.place,
            wire.commodity,
            wire.grade,
            wire.accessibility,
            wire.grade_class,
            wire.reserve_class,
            wire.curve,
            wire.endpoint,
            wire.initial_reserve,
            wire.revision,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl OreDeposit {
    /// Build a deposit.
    ///
    /// # Errors
    /// [`IndustryRecordError`] for a zero or oversized reserve, or a zero
    /// revision.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: OreDepositId,
        place: ObjId,
        commodity: CanonicalId,
        grade: Grade,
        accessibility: Accessibility,
        grade_class: BandId,
        reserve_class: BandId,
        curve: YieldCurve,
        endpoint: EndpointRef,
        initial_reserve: u64,
    ) -> Result<Self, IndustryRecordError> {
        Self::from_parts(
            id,
            place,
            commodity,
            grade,
            accessibility,
            grade_class,
            reserve_class,
            curve,
            endpoint,
            initial_reserve,
            1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: OreDepositId,
        place: ObjId,
        commodity: CanonicalId,
        grade: Grade,
        accessibility: Accessibility,
        grade_class: BandId,
        reserve_class: BandId,
        curve: YieldCurve,
        endpoint: EndpointRef,
        initial_reserve: u64,
        revision: u64,
    ) -> Result<Self, IndustryRecordError> {
        if initial_reserve == 0 || initial_reserve > MAX_ORE_RESERVE {
            return Err(IndustryRecordError::Reserve {
                max: MAX_ORE_RESERVE,
            });
        }
        if revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        Ok(Self {
            id,
            place,
            commodity,
            grade,
            accessibility,
            grade_class,
            reserve_class,
            curve,
            endpoint,
            initial_reserve,
            revision,
        })
    }

    /// Identity.
    #[must_use]
    pub const fn id(&self) -> OreDepositId {
        self.id
    }
    /// The seeded world location it sits at (OPEN 4: an `ObjId`, never a
    /// `RegionId` — `RegionId` is `mush-faction`'s runtime-allocated id and a
    /// pack cannot author it, and using it would put a faction edge on a crate
    /// that must stay neutral).
    #[must_use]
    pub const fn place(&self) -> ObjId {
        self.place
    }
    /// What comes out of it.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }
    /// The quality of what comes out (cut 2's [`Grade`], never redefined).
    #[must_use]
    pub const fn grade(&self) -> Grade {
        self.grade
    }
    /// How workable it is right now.
    #[must_use]
    pub const fn accessibility(&self) -> Accessibility {
        self.accessibility
    }
    /// The recorded grade band a survey copies verbatim.
    #[must_use]
    pub const fn grade_class(&self) -> &BandId {
        &self.grade_class
    }
    /// The recorded reserve band a survey copies verbatim.
    #[must_use]
    pub const fn reserve_class(&self) -> &BandId {
        &self.reserve_class
    }
    /// The depletion curve.
    #[must_use]
    pub const fn curve(&self) -> &YieldCurve {
        &self.curve
    }
    /// Cut 2's endpoint the remaining quantity pools at.
    #[must_use]
    pub const fn endpoint(&self) -> EndpointRef {
        self.endpoint
    }
    /// The authored reserve the pool was seeded with. **Staff-only.**
    #[must_use]
    pub const fn initial_reserve(&self) -> u64 {
        self.initial_reserve
    }
    /// Staleness stamp.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Cumulative extraction, given the endpoint pool's current balance.
    ///
    /// Saturating on purpose: a balance *above* the authored reserve means
    /// somebody credited the deposit endpoint from outside, which is not
    /// negative extraction.
    #[must_use]
    pub const fn extracted(&self, remaining: u64) -> u64 {
        self.initial_reserve.saturating_sub(remaining)
    }

    /// The next cycle's yield: the curve at the current cumulative extraction,
    /// scaled by accessibility, then clamped to what is actually left.
    ///
    /// Pure, checked, fixed order. An accessibility change therefore moves the
    /// **next** cycle and can never re-evaluate a completed one.
    #[must_use]
    pub fn next_yield(&self, remaining: u64) -> u64 {
        let base = self.curve.at(self.extracted(remaining));
        let scaled = base
            .checked_mul(u64::from(self.accessibility.get()))
            .map_or(0, |scaled| scaled / u64::from(MAX_ACCESSIBILITY));
        // E7c3 verify fold C3: a **workable, non-empty** body whose curve still
        // yields (`base > 0`) must extract at least one unit. Without this, a
        // low accessibility scaling (`base * accessibility < MAX_ACCESSIBILITY`)
        // floors to 0 and the body reports `Depleted` while matter remains — the
        // reserve is stranded and the refusal mislabelled. A body the curve has
        // genuinely exhausted (`base == 0`) or an unworkable one (`accessibility
        // == 0`) still yields 0 and refuses honestly.
        let scaled = if scaled == 0 && base > 0 && self.accessibility.is_workable() {
            1
        } else {
            scaled
        };
        scaled.min(remaining)
    }

    /// The post-image of a campaign event that moves accessibility.
    ///
    /// # Errors
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn with_accessibility(
        &self,
        accessibility: Accessibility,
    ) -> Result<Self, IndustryRecordError> {
        let mut next = self.clone();
        next.accessibility = accessibility;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)?;
        Ok(next)
    }
}

// ---- E7c3 B3: survey disclosure ----

/// One principal's disclosure row over one deposit.
///
/// It copies the deposit's **recorded** classes verbatim. It is never `reserve`
/// re-quantised per viewer, which is what makes repeated surveys unable to
/// narrow a band and two principals' bands identical.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyBands {
    /// The deposit's recorded grade class.
    pub grade_class: BandId,
    /// The deposit's recorded reserve class.
    pub reserve_class: BandId,
    /// When it was disclosed.
    pub at: ClockStamp,
    /// The journal entry that disclosed it.
    pub fact: ExternalRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SurveyBandsWire {
    grade_class: BandId,
    reserve_class: BandId,
    at: ClockStamp,
    fact: ExternalRef,
}

impl<'de> Deserialize<'de> for SurveyBands {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SurveyBandsWire::deserialize(deserializer)?;
        Ok(Self {
            grade_class: wire.grade_class,
            reserve_class: wire.reserve_class,
            at: wire.at,
            fact: wire.fact,
        })
    }
}

/// One survey row on the wire: the principal and what it was told.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyRow {
    /// Who was told.
    pub principal: PrincipalId,
    /// What they were told.
    pub bands: SurveyBands,
}

/// One deposit's whole disclosure document — **pure disclosure**.
///
/// Filing a claim does not require a survey and **no legality check reads a
/// band**, which is exactly what keeps this record exempt under §4.1's closing
/// rule. It is the one conventional document cut 3 owns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyDoc {
    deposit: OreDepositId,
    rows: Vec<SurveyRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SurveyDocWire {
    deposit: OreDepositId,
    rows: Vec<SurveyRow>,
}

impl<'de> Deserialize<'de> for SurveyDoc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SurveyDocWire::deserialize(deserializer)?;
        Self::from_rows(wire.deposit, wire.rows).map_err(serde::de::Error::custom)
    }
}

impl SurveyDoc {
    /// An empty disclosure document.
    #[must_use]
    pub const fn empty(deposit: OreDepositId) -> Self {
        Self {
            deposit,
            rows: Vec::new(),
        }
    }

    /// Rebuild from rows.
    ///
    /// # Errors
    /// [`IndustryRecordError::SurveyBandCap`] past [`MAX_SURVEY_BANDS`] — the
    /// map is **refused on load, never truncated**, because a silently dropped
    /// row is a principal who was told something the record then denies —
    /// and [`IndustryRecordError::NonCanonicalRows`] for an unsorted or
    /// duplicated principal.
    pub fn from_rows(
        deposit: OreDepositId,
        rows: Vec<SurveyRow>,
    ) -> Result<Self, IndustryRecordError> {
        if rows.len() > MAX_SURVEY_BANDS {
            return Err(IndustryRecordError::SurveyBandCap {
                max: MAX_SURVEY_BANDS,
            });
        }
        // `mush_org::PrincipalId` has no `Ord`, so the canonical order is the
        // economy's own `principal_token` — the same total order a consent
        // audience is canonicalized by, never a second one.
        let mut previous: Option<smol_str::SmolStr> = None;
        for row in &rows {
            let token = principal_token(&row.principal);
            if previous.as_ref().is_some_and(|previous| previous >= &token) {
                return Err(IndustryRecordError::NonCanonicalRows);
            }
            previous = Some(token);
        }
        Ok(Self { deposit, rows })
    }

    /// Which deposit it discloses.
    #[must_use]
    pub const fn deposit(&self) -> OreDepositId {
        self.deposit
    }

    /// The rows.
    #[must_use]
    pub fn rows(&self) -> &[SurveyRow] {
        &self.rows
    }

    /// One principal's row, if they have surveyed.
    #[must_use]
    pub fn bands_for(&self, principal: &PrincipalId) -> Option<&SurveyBands> {
        self.rows
            .iter()
            .find(|row| &row.principal == principal)
            .map(|row| &row.bands)
    }

    /// The per-principal map, keyed by the economy's canonical principal token.
    #[must_use]
    pub fn as_map(&self) -> BTreeMap<smol_str::SmolStr, SurveyBands> {
        self.rows
            .iter()
            .map(|row| (principal_token(&row.principal), row.bands.clone()))
            .collect()
    }

    /// Record one principal's disclosure. A re-survey by the same principal
    /// **rewrites nothing**: the row already carries the recorded classes, so
    /// the answer cannot narrow (F lens).
    ///
    /// # Errors
    /// [`IndustryRecordError::SurveyBandCap`] when a *new* principal would push
    /// the map past [`MAX_SURVEY_BANDS`].
    pub fn disclosed(
        &self,
        principal: &PrincipalId,
        bands: SurveyBands,
    ) -> Result<(Self, bool), IndustryRecordError> {
        if self.bands_for(principal).is_some() {
            return Ok((self.clone(), false));
        }
        if self.rows.len() >= MAX_SURVEY_BANDS {
            return Err(IndustryRecordError::SurveyBandCap {
                max: MAX_SURVEY_BANDS,
            });
        }
        let mut rows = self.rows.clone();
        rows.push(SurveyRow {
            principal: principal.clone(),
            bands,
        });
        rows.sort_by_key(|row| principal_token(&row.principal));
        Ok((Self::from_rows(self.deposit, rows)?, true))
    }
}

// ---- E7c3 B4: tenure ----

/// What a claim is over.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimSubject {
    /// One named ore body.
    Deposit {
        /// The body.
        deposit: OreDepositId,
    },
    /// A whole place. Two governments may each issue one over the same place —
    /// that needs only two [`ClaimRecord`]s with distinct issuers, never a
    /// `ControlKind` (which stays cut 9's).
    Place {
        /// The location.
        place: ObjId,
    },
}

/// Whether the issuer may issue a second claim over the same subject.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusivity {
    /// One live claim per issuer per subject.
    Exclusive,
    /// Several may coexist under one issuer.
    Shared,
}

/// Whether tenure may change hands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transferability {
    /// The titled object may be transferred (E4's `AssetOp::TransferTitle`).
    Transferable,
    /// Tenure is personal to the holder.
    Bound,
}

/// Where a claim stands in a dispute.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContestState {
    /// Nobody is contesting it.
    Uncontested,
    /// One E4b2 contract with a `Before` deadline and a named adjudicator is
    /// open over it.
    Contested {
        /// The adjudicating contract.
        contract: ContractId,
    },
    /// The issuer suspended it. The record stays; it authorizes nothing.
    Suspended {
        /// Who suspended it.
        by: PrincipalId,
    },
    /// The issuer seized it, attributed and stamped.
    Seized {
        /// Who seized it.
        by: PrincipalId,
        /// When.
        at: ClockStamp,
    },
}

impl ContestState {
    /// Whether this state lets the claim authorize anything at all.
    #[must_use]
    pub const fn authorizes(&self) -> bool {
        matches!(self, Self::Uncontested | Self::Contested { .. })
    }
}

/// Tenure: **a record**, whose transferable half is a title on one world object.
///
/// Authorization is a *separate* thing — the acting principal's
/// `InstrumentKind::Entitlement` licence, checked at transaction time (Q1A/Q14A,
/// D-E7-7). The two are never conflated. Transfer, pledge and dispute are E4's,
/// reused verbatim on [`ClaimRecord::object`]: `AssetOp::TransferTitle`,
/// `Encumbrance::Collateral`, `Encumbrance::Disputed`. Cut 3 defines **no**
/// encumbrance model, **no** second title record and **no** second transfer
/// path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRecord {
    id: ClaimId,
    subject: ClaimSubject,
    issuer: PrincipalId,
    holder: PrincipalId,
    object: ObjId,
    exclusivity: Exclusivity,
    priority: u32,
    term: ClockStamp,
    transferability: Transferability,
    contest_state: ContestState,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimRecordWire {
    id: ClaimId,
    subject: ClaimSubject,
    issuer: PrincipalId,
    holder: PrincipalId,
    object: ObjId,
    exclusivity: Exclusivity,
    priority: u32,
    term: ClockStamp,
    transferability: Transferability,
    contest_state: ContestState,
    revision: u64,
}

impl<'de> Deserialize<'de> for ClaimRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ClaimRecordWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.subject,
            wire.issuer,
            wire.holder,
            wire.object,
            wire.exclusivity,
            wire.priority,
            wire.term,
            wire.transferability,
            wire.contest_state,
            wire.revision,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ClaimRecord {
    /// File a claim.
    ///
    /// # Errors
    /// [`IndustryRecordError`] for a priority past [`MAX_CLAIM_PRIORITY`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ClaimId,
        subject: ClaimSubject,
        issuer: PrincipalId,
        holder: PrincipalId,
        object: ObjId,
        exclusivity: Exclusivity,
        priority: u32,
        term: ClockStamp,
        transferability: Transferability,
    ) -> Result<Self, IndustryRecordError> {
        Self::from_parts(
            id,
            subject,
            issuer,
            holder,
            object,
            exclusivity,
            priority,
            term,
            transferability,
            ContestState::Uncontested,
            1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: ClaimId,
        subject: ClaimSubject,
        issuer: PrincipalId,
        holder: PrincipalId,
        object: ObjId,
        exclusivity: Exclusivity,
        priority: u32,
        term: ClockStamp,
        transferability: Transferability,
        contest_state: ContestState,
        revision: u64,
    ) -> Result<Self, IndustryRecordError> {
        if priority > MAX_CLAIM_PRIORITY {
            return Err(IndustryRecordError::Priority {
                max: MAX_CLAIM_PRIORITY,
            });
        }
        if revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        Ok(Self {
            id,
            subject,
            issuer,
            holder,
            object,
            exclusivity,
            priority,
            term,
            transferability,
            contest_state,
            revision,
        })
    }

    /// Identity.
    #[must_use]
    pub const fn id(&self) -> ClaimId {
        self.id
    }
    /// What it is over.
    #[must_use]
    pub const fn subject(&self) -> ClaimSubject {
        self.subject
    }
    /// Which authority issued it.
    #[must_use]
    pub const fn issuer(&self) -> &PrincipalId {
        &self.issuer
    }
    /// Who holds it.
    #[must_use]
    pub const fn holder(&self) -> &PrincipalId {
        &self.holder
    }
    /// The titled world object that carries transfer, pledge and dispute.
    #[must_use]
    pub const fn object(&self) -> ObjId {
        self.object
    }
    /// Whether the issuer may issue a second.
    #[must_use]
    pub const fn exclusivity(&self) -> Exclusivity {
        self.exclusivity
    }
    /// Priority against other claims over the same subject.
    #[must_use]
    pub const fn priority(&self) -> u32 {
        self.priority
    }
    /// When it lapses.
    #[must_use]
    pub const fn term(&self) -> &ClockStamp {
        &self.term
    }
    /// Whether it may change hands.
    #[must_use]
    pub const fn transferability(&self) -> Transferability {
        self.transferability
    }
    /// Where it stands in a dispute.
    #[must_use]
    pub const fn contest_state(&self) -> &ContestState {
        &self.contest_state
    }
    /// Staleness stamp; the extraction provider checks it.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// The post-image of a contest-state move.
    ///
    /// # Errors
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn with_contest_state(&self, state: ContestState) -> Result<Self, IndustryRecordError> {
        let mut next = self.clone();
        next.contest_state = state;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)?;
        Ok(next)
    }

    /// The post-image of a holder change (the record half of E4's title
    /// transfer; the *authority* half is `AssetOp::TransferTitle` on
    /// [`ClaimRecord::object`], which is where an `Encumbrance::Collateral`
    /// stops it).
    ///
    /// # Errors
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn with_holder(&self, holder: PrincipalId) -> Result<Self, IndustryRecordError> {
        let mut next = self.clone();
        next.holder = holder;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)?;
        Ok(next)
    }
}

// ---- E7c3 B6: the extraction run ----

/// Whether a run is still cycling.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Cycling.
    Running,
    /// Stopped by the operator or the issuer. The record stays.
    Stopped,
}

/// One operator working one deposit.
///
/// **Cut 3 registers no schedule source** (OPEN 5). §3.4 owns scheduling and
/// D-E7-16 gives the tree's first production `Registrar::schedule_source` to
/// cut 8; cut 4 attaches this run's schedule. Each cycle is one authoritative
/// act on cut 2's B1 ingress seam with `IdempotencyKey::Deterministic` over
/// `extract/{deposit}/{cycle}`, and cut 3 defines no second key shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionRun {
    deposit: OreDepositId,
    operator: PrincipalId,
    facility_endpoint: EndpointRef,
    next_cycle: u64,
    state: RunState,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractionRunWire {
    deposit: OreDepositId,
    operator: PrincipalId,
    facility_endpoint: EndpointRef,
    next_cycle: u64,
    state: RunState,
    revision: u64,
}

impl<'de> Deserialize<'de> for ExtractionRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ExtractionRunWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.deposit,
            wire.operator,
            wire.facility_endpoint,
            wire.next_cycle,
            wire.state,
            wire.revision,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ExtractionRun {
    /// Open a run at cycle 0.
    ///
    /// # Errors
    /// [`IndustryRecordError`] never in practice; the wire shares the check.
    pub fn new(
        deposit: OreDepositId,
        operator: PrincipalId,
        facility_endpoint: EndpointRef,
    ) -> Result<Self, IndustryRecordError> {
        Self::from_parts(
            deposit,
            operator,
            facility_endpoint,
            0,
            RunState::Running,
            1,
        )
    }

    fn from_parts(
        deposit: OreDepositId,
        operator: PrincipalId,
        facility_endpoint: EndpointRef,
        next_cycle: u64,
        state: RunState,
        revision: u64,
    ) -> Result<Self, IndustryRecordError> {
        if next_cycle > MAX_EXTRACTION_CYCLES {
            return Err(IndustryRecordError::CycleCap {
                max: MAX_EXTRACTION_CYCLES,
            });
        }
        if revision == 0 {
            return Err(IndustryRecordError::ZeroRevision);
        }
        Ok(Self {
            deposit,
            operator,
            facility_endpoint,
            next_cycle,
            state,
            revision,
        })
    }

    /// Which body.
    #[must_use]
    pub const fn deposit(&self) -> OreDepositId {
        self.deposit
    }
    /// Who is working it — the principal whose licence is checked, which for an
    /// org-held licence is the **org's scope**, not the member personally.
    #[must_use]
    pub const fn operator(&self) -> &PrincipalId {
        &self.operator
    }
    /// Where the lot lands.
    #[must_use]
    pub const fn facility_endpoint(&self) -> EndpointRef {
        self.facility_endpoint
    }
    /// The next cycle ordinal, which is half the deterministic key.
    #[must_use]
    pub const fn next_cycle(&self) -> u64 {
        self.next_cycle
    }
    /// Whether it is cycling.
    #[must_use]
    pub const fn state(&self) -> RunState {
        self.state
    }
    /// Staleness stamp.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// The post-image one completed cycle leaves.
    ///
    /// # Errors
    /// [`IndustryRecordError::CycleCap`] at [`MAX_EXTRACTION_CYCLES`],
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn cycled(&self) -> Result<Self, IndustryRecordError> {
        let next_cycle = self
            .next_cycle
            .checked_add(1)
            .ok_or(IndustryRecordError::CycleCap {
                max: MAX_EXTRACTION_CYCLES,
            })?;
        if next_cycle > MAX_EXTRACTION_CYCLES {
            return Err(IndustryRecordError::CycleCap {
                max: MAX_EXTRACTION_CYCLES,
            });
        }
        let mut next = self.clone();
        next.next_cycle = next_cycle;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)?;
        Ok(next)
    }

    /// The post-image of a stop.
    ///
    /// # Errors
    /// [`IndustryRecordError::RevisionOverflow`] at `u64::MAX`.
    pub fn stopped(&self) -> Result<Self, IndustryRecordError> {
        let mut next = self.clone();
        next.state = RunState::Stopped;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(IndustryRecordError::RevisionOverflow)?;
        Ok(next)
    }
}

// ---- E7c3 B7: the applied marker the facts answer from ----

/// What one committed bundle recorded, and the only thing
/// [`FACT_EXTRACTION_CYCLED`] is ever answered from.
///
/// **Never a status** — E4c's E3 finding was `jobs.resolved` reading a status
/// staff could set by hand (08c §6.2). A bundle that did not commit leaves no
/// marker and the fact answers `None`, which is unsatisfied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedMarker {
    kind: SmolStr,
    deposit: OreDepositId,
    cycle: u64,
    commodity: CanonicalId,
    quantity: u64,
    to: EndpointRef,
    actor: ObjId,
    at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppliedMarkerWire {
    kind: SmolStr,
    deposit: OreDepositId,
    cycle: u64,
    commodity: CanonicalId,
    quantity: u64,
    to: EndpointRef,
    actor: ObjId,
    at: i64,
}

impl<'de> Deserialize<'de> for AppliedMarker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppliedMarkerWire::deserialize(deserializer)?;
        Self::new(
            wire.kind.as_str(),
            wire.deposit,
            wire.cycle,
            wire.commodity,
            wire.quantity,
            wire.to,
            wire.actor,
            wire.at,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl AppliedMarker {
    /// Build a marker.
    ///
    /// # Errors
    /// [`IndustryRecordError`] for an unknown kind or a zero quantity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: &str,
        deposit: OreDepositId,
        cycle: u64,
        commodity: CanonicalId,
        quantity: u64,
        to: EndpointRef,
        actor: ObjId,
        at: i64,
    ) -> Result<Self, IndustryRecordError> {
        if kind != FACT_EXTRACTION_CYCLED {
            return Err(IndustryRecordError::MarkerKind);
        }
        if quantity == 0 {
            return Err(IndustryRecordError::ZeroQuantity);
        }
        Ok(Self {
            kind: SmolStr::new(kind),
            deposit,
            cycle,
            commodity,
            quantity,
            to,
            actor,
            at,
        })
    }

    /// Which fact it answers.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.kind.as_str()
    }
    /// Which body.
    #[must_use]
    pub const fn deposit(&self) -> OreDepositId {
        self.deposit
    }
    /// Which cycle.
    #[must_use]
    pub const fn cycle(&self) -> u64 {
        self.cycle
    }
    /// What came out.
    #[must_use]
    pub const fn commodity(&self) -> &CanonicalId {
        &self.commodity
    }
    /// How much.
    #[must_use]
    pub const fn quantity(&self) -> u64 {
        self.quantity
    }
    /// Where it went.
    #[must_use]
    pub const fn to(&self) -> EndpointRef {
        self.to
    }
    /// Who did it.
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    /// When (IC seconds).
    #[must_use]
    pub const fn at(&self) -> i64 {
        self.at
    }
}

/// How many ore bodies one place carries, so [`MAX_ORE_DEPOSITS_PER_PLACE`] is
/// **counted** rather than merely declared (cut 2's fold B8 mechanism).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PlaceCensus(u32);

#[derive(Deserialize)]
#[serde(transparent)]
struct PlaceCensusWire(u32);

impl<'de> Deserialize<'de> for PlaceCensus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PlaceCensusWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl PlaceCensus {
    /// Build a census.
    ///
    /// # Errors
    /// [`IndustryRecordError::PlaceCensus`] past the cap.
    pub const fn new(count: u32) -> Result<Self, IndustryRecordError> {
        if count as usize > MAX_ORE_DEPOSITS_PER_PLACE {
            return Err(IndustryRecordError::PlaceCensus {
                max: MAX_ORE_DEPOSITS_PER_PLACE,
            });
        }
        Ok(Self(count))
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Every claim **ever filed** over one subject — the **index**, not a counter.
///
/// An `ActionTxn` has no scan, so a provider that must know the claims over a
/// body needs the set written beside the records, in the same bundle. That is
/// cut 2's fold B10 mechanism (`inventory.reservation_index`) applied to tenure.
///
/// **This index is add-only: it is never pruned.** `file_claim` extends it and
/// no path removes an id — a released (`Uncontested`), suspended, seized or
/// past-term claim stays counted. So [`MAX_CLAIMS_PER_SUBJECT`] is a **lifetime**
/// bound on how many claims may ever be filed over one subject, not a bound on
/// the live set (E7c3 verify fold B5). *Exclusivity* is enforced separately, in
/// `IndustryProvider::file`, against the claims whose `contest_state` still
/// [`authorizes`](ContestState::authorizes) — that check reads the live set;
/// this cap counts the lifetime one. Pruning terminal claims from the index is
/// deferred to cut 4, alongside the `FactProvider`/`WorldFacts` widening that
/// retires the marker mirrors.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ClaimIndex {
    claims: Vec<ClaimId>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct ClaimIndexWire(Vec<ClaimId>);

impl<'de> Deserialize<'de> for ClaimIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ClaimIndexWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl ClaimIndex {
    /// Build an index.
    ///
    /// # Errors
    /// [`IndustryRecordError::ClaimCensus`] past the cap, or
    /// [`IndustryRecordError::NonCanonicalRows`] for an unsorted or duplicated
    /// list.
    pub fn new(claims: Vec<ClaimId>) -> Result<Self, IndustryRecordError> {
        if claims.len() > MAX_CLAIMS_PER_SUBJECT {
            return Err(IndustryRecordError::ClaimCensus {
                max: MAX_CLAIMS_PER_SUBJECT,
            });
        }
        let mut previous: Option<ClaimId> = None;
        for claim in &claims {
            if previous.is_some_and(|previous| previous >= *claim) {
                return Err(IndustryRecordError::NonCanonicalRows);
            }
            previous = Some(*claim);
        }
        Ok(Self { claims })
    }

    /// The live set.
    #[must_use]
    pub fn claims(&self) -> &[ClaimId] {
        &self.claims
    }

    /// How many.
    #[must_use]
    pub fn len(&self) -> usize {
        self.claims.len()
    }

    /// Whether the subject carries no claim at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// The post-image of one more claim.
    ///
    /// # Errors
    /// [`IndustryRecordError::ClaimCensus`] past the cap.
    pub fn with(&self, claim: ClaimId) -> Result<Self, IndustryRecordError> {
        if self.claims.contains(&claim) {
            return Ok(self.clone());
        }
        let mut claims = self.claims.clone();
        claims.push(claim);
        claims.sort_unstable();
        Self::new(claims)
    }
}

/// E7c4 (D2 discharged): per-deposit retention of the durable `industry.applied`
/// family.
pub const MAX_RETAINED_MARKERS_PER_DEPOSIT: usize = 8;

/// A bounded FIFO of the most recent applied-marker keys for one deposit, written
/// in the same bundle as each cycle so the durable marker family is **bounded
/// per deposit** (D2, deferred to cut 4). Pruning an evicted marker cannot break
/// replay idempotency: a replayed old cycle refuses at the run's monotone
/// `next_cycle` check long before its marker would be consulted.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MarkerRing {
    keys: Vec<SmolStr>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct MarkerRingWire(Vec<SmolStr>);

impl<'de> Deserialize<'de> for MarkerRing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = MarkerRingWire::deserialize(deserializer)?;
        Self::new(wire.0).map_err(serde::de::Error::custom)
    }
}

impl MarkerRing {
    /// Build a ring, refused past the retention cap (never truncated).
    ///
    /// # Errors
    /// [`IndustryRecordError::MarkerRingCap`] past the cap.
    pub fn new(keys: Vec<SmolStr>) -> Result<Self, IndustryRecordError> {
        if keys.len() > MAX_RETAINED_MARKERS_PER_DEPOSIT {
            return Err(IndustryRecordError::MarkerRingCap {
                max: MAX_RETAINED_MARKERS_PER_DEPOSIT,
            });
        }
        Ok(Self { keys })
    }

    /// An empty ring.
    #[must_use]
    pub fn empty() -> Self {
        Self { keys: Vec::new() }
    }

    /// Push a key, returning the new ring and the evicted (oldest) key, if the
    /// push overflowed the retention cap.
    #[must_use]
    pub fn push(&self, key: SmolStr) -> (Self, Option<SmolStr>) {
        let mut keys = self.keys.clone();
        keys.push(key);
        let evicted = if keys.len() > MAX_RETAINED_MARKERS_PER_DEPOSIT {
            Some(keys.remove(0))
        } else {
            None
        };
        (Self { keys }, evicted)
    }

    /// The retained keys, oldest first.
    #[must_use]
    pub fn keys(&self) -> &[SmolStr] {
        &self.keys
    }

    /// How many keys are retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the ring is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Every invariant refusal a cut-3 record can make.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IndustryRecordError {
    /// A band token was empty, oversized or non-canonical.
    #[error("a band id is 1..=32 bytes of [a-z0-9_.-]")]
    Band,
    /// Accessibility above the cap.
    #[error("accessibility is a percentage 0..={max}")]
    Accessibility {
        /// The cap.
        max: u8,
    },
    /// A malformed yield curve.
    #[error(
        "a yield curve is 1..=16 points starting at cumulative 0, with strictly increasing \
         thresholds and never-rising yields"
    )]
    Curve,
    /// A curve point's yield above the cap.
    #[error("a per-cycle yield is at most {max}")]
    CycleYield {
        /// The cap.
        max: u64,
    },
    /// A reserve that is zero or above the cap.
    #[error("an authored reserve is 1..={max}")]
    Reserve {
        /// The cap.
        max: u64,
    },
    /// A claim priority above the cap.
    #[error("a claim priority is at most {max}")]
    Priority {
        /// The cap.
        max: u32,
    },
    /// A survey map past the cap. **Refused, never truncated.**
    #[error("a deposit discloses to at most {max} principals")]
    SurveyBandCap {
        /// The cap.
        max: usize,
    },
    /// Survey rows out of canonical order, or a duplicated principal.
    #[error("survey rows are sorted by principal and duplicate-free")]
    NonCanonicalRows,
    /// A cycle ordinal past the cap.
    #[error("a run reaches at most {max} cycles")]
    CycleCap {
        /// The cap.
        max: u64,
    },
    /// A place with too many ore bodies.
    #[error("a place carries at most {max} ore bodies")]
    PlaceCensus {
        /// The cap.
        max: usize,
    },
    /// A subject with too many live claims.
    #[error("a subject carries at most {max} live claims")]
    ClaimCensus {
        /// The cap.
        max: usize,
    },
    /// A marker naming a fact this crate does not publish.
    #[error("an applied marker names one of this crate's own facts")]
    MarkerKind,
    /// A zero quantity where one is meaningless.
    #[error("a quantity is non-zero")]
    ZeroQuantity,
    /// A zero revision.
    #[error("a revision starts at 1")]
    ZeroRevision,
    /// A revision at `u64::MAX`.
    #[error("a revision would overflow")]
    RevisionOverflow,
    /// A facility slot count that is zero or above the cap (E7c4 B1).
    #[error("a facility runs 1..={max} concurrent slots")]
    FacilitySlots {
        /// The cap.
        max: u32,
    },
    /// A facility capability set past the cap (E7c4 B1).
    #[error("a facility carries at most {max} capabilities")]
    FacilityCapabilities {
        /// The cap.
        max: usize,
    },
    /// A work order input line count past the cap (E7c4 B3).
    #[error("a work order names at most {max} material inputs")]
    WorkOrderInputs {
        /// The cap.
        max: usize,
    },
    /// A reservation list on a stored work order past the cap. **Refused on
    /// load, never truncated** (E7c4 B3).
    #[error("a work order names at most {max} reservations")]
    ReservationCap {
        /// The cap.
        max: usize,
    },
    /// A recipe pinned with a zero quantity, or work units above the cap
    /// (E7c4 B3).
    #[error("a work order quantity is 1..={max}")]
    QuantityCap {
        /// The cap.
        max: u32,
    },
    /// A marker ring past its retention cap (E7c4 D2).
    #[error("a marker ring retains at most {max} keys")]
    MarkerRingCap {
        /// The cap.
        max: usize,
    },
    /// A recipe that produces output while consuming **no** input — minting matter
    /// from nothing, which is staff-only (deposits) and never a work order (C1,
    /// independent verify fold). It is a floor, not a substitute for authority
    /// binding: it does not stop an authorized insider inflating a real recipe's
    /// output lines, which only a kernel prepare-seam closes.
    #[error("a work-order recipe that mints output must consume at least one input")]
    RecipeMintsFromNothing,
}
