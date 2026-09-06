//! **`mush-industry` (E7 cut 3) — ore deposits, surveys, claims and extraction.**
//!
//! 10b §3.3 / doc 10 §14: the neutral industry authority. Cut 3 ships deposits,
//! extraction rights, surveys and claims; facilities, recipes, capacity and
//! production are cut 4's, on this same crate and this same `OwnerId`.
//!
//! # What this crate is not
//!
//! Nothing TacSim-shaped enters it: **no ore family definition, no band table,
//! no route, no corridor fact, no location name, no recipe, no listing, no
//! combat formula**. The *ruleset* authors that data through cut 1's packs and
//! this crate owns the state machine (`crates/mush-tactics/src/mobility.rs`'s
//! stated pattern). It also builds nothing from cuts 4–11: no facility, no
//! recipe evaluation, no work order, no capacity reservation, no production
//! schedule, no domain clock, no propagation, no recovery.
//!
//! # What it consumes verbatim and never rebuilds
//!
//! * **K's contention kernel** — `ReservationBook::checked_apply_resource` and
//!   `checked_hold` are the only arithmetic gate; cut 3 writes **no depletion
//!   arithmetic of its own**, so "two reservations against one remaining unit"
//!   is closed by the kernel and not by a check in this crate.
//! * **Cut 2's quantity authority** — `EndpointRef`, the pool/book pair, `Lot`,
//!   `Grade`, `Lineage`, `PoolTranche` and the owner-exported verbs. A deposit's
//!   remaining reserve **is** cut 2's pool at the deposit's own endpoint, so
//!   D-E7-3's "industry keeps no private stock number" is literally true.
//! * **E3/E4's custody and title** — `Vault`, `CustodyRef`, `CustodyProvider`,
//!   `resolve`'s exactly-one-claiming-provider rule, `EconomicAsset`,
//!   `Encumbrance`, `issue_title`, `AssetOp`. Cut 3 adds **no second custody
//!   model, no second title record and no second encumbrance**.
//! * **E6c's catalog and the whole of `mush-market`**, which it does not touch:
//!   a `Representation::Lot` listing is still refused at creation and lifting
//!   that refusal is **cut 11's**, not cut 3's.
//! * **Money stays an E4 `Hold`** and never grows a resource field (T4,
//!   D-E7-2).
//!
//! # No RNG, anywhere
//!
//! Surveys are deterministic and take no RNG: a survey is a *revelation* of the
//! deposit's **recorded** classes, and yield is a deterministic curve with
//! depletion (D-E7-6, Q1A). There is no `rand::`, no `rand_chacha`, no
//! `thread_rng`, no `SeedableRng` and no `OsRng` in this crate, so
//! `crates/mush-contract-tests/tests/provider_rng_policy.rs` is green by
//! construction. If a later cut ever wants survey *quality* to vary, the only
//! lawful source is `PrepareCtx::start_resolution` on the prepared path — never
//! ambient RNG — and that would move `@survey` to `Authoritative`.
//!
//! # The three lenses cut 3 is verified on
//!
//! **F (fog)** — can a viewer learn a reserve, a curve or another principal's
//! band through any render, error, or arithmetic difference? **D (depletion)** —
//! can total extracted exceed the reserve under concurrency, restart or event
//! ordering? **T (tenure)** — can a claim be transferred, extracted under, or
//! contested by someone the authority chain does not permit?

pub mod commands;
pub mod custody;
pub mod facts;
pub mod families;
pub mod licence;
pub mod module;
pub mod provider;
pub mod record;
pub mod store;

pub use custody::IndustryCustodyProvider;
pub use facts::IndustryFactProvider;
pub use families::{IndustryError, IndustryFamilies, claim_subject_token, marker_id};
pub use licence::{ExtractionRefusal, extraction_authorized, licence_accounts};
pub use module::{
    CLAIM_CATALOG_CODE, CLAIM_CATALOG_RULESET, IndustryHandles, IndustryModule, SWEEP_PERIOD_SECS,
    SWEEP_TAG, industry_title_issuer,
};
pub use provider::{IndustryVerb, action_ref, deposit_backing, deposit_of_backing};
pub use record::{
    Accessibility, AppliedMarker, BandId, ClaimId, ClaimIndex, ClaimRecord, ClaimSubject,
    ContestState, CurvePoint, Exclusivity, ExtractionRun, FACT_EXTRACTION_CYCLED,
    FACT_WORKORDER_COMPLETED, INDUSTRY_PAGE, IndustryRecordError, MAX_ACCESSIBILITY,
    MAX_APPLIED_MARKERS, MAX_BAND_BYTES, MAX_CLAIM_PRIORITY, MAX_CLAIMS_PER_SUBJECT,
    MAX_CURVE_POINTS, MAX_CYCLE_YIELD, MAX_EXTRACTION_CYCLES, MAX_LIST_PAGES, MAX_LIST_ROWS,
    MAX_MARKER_ID_BYTES, MAX_MARKER_PAGES, MAX_ORE_DEPOSITS_PER_PLACE, MAX_ORE_RESERVE,
    MAX_REASON_BYTES, MAX_SURVEY_BANDS, NS, OreDeposit, OreDepositId, PlaceCensus, RunState,
    SurveyBands, SurveyDoc, SurveyRow, Transferability, YieldCurve, owner,
};
pub use store::{IndustryStore, Quarantine};
