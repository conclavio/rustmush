//! E7c3 B2/B4/B6 — the prepared path, on `crates/ruleset-wod/src/authoritative.rs`'s
//! shape: state family + definition validator + definitions + `reg.action_provider`.
//!
//! # Where legality lives, and why there
//!
//! `prepare_authoritative` receives an `ActionTxn` and **no world**, so a
//! licence account, a vault's status and `acts_for` cannot be read inside a
//! provider. Legality therefore lives where the tree already puts it (cut 2's
//! deviation (d)): [`ActionProvider::project_offer`] publishes the ore bodies
//! this actor may work **right now** as `ProjectedParameterKind::Choice`
//! values, and `Engine::resolve_legal_action` **regenerates and re-validates
//! the offer immediately before preparation**. A licence that expires, a claim
//! that is suspended or a body that depletes between the command and the commit
//! therefore drops out of the regenerated list and the submission is refused.
//! This is not a check that can go stale — it is the same mechanism a station
//! grant uses.
//!
//! It is also the **fog** property: a body the actor may not work never appears
//! in the list at all, so naming one is refused identically whether it exists,
//! is depleted, is claimed by somebody else or has never been surveyed.
//!
//! What the provider *can* see it re-checks anyway: the claim, the run and the
//! deposit are this owner's typed families, and the cycle re-reads all three
//! under **checked revisions** before it writes a thing.
//!
//! # No RNG
//!
//! Nothing in this file draws. Yield is `YieldCurve::at` — a bounded walk over
//! a monotone table with checked integer arithmetic in a fixed order — so two
//! evaluations of one cycle always agree and a replay is a HIT.

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp, CommitToken,
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, DurableSchedule, DurableScheduleDraft, Engine, LegalActionQuery,
    MAX_ACTION_PARAMETER_BYTES, ModError, ObjId, PerceivedRef, PrincipalId as CorePrincipalId,
    Registrar, ReservationId, RevisionSet, RulesValue, ScheduleActionAdapter, ScheduleActionDraft,
    ScheduleAdvance, ScheduleCatchUpOrder, ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy,
    StateFamilyHandle, SubjectKey, World,
};
use mush_economy::{AccountId, ContractId, EconomicVisibility, ExternalRef, VaultId};
use mush_inventory::{EndpointRef, Grade};
use mush_org::PrincipalId;
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::families::{
    Cycled, IndustryError, IndustryFamilies, derive_claim_id, derive_deposit_id, marker_id,
};
use crate::record::{
    Accessibility, BandId, ClaimId, ClaimRecord, ClaimSubject, ContestState, CurvePoint,
    Exclusivity, ExtractionRun, FACT_WORKORDER_COMPLETED, MAX_ACCESSIBILITY, MAX_BAND_BYTES,
    MAX_CLAIM_PRIORITY, MAX_CURVE_POINTS, MAX_ORE_RESERVE, MAX_REASON_BYTES, NS, OreDeposit,
    OreDepositId, RunState, Transferability, YieldCurve,
};
use crate::workorder::{
    DeliveryTarget, InterruptionCause, PRODUCTION_CADENCE_SECS, PRODUCTION_MAX_CATCH_UP,
    PRODUCTION_UNITS_PER_OCCURRENCE, PinnedRecipe, ProductionJob, ProductionSchedule, RecipeRef,
    WorkOrder, WorkOrderId, WorkOrderMarker, WorkOrderState, derive_work_order_id, wo_holder,
};

/// The eight action ids, and the one outcome schema they share.
pub const ACTION_DEPOSIT_OPEN: &str = "industry_deposit_open";
/// Move a body's accessibility (the attributed campaign event).
pub const ACTION_DEPOSIT_ACCESS: &str = "industry_deposit_access";
/// File a claim.
pub const ACTION_CLAIM_FILE: &str = "industry_claim_file";
/// The record half of a title transfer.
pub const ACTION_CLAIM_MOVE: &str = "industry_claim_move";
/// Move a claim's contest state.
pub const ACTION_CLAIM_CONTEST: &str = "industry_claim_contest";
/// Open an extraction run.
pub const ACTION_EXTRACT_START: &str = "industry_extract_start";
/// Run one extraction cycle.
pub const ACTION_EXTRACT_CYCLE: &str = "industry_extract_cycle";
/// Stop an extraction run.
pub const ACTION_EXTRACT_STOP: &str = "industry_extract_stop";
/// Draft a work order (E7c4).
pub const ACTION_WO_DRAFT: &str = "industry_wo_draft";
/// Submit (acquire + start) a work order (E7c4).
pub const ACTION_WO_SUBMIT: &str = "industry_wo_submit";
/// Advance a work order's production one occurrence (E7c4, the schedule action).
pub const ACTION_WO_ADVANCE: &str = "industry_wo_advance";
/// Deliver (complete) a work order (E7c4).
pub const ACTION_WO_DELIVER: &str = "industry_wo_deliver";
/// Cancel a work order (E7c4).
pub const ACTION_WO_CANCEL: &str = "industry_wo_cancel";
/// Restamp an interrupted work order (E7c4).
pub const ACTION_WO_RESTAMP: &str = "industry_wo_restamp";
/// The one durable outcome schema all actions reference.
pub const OUTCOME_ID: &str = "industry_result";

/// Largest ore-body choice list one offer publishes. Well inside K's
/// `MAX_PROJECTED_ACTION_CHOICES` (256).
pub const MAX_DEPOSIT_CHOICES: usize = 64;
/// Largest commodity id one parameter may carry.
pub const MAX_COMMODITY_BYTES: usize = 64;
/// Largest facility book one `WoDraft` offer publishes.
pub const MAX_WO_OFFER_FACILITIES: usize = 16;
/// Largest endpoint book one `WoDraft` offer publishes.
pub const MAX_WO_OFFER_ENDPOINTS: usize = 16;
/// Largest account book one `WoDraft` offer publishes.
pub const MAX_WO_OFFER_ACCOUNTS: usize = 16;
/// Largest recipe book one `WoDraft` offer publishes. Well inside K's
/// `MAX_PROJECTED_ACTION_CHOICES` (256) and, at 96 rows, inside
/// [`MAX_WO_OFFER_BOOK_BYTES`] for any key a pack can spell.
pub const MAX_OFFERED_RECIPES: usize = 96;
/// Largest the recipe book may grow, in bytes of key + pinned hash + pack
/// digest. The whole fixed object still has to canonicalize inside
/// `MAX_ACTION_PARAMETER_BYTES` and the offer inside
/// `MAX_PROJECTED_LEGAL_ACTION_BYTES`; this is the crate's own, much tighter
/// bound, so the book is never what breaks either.
///
/// WS02-S5 doubled it because each row grew the 64-hex pack digest: the digest
/// is no longer a client parameter, so the book is the only place `wo_draft` can
/// read it from. At 96 rows the count cap still bites first for any key a real
/// pack spells; the byte cap is what catches a pathological one.
pub const MAX_WO_OFFER_BOOK_BYTES: usize = 16 * 1024;
/// Largest curve text one parameter may carry (`c:y,c:y,…`).
pub const MAX_CURVE_BYTES: usize = 256;

pub use crate::custody::{backing as deposit_backing, deposit_of as deposit_of_backing};

/// Which of the eight this provider instance owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndustryVerb {
    /// Open one ore body and seed its endpoint pool.
    DepositOpen,
    /// Move one body's accessibility for the **next** cycle.
    DepositAccess,
    /// File one claim over a body or a place.
    ClaimFile,
    /// The record half of a title transfer.
    ClaimMove,
    /// Move one claim's contest state.
    ClaimContest,
    /// Open an extraction run at one body.
    ExtractStart,
    /// Run exactly one extraction cycle.
    ExtractCycle,
    /// Stop an extraction run.
    ExtractStop,
    /// E7c4 B3: draft one work order (Draft state, no reservation).
    WoDraft,
    /// E7c4 B4: acquire and start one work order (reserve materials + capacity).
    WoSubmit,
    /// E7c4 B7: run one production occurrence (progress; the schedule advance).
    WoAdvance,
    /// E7c4 B7: complete and deliver one work order (consume, materialize, place).
    WoDeliver,
    /// E7c4 B7: cancel one work order (release every reservation once).
    WoCancel,
    /// E7c4 B6: restamp one interrupted order under a campaign-registered policy.
    WoRestamp,
}

impl IndustryVerb {
    /// Every verb, in registration order.
    #[must_use]
    pub const fn all() -> [Self; 14] {
        [
            Self::DepositOpen,
            Self::DepositAccess,
            Self::ClaimFile,
            Self::ClaimMove,
            Self::ClaimContest,
            Self::ExtractStart,
            Self::ExtractCycle,
            Self::ExtractStop,
            Self::WoDraft,
            Self::WoSubmit,
            Self::WoAdvance,
            Self::WoDeliver,
            Self::WoCancel,
            Self::WoRestamp,
        ]
    }

    const fn id(self) -> &'static str {
        match self {
            Self::DepositOpen => ACTION_DEPOSIT_OPEN,
            Self::DepositAccess => ACTION_DEPOSIT_ACCESS,
            Self::ClaimFile => ACTION_CLAIM_FILE,
            Self::ClaimMove => ACTION_CLAIM_MOVE,
            Self::ClaimContest => ACTION_CLAIM_CONTEST,
            Self::ExtractStart => ACTION_EXTRACT_START,
            Self::ExtractCycle => ACTION_EXTRACT_CYCLE,
            Self::ExtractStop => ACTION_EXTRACT_STOP,
            Self::WoDraft => ACTION_WO_DRAFT,
            Self::WoSubmit => ACTION_WO_SUBMIT,
            Self::WoAdvance => ACTION_WO_ADVANCE,
            Self::WoDeliver => ACTION_WO_DELIVER,
            Self::WoCancel => ACTION_WO_CANCEL,
            Self::WoRestamp => ACTION_WO_RESTAMP,
        }
    }

    /// The future family a verb's descriptor names (rule 1's tell).
    const fn family(self) -> &'static str {
        match self {
            Self::WoDraft
            | Self::WoSubmit
            | Self::WoAdvance
            | Self::WoDeliver
            | Self::WoCancel
            | Self::WoRestamp => "industry.workorder",
            _ => "industry.deposit",
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::DepositOpen => "deposit-open",
            Self::DepositAccess => "deposit-access",
            Self::ClaimFile => "claim-file",
            Self::ClaimMove => "claim-move",
            Self::ClaimContest => "claim-contest",
            Self::ExtractStart => "extract-start",
            Self::ExtractCycle => "extract-cycle",
            Self::ExtractStop => "extract-stop",
            Self::WoDraft => "wo-draft",
            Self::WoSubmit => "wo-submit",
            Self::WoAdvance => "wo-advance",
            Self::WoDeliver => "wo-deliver",
            Self::WoCancel => "wo-cancel",
            Self::WoRestamp => "wo-restamp",
        }
    }

    const fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::DepositOpen => (
                "Open an ore body",
                "Record an ore body and seed its reserve.",
            ),
            Self::DepositAccess => (
                "Change accessibility",
                "Record a campaign event that moves an ore body's next cycle.",
            ),
            Self::ClaimFile => ("File a claim", "File tenure over an ore body or a place."),
            Self::ClaimMove => ("Move a claim", "Record a claim's new holder."),
            Self::ClaimContest => ("Contest a claim", "Move a claim's contest state."),
            Self::ExtractStart => ("Start extraction", "Open an extraction run at an ore body."),
            Self::ExtractCycle => ("Run a cycle", "Run one extraction cycle."),
            Self::ExtractStop => ("Stop extraction", "Stop an extraction run."),
            Self::WoDraft => (
                "Draft a work order",
                "Record a production order at a facility.",
            ),
            Self::WoSubmit => (
                "Submit a work order",
                "Reserve materials and a facility slot and start production.",
            ),
            Self::WoAdvance => (
                "Advance production",
                "Run one scheduled production occurrence.",
            ),
            Self::WoDeliver => (
                "Deliver a work order",
                "Complete production and place the output at its destination.",
            ),
            Self::WoCancel => ("Cancel a work order", "Release every reservation once."),
            Self::WoRestamp => (
                "Restamp a work order",
                "Re-anchor an interrupted order under a campaign policy.",
            ),
        }
    }
}

/// One industry action's provider.
pub struct IndustryProvider {
    verb: IndustryVerb,
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: IndustryFamilies,
    /// The composition root's recipe authority, the same `Arc` the command lane
    /// and the production schedule adapter hold. `None` on a server that has said
    /// nothing about what a recipe is: `@wo/draft` already refuses there, and the
    /// offer follows it.
    recipes: Option<std::sync::Arc<dyn crate::workorder::RecipeAuthority>>,
    /// The hashed catalog, keyed on [`crate::workorder::RecipeAuthority::catalog_revision`]. The
    /// walk and the hashing happen once per pack publish, not once per
    /// projection — and `project_legal_actions` runs every provider on every
    /// projection pass, including every authoritative submission.
    catalog: std::sync::Mutex<Option<(u64, std::sync::Arc<Vec<CatalogEntry>>)>>,
    /// The catalog revision the truncation warning has already been logged for,
    /// so a truncated book says so once per publish rather than once per
    /// projection.
    truncation_warned: std::sync::Mutex<Option<u64>>,
}

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded industry action error")
}

/// The deterministic `ScheduleId` one work order's production schedule lives at
/// (`production.{order:016x}` under `mush-industry`'s owner). Factored so the
/// terminal transitions can tear the same schedule down that `open_schedule`
/// created (R1, independent verify fold).
fn production_schedule_id(order: WorkOrderId) -> Result<ScheduleId, ActionError> {
    let local = CanonicalId::new(format!("production.{:016x}", order.0))
        .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
    ScheduleId::derive(&crate::record::owner(), &local)
        .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))
}

/// **One message, whatever the reason.** Depletion, a missing licence, a
/// suspended claim and a body the actor may not know exists all answer the same
/// sentence to a non-staff principal (08c §6.2's D1 shape, §4 rule 4): two
/// refusals a viewer may not distinguish are byte-identical.
const REFUSED: &str = "You cannot work that here.";

/// The actor the `ClaimFile` offer was projected to, published as a fixed
/// parameter so the prepare can tell its own offer from somebody else's.
pub const CLAIM_FILED_BY: &str = "claim_filed_by";
/// Whether that actor was staff when the `ClaimFile` offer was projected — the
/// only way to reach `file()` with an issuer no organization vouches for.
pub const CLAIM_FILE_STAFF: &str = "claim_file_staff";
/// The `principal_token` spellings of the authorities a non-staff actor may file
/// under, published as a fixed parameter beside the offer's own `Choice`.
pub const CLAIM_ISSUERS: &str = "claim_issuers";

/// The actor the `WoDraft` offer was projected to, published as a fixed
/// parameter so the prepare can tell its own offer from somebody else's.
pub const WO_DRAFTER: &str = "wo_drafter";
/// Whether that actor was staff when the `WoDraft` offer was projected — staff
/// may draft at any facility by id, exactly as they may name any claim by id.
pub const WO_DRAFT_STAFF: &str = "wo_draft_staff";
/// The facilities the drafter may draft at, `id → {slots, capacity_revision,
/// policy_revision, capabilities}`, server-authored beside the offer.
pub const WO_FACILITIES: &str = "wo_facilities";
/// The drafter's live inventory endpoints, server-authored beside the offer.
pub const WO_ENDPOINTS: &str = "wo_endpoints";
/// The drafter's own active accounts, server-authored beside the offer.
pub const WO_ACCOUNTS: &str = "wo_accounts";
/// The recipes the drafter's facilities can actually run,
/// `"<pack>/<version>/<id>" → 32-hex pinned hash`.
pub const WO_RECIPES: &str = "wo_recipes";
/// Whether [`WO_RECIPES`] left something out — the flag an operator reads when a
/// recipe that exists cannot be drafted.
pub const WO_RECIPES_TRUNCATED: &str = "wo_recipes_truncated";

/// The one line every work-order refusal a viewer may not distinguish renders.
pub(crate) const WO_REFUSED: &str = "You cannot do that with that work order.";
/// The line a replayed delivery renders (a marker HIT).
const WO_ALREADY: &str = "That work order has already been delivered.";

fn refuse(source: &IndustryError) -> ActionError {
    match source {
        IndustryError::State | IndustryError::Identity => error(
            ActionErrorCode::Integrity,
            "that ore body cannot be used right now",
        ),
        // Every mechanical refusal a principal could mine for information
        // renders identically.
        IndustryError::NoDeposit
        | IndustryError::NoClaim
        | IndustryError::NoRun
        | IndustryError::RunStopped
        | IndustryError::RunNotYours
        | IndustryError::Unworkable
        | IndustryError::Depleted => error(ActionErrorCode::ProviderRejected, REFUSED),
        // Every work-order mechanical refusal a competitor could mine for
        // information renders identically (the P lens): a full facility slot, a
        // missing order, a stale record and a half-present capacity book all read
        // the same to a principal who may not see the order.
        IndustryError::NoWorkOrder
        | IndustryError::WorkOrderExists
        | IndustryError::Book(_)
        | IndustryError::Reservation(_)
        | IndustryError::CapacityHalf
        | IndustryError::NoCapacity => error(ActionErrorCode::ProviderRejected, WO_REFUSED),
        other => error(ActionErrorCode::ProviderRejected, other.to_string()),
    }
}

// ---- parameter helpers ----

fn text_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("an industry action needs a {key}"),
            )
        })
}

fn u64_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
    max: u64,
) -> Result<u64, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value >= 1 && *value <= max)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("an industry action needs a {key} between 1 and {max}"),
            )
        })
}

fn deposit_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<OreDepositId, ActionError> {
    OreDepositId::parse(&text_field(parameters, key)?)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, REFUSED))
}

fn claim_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<ClaimId, ActionError> {
    ClaimId::parse(&text_field(parameters, key)?)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, REFUSED))
}

fn endpoint_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<EndpointRef, ActionError> {
    text_field(parameters, key)?
        .parse::<u64>()
        .ok()
        .map(|id| EndpointRef(VaultId(id)))
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("an industry action needs an endpoint for {key}"),
            )
        })
}

/// A **required** endpoint carried as an integer — the shape `project_offer`
/// declares for `source` and `destination`, and the shape `@wo/draft` stages.
///
/// ---- E7r1 fold Z4: `@wo/draft` never resolved on a real server ----
///
/// Found by driving Z2's rung, not on the verify's list. `IndustryVerb::WoDraft`'s
/// projection declares `integer("destination", …)` and the command stages a JSON
/// **number**, while [`endpoint_field`] reads through [`text_field`]'s
/// `Value::as_str` — so the prepare answered *"an industry action needs a
/// destination"* on every submission that actually went through
/// `resolve_legal_action`. Nothing caught it because every shipped test builds a
/// `WorkOrder` by hand or stages a submission it never resolves. `source` was
/// already read numerically by [`optional_endpoint`]; this is its required twin, so
/// the declaration, the staging and the read are one shape.
fn required_endpoint(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<EndpointRef, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_u64)
        .filter(|id| *id != 0)
        .map(|id| EndpointRef(VaultId(id)))
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("an industry action needs an endpoint for {key}"),
            )
        })
}

fn work_order_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<WorkOrderId, ActionError> {
    WorkOrderId::parse(&text_field(parameters, key)?)
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, WO_REFUSED))
}

fn canonical_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<CanonicalId, ActionError> {
    let raw = text_field(parameters, key)?;
    if raw.len() > max {
        return Err(error(ActionErrorCode::InvalidRequest, WO_REFUSED));
    }
    CanonicalId::new(&raw).map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))
}

fn optional_endpoint(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<EndpointRef>, ActionError> {
    match parameters.get(key).and_then(Value::as_u64) {
        None | Some(0) => Ok(None),
        Some(id) => Ok(Some(EndpointRef(VaultId(id)))),
    }
}

fn commodity_field(
    parameters: &serde_json::Map<String, Value>,
) -> Result<CanonicalId, ActionError> {
    let raw = text_field(parameters, "commodity")?;
    if raw.len() > MAX_COMMODITY_BYTES {
        return Err(error(
            ActionErrorCode::InvalidRequest,
            "that commodity id is too long",
        ));
    }
    CanonicalId::new(&raw).map_err(|_| {
        error(
            ActionErrorCode::InvalidRequest,
            "that is not a commodity id",
        )
    })
}

fn band_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<BandId, ActionError> {
    BandId::new(&text_field(parameters, key)?)
        .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))
}

fn principal_field(
    parameters: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<PrincipalId, ActionError> {
    let raw = text_field(parameters, key)?;
    parse_principal(&raw).ok_or_else(|| {
        error(
            ActionErrorCode::InvalidRequest,
            format!("an industry action needs a principal for {key}"),
        )
    })
}

/// `obj:<n>` / `org:<n>` / `sys:<name>` — the economy's own `principal_token`
/// spelling, read back. Cut 3 defines no second principal text.
#[must_use]
pub fn parse_principal(text: &str) -> Option<PrincipalId> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("obj:") {
        return rest
            .parse::<u32>()
            .ok()
            .map(|id| PrincipalId::Object(ObjId(id)));
    }
    if let Some(rest) = text.strip_prefix("org:") {
        return rest
            .parse::<u64>()
            .ok()
            .map(|id| PrincipalId::Organization(mush_org::OrgId(id)));
    }
    if let Some(rest) = text.strip_prefix("sys:") {
        return mush_org::SystemPrincipalId::new(rest).map(PrincipalId::System);
    }
    None
}

/// `c:y,c:y,…` — a bounded monotone curve, parsed once.
///
/// # Errors
/// [`ActionError`] for an oversized, malformed or non-monotone table.
pub fn parse_curve(text: &str) -> Result<YieldCurve, ActionError> {
    if text.len() > MAX_CURVE_BYTES {
        return Err(error(
            ActionErrorCode::InvalidRequest,
            "that yield curve is too long",
        ));
    }
    let mut points = Vec::new();
    for token in text.split(',') {
        if points.len() >= MAX_CURVE_POINTS {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                format!("a yield curve names at most {MAX_CURVE_POINTS} points"),
            ));
        }
        let (cumulative, per_cycle) = token.trim().split_once(':').ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "a yield curve point is <cumulative>:<per-cycle>",
            )
        })?;
        let cumulative = cumulative.trim().parse::<u64>().map_err(|_| {
            error(
                ActionErrorCode::InvalidRequest,
                "a yield curve threshold is a whole number",
            )
        })?;
        let per_cycle = per_cycle.trim().parse::<u64>().map_err(|_| {
            error(
                ActionErrorCode::InvalidRequest,
                "a yield curve yield is a whole number",
            )
        })?;
        points.push(CurvePoint {
            cumulative,
            per_cycle,
        });
    }
    YieldCurve::new(points)
        .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))
}

/// The IC stamp a preparation uses. Deliberately **not** the wall clock: every
/// `term` comparison later is `ClockStamp::checked_cmp`, which refuses to
/// compare across generations rather than expiring a claim early.
fn prepare_now(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
    parameters
        .get("now")
        .and_then(|value| serde_json::from_value::<ClockStamp>(value.clone()).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::Unavailable,
                "the world clock is unavailable",
            )
        })
}

fn parameter(
    id: &str,
    required: bool,
    kind: mush_core::ProjectedParameterKind,
) -> Result<mush_core::ProjectedParameter, ActionError> {
    Ok(mush_core::ProjectedParameter {
        id: CanonicalId::new(id)
            .map_err(|_| error(ActionErrorCode::Integrity, "industry parameter id"))?,
        required,
        kind,
    })
}

fn text(id: &str, bytes: usize) -> Result<mush_core::ProjectedParameter, ActionError> {
    parameter(
        id,
        true,
        mush_core::ProjectedParameterKind::Text {
            maximum_bytes: bytes,
        },
    )
}

fn integer(id: &str, min: i64, max: i64) -> Result<mush_core::ProjectedParameter, ActionError> {
    parameter(
        id,
        true,
        mush_core::ProjectedParameterKind::Integer {
            minimum: min,
            maximum: max,
        },
    )
}

/// Every ore body this actor may work right now, bounded and sorted.
///
/// Bounded on records **visited** (D-LBK-5) and total: an unreadable family
/// answers an empty list, which projects **no offer at all** rather than an
/// offer nobody can honour — and a viewer with no authorized body sees exactly
/// what a viewer whose licence lapsed sees, which is nothing.
#[must_use]
pub fn authorized_deposits(
    engine: &Engine,
    families: &IndustryFamilies,
    actor: ObjId,
    operator: &PrincipalId,
) -> Vec<OreDepositId> {
    let world = &engine.world;
    let state = engine.action_state();
    let licence = world
        .module_state::<crate::store::IndustryStore>()
        .and_then(|store| store.licence().cloned());
    let now = world.world_ic_now();
    let mut claims: Vec<ClaimRecord> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_LIST_PAGES {
            break;
        }
        pages += 1;
        let Ok(page) = state.scan_family(
            &families.claims,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) else {
            return Vec::new();
        };
        for entry in page.entries() {
            if claims.len() >= crate::record::MAX_LIST_ROWS * 4 {
                break;
            }
            claims.push(entry.value().clone());
        }
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if out.len() > MAX_DEPOSIT_CHOICES || pages >= crate::record::MAX_LIST_PAGES {
            break;
        }
        pages += 1;
        let Ok(page) = state.scan_family(
            &families.deposits,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) else {
            return Vec::new();
        };
        let mut rows: Vec<OreDeposit> = page
            .entries()
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let next = page.next_after().map(ToString::to_string);
        rows.sort_by_key(OreDeposit::id);
        for deposit in rows {
            if out.len() > MAX_DEPOSIT_CHOICES {
                break;
            }
            if crate::store::is_quarantined(world, deposit.id()) {
                continue;
            }
            let remaining = state
                .get(&families.inventory.pools, &{
                    match mush_inventory::pool_key(deposit.endpoint(), deposit.commodity()) {
                        Ok(key) => key,
                        Err(_) => continue,
                    }
                })
                .ok()
                .flatten()
                .and_then(|scalar| u64::try_from(scalar.current()).ok())
                .unwrap_or(0);
            let mine: Vec<ClaimRecord> = claims
                .iter()
                .filter(|claim| {
                    claim.subject()
                        == ClaimSubject::Deposit {
                            deposit: deposit.id(),
                        }
                })
                .cloned()
                .collect();
            if crate::licence::extraction_authorized(
                world,
                actor,
                operator,
                &deposit,
                remaining,
                &mine,
                licence.as_ref(),
                &now,
            )
            .is_ok()
            {
                out.push(deposit.id());
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    out.sort_unstable();
    out.truncate(MAX_DEPOSIT_CHOICES);
    out
}

/// Every claim, bounded on records **visited** (D-LBK-5). An unreadable family
/// answers an empty list, which projects no offer at all.
fn scan_all_claims(engine: &Engine, families: &IndustryFamilies) -> Vec<ClaimRecord> {
    let state = engine.action_state();
    let mut claims: Vec<ClaimRecord> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_LIST_PAGES {
            break;
        }
        pages += 1;
        let Ok(page) = state.scan_family(
            &families.claims,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) else {
            return Vec::new();
        };
        for entry in page.entries() {
            if claims.len() >= crate::record::MAX_LIST_ROWS * 4 {
                break;
            }
            claims.push(entry.value().clone());
        }
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    claims
}

/// The claims a non-staff actor may move the **contest state** of: those whose
/// `issuer()` its principal `acts_for` (E7c3 verify fold C1). Bounded, sorted,
/// capped at [`MAX_DEPOSIT_CHOICES`]. Staff are not restricted here — this feeds
/// the non-staff `Choice`, so a griefer is never offered a foreign claim id.
#[must_use]
pub fn administrable_claims(
    engine: &Engine,
    families: &IndustryFamilies,
    actor: ObjId,
) -> Vec<ClaimId> {
    let mut out: Vec<ClaimId> = scan_all_claims(engine, families)
        .into_iter()
        .filter(|claim| mush_economy::acts_for(&engine.world, actor, claim.issuer()))
        .map(|claim| claim.id())
        .collect();
    out.sort_unstable();
    out.dedup();
    out.truncate(MAX_DEPOSIT_CHOICES);
    out
}

/// The authorities a **non-staff** actor may file a claim under: the
/// organizations it is a member of, plus the organizations it holds a direct
/// capability grant in, filtered by `acts_for` — which for an organization means
/// holding `economy.account.post` on its root compartment.
///
/// Both halves come from `OrgState`'s reverse indexes (`member_orgs`,
/// `grant_orgs`), never from a walk of `orgs()`: the cost is `O(this actor's
/// orgs)` and does not grow with the size of the world. Sorted by id, deduped and
/// capped at [`MAX_DEPOSIT_CHOICES`], so the offer is deterministic and bounded.
///
/// An `Object` principal is never in the list — not even the actor's own, and not
/// one it MUSH-controls. An issuer is an authority *over* the ground; a player who
/// could name themselves would be issuing tenure to themselves by fiat, which is
/// the thing the whole gate exists to refuse. Staff are not restricted here: this
/// feeds the non-staff `Choice` only.
#[must_use]
pub fn issuable_authorities(engine: &Engine, actor: ObjId) -> Vec<PrincipalId> {
    let Some(store) = engine.world.module_state::<mush_org::OrgStore>() else {
        return Vec::new();
    };
    let state = store.state();
    let mut orgs: Vec<mush_org::OrgId> = state
        .member_orgs(actor)
        .chain(state.grant_orgs(actor))
        .collect();
    orgs.sort_unstable();
    orgs.dedup();
    // Capped **before** the filter, not after: `acts_for` is a capability lookup
    // per candidate, run on the world thread once per projection, so the cap has
    // to bound the probes and not merely the answer. The cost is that an org the
    // filter would have dropped can shorten the offered list — the same
    // truncate-and-accept posture the sibling `administrable_claims` takes.
    orgs.truncate(MAX_DEPOSIT_CHOICES);
    orgs.into_iter()
        .map(PrincipalId::Organization)
        .filter(|who| mush_economy::acts_for(&engine.world, actor, who))
        .collect()
}

/// The issuer gate's structural half, run inside `file()` before anything is
/// written.
///
/// It compares the submitted `issuer` against the offer's own fixed parameters —
/// [`CLAIM_FILED_BY`], [`CLAIM_FILE_STAFF`] and [`CLAIM_ISSUERS`], which
/// `merge_parameters` will not let a submission overwrite — so the authority a
/// claim is filed under is one the **projection** vouched for, not one the
/// parameters merely assert. An invocation that reached the provider by some other
/// route carries no such list and is refused outright.
///
/// Every arm answers the identical `REFUSED` line: which of the three failed is
/// not a fact a refused filer gets to learn.
fn authorize_claim_file(
    invocation: &ActionInvocation,
    parameters: &serde_json::Map<String, Value>,
    issuer: &PrincipalId,
) -> Result<(), ActionError> {
    let actor = invocation.actor();
    let unauthorized = || error(ActionErrorCode::Unauthorized, REFUSED);
    // Nobody issues their own tenure, staff included (the command refuses this
    // first; this is the half a hand-built invocation still meets).
    if issuer == &PrincipalId::Object(actor) {
        return Err(unauthorized());
    }
    // The offer was projected to *this* actor: a replayed invocation cannot carry
    // one filer's authority list under another filer's name.
    let filed_by = parameters
        .get(CLAIM_FILED_BY)
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .map(ObjId);
    if filed_by != Some(actor) {
        return Err(unauthorized());
    }
    if parameters
        .get(CLAIM_FILE_STAFF)
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let token = mush_economy::principal_token(issuer);
    let vouched = parameters
        .get(CLAIM_ISSUERS)
        .and_then(Value::as_array)
        .is_some_and(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .any(|allowed| allowed == token.as_str())
        });
    if vouched { Ok(()) } else { Err(unauthorized()) }
}

/// **This actor's orders that are holding production resources with no live budget
/// hold** (independent verify fold, D4) — the delivery offer's server-authored
/// refusal list, rendered as decimal strings so it compares against the same
/// `order` text the submission carries.
///
/// It runs where a world exists and the submitter does not: inside `project_offer`.
/// `scan_all_claims`' bounded-page shape, filtered to the requester so one player's
/// projection never walks another's book, and truncated at
/// [`crate::money::MAX_UNFUNDED_CHOICES`].
#[must_use]
fn unfunded_orders(engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<String> {
    let requester = PrincipalId::Object(actor);
    let state = engine.action_state();
    let mut out: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_LIST_PAGES || out.len() >= crate::money::MAX_UNFUNDED_CHOICES
        {
            break;
        }
        pages += 1;
        let Ok(page) = state.scan_family(
            &families.work_orders,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) else {
            return out;
        };
        for entry in page.entries() {
            let order = entry.value();
            if order.requester() == &requester
                && crate::money::budget_missing_in(&engine.world, order)
                && out.len() < crate::money::MAX_UNFUNDED_CHOICES
            {
                out.push(order.id().0.to_string());
            }
        }
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    out
}

/// The transferable claims a non-staff actor may move the **holder** of: those
/// it holds or `acts_for` the holder of (E7c3 verify fold C4). The authority
/// half stays E4's `AssetOp::TransferTitle`, posted by the command; this gate
/// keeps the record-half offer from being open to every actor.
#[must_use]
pub fn movable_claims(engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId> {
    let mut out: Vec<ClaimId> = scan_all_claims(engine, families)
        .into_iter()
        .filter(|claim| {
            claim.transferability() == Transferability::Transferable
                && mush_economy::acts_for(&engine.world, actor, claim.holder())
        })
        .map(|claim| claim.id())
        .collect();
    out.sort_unstable();
    out.dedup();
    out.truncate(MAX_DEPOSIT_CHOICES);
    out
}

// ---- WS02-S4: the work-order draft offer's server-authored books ------------
//
// The `WoDraft` arm used to declare thirteen free parameters and nothing else:
// every one of them client-supplied, no facility gate, no recipe gate, and the
// command's three checks (owner/`acts_for`, quarantine, the per-requester cap)
// reachable only by typing `@wo/draft`. A direct offer submission met none of
// them. The three layers below are `ClaimFile`'s, applied here:
//
// 1. the **command** still refuses first, with its own diagnostics;
// 2. the **offer** refuses to exist at all for an actor with no facility to
//    draft at — the same nothing a verb that does not exist projects, so the
//    projection is not a facility-existence oracle either;
// 3. the **books** ride the offer's fixed parameters, which `merge_parameters`
//    refuses to let a submitter collide with, so `wo_draft` re-derives the
//    drafter's authority from the offer that produced the invocation instead of
//    trusting the parameters beside it.

/// One published recipe, reduced to what an offer has to carry: its key, the
/// hash of its pinned lines, and the facility capabilities it needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntry {
    /// `"<pack>/<version>/<id>"`.
    pub key: String,
    /// The 32-hex prefix of the pinned lines' canonical hash.
    pub hash: String,
    /// The pack digest the authority publishes for it — the value `wo_draft`
    /// pins on the `RecipeRef`, read from here and never from the client.
    pub digest: DefinitionDigest,
    /// What a facility must offer to run it.
    pub requires: std::collections::BTreeSet<CanonicalId>,
}

/// The four books one `WoDraft` offer publishes, and whether the recipe book
/// left anything out.
#[derive(Clone, Debug, Default)]
pub struct WoOfferBooks {
    /// `id → {slots, capacity_revision, policy_revision, capabilities}`.
    pub facilities: serde_json::Map<String, Value>,
    /// The drafter's live endpoints, ascending.
    pub endpoints: Vec<u64>,
    /// The drafter's own active accounts, ascending.
    pub accounts: Vec<u64>,
    /// `"<pack>/<version>/<id>" → {hash, digest}` — the 32-hex pinned hash and
    /// the 64-hex pack digest, both the authority's.
    pub recipes: serde_json::Map<String, Value>,
    /// Whether a cap bit — [`MAX_OFFERED_RECIPES`] or
    /// [`MAX_WO_OFFER_BOOK_BYTES`].
    pub truncated: bool,
}

/// The 32-hex identity of one pinned recipe, the value the recipe book carries.
///
/// # Errors
/// [`ActionError`] if the pinned lines will not canonicalize inside
/// `MAX_ACTION_PARAMETER_BYTES` — which a constructed [`PinnedRecipe`] cannot do.
pub fn pinned_hash(pinned: &PinnedRecipe) -> Result<String, ActionError> {
    let value =
        serde_json::to_value(pinned).map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
    let hash = CanonicalHash::for_canonical_value(
        "industry.pinned",
        1,
        &value,
        MAX_ACTION_PARAMETER_BYTES,
    )
    .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
    Ok(hash.to_hex().chars().take(32).collect())
}

/// The published catalog, reduced to [`CatalogEntry`] rows in key order.
///
/// Deterministic: the authority answers `(pack, version, id)` order and this
/// sorts on the rendered key anyway, so two walks of one unchanged world produce
/// the identical vector — which is what makes the offer's cache and its
/// byte-identical projections true rather than hopeful.
#[must_use]
pub fn catalog_entries(
    recipes: &dyn crate::workorder::RecipeAuthority,
    world: &World,
) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = recipes
        .catalog(world)
        .into_iter()
        .filter_map(|(reference, pinned)| {
            Some(CatalogEntry {
                key: recipe_key(&reference),
                hash: pinned_hash(&pinned).ok()?,
                digest: reference.digest(),
                requires: pinned.requires().clone(),
            })
        })
        .collect();
    out.sort_by(|left, right| left.key.cmp(&right.key));
    out.dedup_by(|left, right| left.key == right.key);
    out
}

/// The book key one recipe reference is spelled under.
#[must_use]
pub fn recipe_key(reference: &RecipeRef) -> String {
    recipe_book_key(reference.pack(), reference.version(), reference.id())
}

/// The same key, spelled from the parts a submission carries — `wo_draft` has no
/// [`RecipeRef`] to ask until it has read the digest out of the book this key
/// finds. One function so the projection's spelling and the prepare's cannot
/// drift apart.
#[must_use]
fn recipe_book_key(pack: &CanonicalId, version: u32, id: &CanonicalId) -> String {
    format!("{}/{version}/{}", pack.as_str(), id.as_str())
}

/// **The facilities this actor may draft at right now**: staff, the owner, or a
/// principal the actor `acts_for` — `commands.rs`'s own predicate, minus the
/// quarantined ones, bounded at [`MAX_WO_OFFER_FACILITIES`].
///
/// An empty answer is what makes the offer disappear, so a stranger sees exactly
/// what an actor looking at a facility that does not exist sees.
#[must_use]
pub fn draftable_facilities(
    engine: &Engine,
    actor: ObjId,
    staff: bool,
) -> Vec<crate::facility::FacilityRecord> {
    let Some(store) = engine.world.module_state::<crate::store::IndustryStore>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for facility in store.facilities() {
        if out.len() >= MAX_WO_OFFER_FACILITIES {
            break;
        }
        // R4: a facility whose capacity book was lost authorizes nobody — the
        // command's refusal, in the shape the offer can express it.
        if store.is_facility_quarantined(facility.object()) {
            continue;
        }
        if !(staff || mush_economy::acts_for(&engine.world, actor, facility.owner())) {
            continue;
        }
        out.push(facility.clone());
    }
    out
}

/// Build the four books for one drafter, from the facilities they may draft at
/// and the hashed catalog.
///
/// The recipe book is scoped to what those facilities can actually run, sorted,
/// and cut at [`MAX_OFFERED_RECIPES`] or [`MAX_WO_OFFER_BOOK_BYTES`], whichever
/// bites first. A recipe outside the cut is undraftable and refuses with the
/// byte-identical `WO_REFUSED`; [`WoOfferBooks::truncated`] and the log line the
/// offer emits are how an operator learns that is why.
#[must_use]
pub fn wo_offer_books(
    engine: &Engine,
    actor: ObjId,
    facilities: &[crate::facility::FacilityRecord],
    catalog: &[CatalogEntry],
) -> WoOfferBooks {
    let mut books = WoOfferBooks::default();
    for facility in facilities {
        books.facilities.insert(
            facility.object().0.to_string(),
            json!({
                "slots": facility.maximum_slots(),
                "capacity_revision": facility.capacity_revision(),
                "policy_revision": facility.policy_revision(),
                "capabilities": facility
                    .capabilities()
                    .iter()
                    .map(CanonicalId::as_str)
                    .collect::<Vec<_>>(),
            }),
        );
    }
    books.endpoints = mush_inventory::live_endpoints(&engine.world, actor)
        .into_iter()
        .take(MAX_WO_OFFER_ENDPOINTS)
        .map(|vault| vault.0)
        .collect();
    books.accounts = drafter_accounts(&engine.world, actor);
    let mut bytes = 0_usize;
    for entry in catalog {
        if !facilities
            .iter()
            .any(|facility| facility.offers(&entry.requires))
        {
            continue;
        }
        if books.recipes.len() >= MAX_OFFERED_RECIPES {
            books.truncated = true;
            break;
        }
        // The row's own weight, quotes and separators included — cheap, and it
        // bounds the book on the axis the offer's canonical limit cares about.
        let digest = entry.digest.to_hex();
        let weight = entry.key.len() + entry.hash.len() + digest.len() + 24;
        if bytes + weight > MAX_WO_OFFER_BOOK_BYTES {
            books.truncated = true;
            break;
        }
        bytes += weight;
        books.recipes.insert(
            entry.key.clone(),
            json!({ "hash": entry.hash.clone(), "digest": digest }),
        );
    }
    books
}

/// The drafter's **own** active accounts — the set `money::check_wo_accounts`
/// will accept as a budget, since a work order's hold is a reservation on the
/// requester's own account, staff included.
#[must_use]
fn drafter_accounts(world: &World, actor: ObjId) -> Vec<u64> {
    let owner = PrincipalId::Object(actor);
    let Some(store) = world.module_state::<mush_economy::EconomyStore>() else {
        return Vec::new();
    };
    let mut out: Vec<u64> = Vec::new();
    for account in store.snapshot().accounts() {
        if out.len() >= MAX_WO_OFFER_ACCOUNTS {
            break;
        }
        if account.owner == owner && matches!(account.status, mush_economy::AccountStatus::Active) {
            out.push(account.id.0);
        }
    }
    out.sort_unstable();
    out
}

/// Everything one `WoDraft` submission's **books** say about the facility and the
/// recipe it names — every number the order goes on to pin, read off the offer
/// the projection minted rather than off the parameters beside it.
#[derive(Clone, Debug)]
struct DraftAuthority {
    /// The facility's own concurrent-slot cap.
    facility_slots: u32,
    /// The facility's capacity staleness stamp.
    capacity_revision: u64,
    /// The facility's policy staleness stamp.
    policy_revision: u64,
    /// What the facility can run, for the `requires` subset test.
    capabilities: std::collections::BTreeSet<CanonicalId>,
    /// The authority's published pack digest for the named recipe.
    digest: DefinitionDigest,
    /// The 32-hex canonical hash of the authority's pinned lines.
    pinned_hash: String,
}

/// **The draft's structural gate** — `file()`'s shape, for the facility instead
/// of the issuer: re-derive the drafter's authority from the offer that produced
/// this invocation rather than from the parameters beside it.
///
/// A `prepare_authoritative` provider gets no world, so it cannot ask `acts_for`
/// anything. What it can do is refuse a facility or a recipe the projection that
/// minted this offer did not publish, and **read the numbers out of the books
/// instead of off the wire** — a submitter cannot reach either, because
/// `merge_parameters` refuses a supplied key that collides with a fixed one.
/// Every refusal is the byte-identical `WO_REFUSED`.
///
/// ---- WS02-S5: the books are the source, not a cross-check ----
///
/// S4 published these books and compared them against client-supplied
/// `facility_slots`, `capacity_revision`, `policy_revision` and `digest`. That
/// left the one line nobody compared — `pinned` — free, and `pinned` is what
/// `wo_deliver` mints from. Those four parameters are gone from the projection
/// now (so `ProjectedParameterSchema::validate` refuses a submission that even
/// mentions one) and this function *returns* them, so the only way a number
/// reaches a `WorkOrder` is through a book the server wrote.
///
/// The staff shortcut went with them. It could only ever have skipped straight
/// past values that no longer exist to skip to; staff authority is expressed by
/// `draftable_facilities` putting **every** facility in a staff drafter's book,
/// which is where it belonged.
fn wo_draft_authority(
    parameters: &serde_json::Map<String, Value>,
    actor: ObjId,
    facility: ObjId,
    recipe_key: &str,
) -> Result<DraftAuthority, ActionError> {
    let refused = || error(ActionErrorCode::Unauthorized, WO_REFUSED);
    // The offer was projected to *this* actor: a replayed invocation cannot carry
    // one drafter's books under another drafter's name.
    let drafter = parameters
        .get(WO_DRAFTER)
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .map(ObjId);
    if drafter != Some(actor) {
        return Err(refused());
    }
    let row = parameters
        .get(WO_FACILITIES)
        .and_then(Value::as_object)
        .and_then(|book| book.get(&facility.0.to_string()))
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    // The staleness stamps and the slot cap are the record's, full stop: an offer
    // route could otherwise pin a 1-slot facility as an 8-slot one, or pin
    // revisions that never existed so the submit's drift check could not fire.
    let facility_slots = row
        .get("slots")
        .and_then(Value::as_u64)
        .and_then(|slots| u32::try_from(slots).ok())
        .filter(|slots| *slots >= 1 && *slots <= crate::facility::MAX_FACILITY_SLOTS)
        .ok_or_else(refused)?;
    let capacity_revision = row
        .get("capacity_revision")
        .and_then(Value::as_u64)
        .filter(|revision| *revision >= 1)
        .ok_or_else(refused)?;
    let policy_revision = row
        .get("policy_revision")
        .and_then(Value::as_u64)
        .filter(|revision| *revision >= 1)
        .ok_or_else(refused)?;
    let capabilities = row
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(refused)?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .and_then(|id| CanonicalId::new(id).ok())
                .ok_or_else(refused)
        })
        .collect::<Result<std::collections::BTreeSet<CanonicalId>, ActionError>>()?;
    let recipe = parameters
        .get(WO_RECIPES)
        .and_then(Value::as_object)
        .and_then(|book| book.get(recipe_key))
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let pinned_hash = recipe
        .get("hash")
        .and_then(Value::as_str)
        .ok_or_else(refused)?
        .to_owned();
    let digest = recipe
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|hex| DefinitionDigest::from_hex(hex).ok())
        .ok_or_else(refused)?;
    Ok(DraftAuthority {
        facility_slots,
        capacity_revision,
        policy_revision,
        capabilities,
        digest,
        pinned_hash,
    })
}

impl IndustryProvider {
    fn parameters<'i>(
        &self,
        invocation: &'i ActionInvocation,
    ) -> Result<&'i serde_json::Map<String, Value>, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "an industry action is self-scoped and takes no targets",
            ));
        }
        invocation.parameters().as_object().ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "industry parameters must be an object",
            )
        })
    }

    /// The hashed catalog and the revision it was walked at, from the cache when
    /// no pack has been published since.
    ///
    /// A poisoned lock is not a crash on the one task that must never panic: the
    /// walk simply happens again, which is slower and just as correct.
    fn catalog_book(&self, world: &World) -> (u64, std::sync::Arc<Vec<CatalogEntry>>) {
        let Some(recipes) = self.recipes.as_ref() else {
            return (0, std::sync::Arc::new(Vec::new()));
        };
        let revision = recipes.catalog_revision(world);
        if let Ok(guard) = self.catalog.lock()
            && let Some((cached, book)) = guard.as_ref()
            && *cached == revision
        {
            return (revision, std::sync::Arc::clone(book));
        }
        let book = std::sync::Arc::new(catalog_entries(recipes.as_ref(), world));
        if let Ok(mut guard) = self.catalog.lock() {
            *guard = Some((revision, std::sync::Arc::clone(&book)));
        }
        (revision, book)
    }

    /// Say once per pack publish that a drafter's recipe book was cut short —
    /// the operator-facing half of the truncation, which no player sees.
    fn warn_truncated(&self, revision: u64, actor: ObjId, offered: usize) {
        let Ok(mut guard) = self.truncation_warned.lock() else {
            return;
        };
        if *guard == Some(revision) {
            return;
        }
        *guard = Some(revision);
        tracing::warn!(
            target: "industry",
            actor = actor.0,
            offered,
            cap = MAX_OFFERED_RECIPES,
            bytes = MAX_WO_OFFER_BOOK_BYTES,
            "industry: a work-order draft offer's recipe book was truncated; \
             recipes past the cut cannot be drafted"
        );
    }
}

impl ActionProvider for IndustryProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let actor = query.actor();
        let staff = crate::store::is_staff(&engine.world, actor);
        let mut parameters = Vec::new();
        // The `ClaimFile` offer's server-authored authority list, published below
        // as a fixed parameter so `prepare_authoritative` can re-check it.
        let mut claim_issuers: Vec<String> = Vec::new();
        // The `WoDraft` offer's server-authored books, published the same way and
        // for the same reason.
        let mut wo_books: Option<WoOfferBooks> = None;
        match self.verb {
            IndustryVerb::DepositOpen => {
                // Opening an ore body is a staff act: it credits a pool from
                // nothing, which is the one thing no player may do.
                if !staff {
                    return Ok(None);
                }
                parameters.push(integer("place", 0, i64::from(u32::MAX))?);
                parameters.push(text("commodity", MAX_COMMODITY_BYTES)?);
                parameters.push(integer("grade", 0, i64::from(mush_inventory::MAX_GRADE))?);
                parameters.push(integer("accessibility", 0, i64::from(MAX_ACCESSIBILITY))?);
                parameters.push(text("grade_class", MAX_BAND_BYTES)?);
                parameters.push(text("reserve_class", MAX_BAND_BYTES)?);
                parameters.push(text("curve", MAX_CURVE_BYTES)?);
                parameters.push(text("endpoint", 32)?);
                parameters.push(integer(
                    "reserve",
                    1,
                    i64::try_from(MAX_ORE_RESERVE).unwrap_or(i64::MAX),
                )?);
            }
            IndustryVerb::DepositAccess => {
                if !staff {
                    return Ok(None);
                }
                parameters.push(text("deposit", 32)?);
                parameters.push(integer("accessibility", 0, i64::from(MAX_ACCESSIBILITY))?);
                parameters.push(text("event", MAX_REASON_BYTES)?);
            }
            IndustryVerb::ClaimFile => {
                parameters.push(text("subject", 40)?);
                // ---- The issuer gate, layer 2: the offer itself ----
                //
                // Free text here let any actor file tenure in any government's
                // name. Staff may still name any authority by token; every other
                // actor is offered a `Choice` over exactly the organizations it
                // `acts_for`, so a forged issuer is refused by
                // `ProjectedParameterSchema::validate` before the provider runs.
                // An actor who speaks for nobody gets **no offer at all** — the
                // same nothing an actor sees for a verb that does not exist, so
                // the projection is not an org-membership oracle either.
                if staff {
                    parameters.push(text("issuer", 64)?);
                } else {
                    claim_issuers = issuable_authorities(engine, actor)
                        .iter()
                        .map(|who| mush_economy::principal_token(who).to_string())
                        .collect();
                    if claim_issuers.is_empty() {
                        return Ok(None);
                    }
                    parameters.push(parameter(
                        "issuer",
                        true,
                        mush_core::ProjectedParameterKind::Choice {
                            choices: claim_issuers.clone(),
                        },
                    )?);
                }
                parameters.push(text("holder", 64)?);
                parameters.push(integer("object", 0, i64::from(u32::MAX))?);
                parameters.push(text("exclusivity", 16)?);
                parameters.push(integer("priority", 0, i64::from(MAX_CLAIM_PRIORITY))?);
                parameters.push(integer("term", i64::MIN, i64::MAX)?);
                parameters.push(text("transferability", 16)?);
            }
            IndustryVerb::ClaimMove => {
                // E7c3 verify fold C4: the record half of a transfer is gated
                // here — a non-staff actor is offered a `Choice` only over the
                // transferable claims it holds, so the offer cannot be selected
                // by a non-holder to rewrite `record.holder` and split tenure
                // from title. The authority half stays E4's `TransferTitle`,
                // which the command posts first. Staff may name any claim by id.
                if staff {
                    parameters.push(text("claim", 32)?);
                } else {
                    let claims = movable_claims(engine, &self.families, actor);
                    if claims.is_empty() {
                        return Ok(None);
                    }
                    parameters.push(parameter(
                        "claim",
                        true,
                        mush_core::ProjectedParameterKind::Choice {
                            choices: claims.iter().map(ToString::to_string).collect(),
                        },
                    )?);
                }
                parameters.push(text("holder", 64)?);
                parameters.push(integer("revision", 1, i64::MAX)?);
            }
            IndustryVerb::ClaimContest => {
                // E7c3 verify fold C1: the whole contest path — suspend, seize,
                // contest, release — is gated here, the seam that has `&Engine`
                // (`prepare_authoritative` gets no world). Staff may contest any
                // claim by id; every other actor is offered a `Choice` only over
                // the claims whose `issuer()` its principal `acts_for`, so naming
                // a foreign claim is refused identically whether it exists or not,
                // and no griefer can forge an issuer's suspension or seizure or
                // undo a real authority's.
                if staff {
                    parameters.push(text("claim", 32)?);
                } else {
                    let claims = administrable_claims(engine, &self.families, actor);
                    if claims.is_empty() {
                        return Ok(None);
                    }
                    parameters.push(parameter(
                        "claim",
                        true,
                        mush_core::ProjectedParameterKind::Choice {
                            choices: claims.iter().map(ToString::to_string).collect(),
                        },
                    )?);
                }
                parameters.push(text("state", 16)?);
                parameters.push(integer("revision", 1, i64::MAX)?);
                parameters.push(parameter(
                    "contract",
                    false,
                    mush_core::ProjectedParameterKind::Integer {
                        minimum: 0,
                        maximum: i64::MAX,
                    },
                )?);
            }
            IndustryVerb::ExtractStart | IndustryVerb::ExtractCycle => {
                let operator = PrincipalId::Object(actor);
                let bodies = authorized_deposits(engine, &self.families, actor, &operator);
                if bodies.is_empty() {
                    // No authorized body: **no offer at all**. A viewer whose
                    // licence lapsed sees exactly what a viewer who never had
                    // one sees, and both see what a viewer looking at a body
                    // that does not exist sees.
                    return Ok(None);
                }
                parameters.push(parameter(
                    "deposit",
                    true,
                    mush_core::ProjectedParameterKind::Choice {
                        choices: bodies.iter().map(ToString::to_string).collect(),
                    },
                )?);
                if self.verb == IndustryVerb::ExtractStart {
                    // The destination is a `Choice` over the actor's own live
                    // endpoints, on cut 2's exact pattern, for a reason that is
                    // not decoration: `mint_origin` CREATES matter, so a free
                    // text field would let an operator mint lots into somebody
                    // else's vault — a gift they cannot refuse and, at 512 lots
                    // per endpoint, a denial they cannot undo. An actor with no
                    // live endpoint of their own gets no offer at all.
                    let endpoints = mush_inventory::live_endpoints(&engine.world, actor);
                    if endpoints.is_empty() {
                        return Ok(None);
                    }
                    parameters.push(parameter(
                        "endpoint",
                        true,
                        mush_core::ProjectedParameterKind::Choice {
                            choices: endpoints.iter().map(|vault| vault.0.to_string()).collect(),
                        },
                    )?);
                }
            }
            IndustryVerb::ExtractStop => {
                parameters.push(text("deposit", 32)?);
            }
            IndustryVerb::WoDraft => {
                // ---- The facility gate, layer 2: the offer itself (WS02-S4) ----
                //
                // The cheap path FIRST, and it is not a style preference:
                // `project_legal_actions` runs this on every projection pass,
                // including every authoritative command submission and every
                // decision capture. An actor with no facility to draft at gets
                // **no offer at all** and no book is ever built for them.
                //
                // `facility` stays an integer: a `Choice` must be a string, and
                // the command stages an integer, so making it a choice would
                // refuse every `@wo/draft` on the parameter schema.
                let facilities = draftable_facilities(engine, actor, staff);
                if facilities.is_empty() {
                    return Ok(None);
                }
                // C2/D2 (m2): the per-requester cap, read off the RAM index the
                // store maintains on open/terminate rather than walked here —
                // the command's own bounded scan stays the authority, this is
                // the projection-cheap read of the same number.
                let open = engine
                    .world
                    .module_state::<crate::store::IndustryStore>()
                    .map_or(0, |store| {
                        store.open_work_orders(&PrincipalId::Object(actor))
                    });
                if open >= crate::workorder::MAX_WORK_ORDERS_PER_REQUESTER {
                    return Ok(None);
                }
                let (revision, catalog) = self.catalog_book(&engine.world);
                let books = wo_offer_books(engine, actor, &facilities, &catalog);
                if books.recipes.is_empty() {
                    // Nothing this actor's facilities can run: no offer, for the
                    // same fog reason an unauthorized facility gets none.
                    return Ok(None);
                }
                if books.truncated {
                    self.warn_truncated(revision, actor, books.recipes.len());
                }
                wo_books = Some(books);
                // The draft command (world available) resolves the recipe from the
                // authority, then submits these as parameters; the provider
                // re-derives every pinned number from the books above and writes
                // the Draft work order. `pinned` carries the resolved lines as a
                // JSON blob, which the provider hashes against the book's.
                //
                // WS02-S5: `facility_slots`, `capacity_revision`,
                // `policy_revision` and `digest` are **not** declared here any
                // more. `ProjectedParameterSchema::validate` refuses a supplied
                // key the projection does not name, so their absence is itself the
                // fence: a submission that so much as mentions `capacity_revision`
                // is refused before the provider runs, and `wo_draft` reads all
                // four out of `wo_facilities`/`wo_recipes` instead.
                parameters.push(integer("facility", 0, i64::from(u32::MAX))?);
                parameters.push(text("pack", 64)?);
                parameters.push(integer("version", 1, i64::from(u32::MAX))?);
                parameters.push(text("recipe", 64)?);
                parameters.push(integer(
                    "quantity",
                    1,
                    i64::from(crate::workorder::MAX_WORK_ORDER_QUANTITY),
                )?);
                parameters.push(integer("account", 0, i64::MAX)?);
                parameters.push(integer("source", 0, i64::MAX)?);
                parameters.push(integer("destination", 0, i64::MAX)?);
                parameters.push(text("beneficiary", 64)?);
                parameters.push(text("pinned", 4096)?);
            }
            IndustryVerb::WoSubmit
            | IndustryVerb::WoDeliver
            | IndustryVerb::WoCancel
            | IndustryVerb::WoRestamp => {
                parameters.push(text("order", 32)?);
                if self.verb == IndustryVerb::WoRestamp {
                    parameters.push(text("policy", 64)?);
                }
                // ---- E7r1 fold Z2: the SUBMIT ingress declares the hold ----
                //
                // Found by the fold's own grounding, not on the verify's list, and it
                // is why B15's "unusable by construction" is only half the story: a
                // command's submission takes the SAME route as a direct offer
                // (`Ctx::submit_authoritative` -> `command_offer_submission` ->
                // `resolve_legal_action`), and `ProjectedParameterSchema::validate`
                // refuses a supplied key the projection does not declare. `@wo/submit`
                // stages `{"order":…, "hold":…}` against a projection that declared
                // only `order`, so on a real server EVERY `@wo/submit` was refused
                // `InvalidParameters` before it reached the provider — R1's whole
                // money leg was unreachable outside the tests, which stage the
                // submission and never resolve it.
                //
                // Declaring it makes the verb work. It does NOT make the offer route
                // a free-production hole, because the id is not the authority:
                // `money::live_hold` finds the reservation **by purpose**, so a forged
                // id records a `HoldId` that no hold answers to, `money::budget_missing`
                // is then true for that order for ever, `wo_deliver` refuses it
                // through `money::UNFUNDED_ORDERS` below, and the sweep pages it. The
                // prepare's `hold != 0` check stays exactly what it always was — a
                // shape assertion — and now says so.
                if self.verb == IndustryVerb::WoSubmit {
                    parameters.push(integer("hold", 0, i64::MAX)?);
                }
            }
            IndustryVerb::WoAdvance => {
                // The production advance is **not** player-offered: it enters only
                // through the schedule source's adapter `plan`, on the scheduler
                // principal. No offer at all.
                return Ok(None);
            }
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "industry definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "industry guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "industry definition guard is unavailable",
                )
            })?;
        let (label, help) = self.verb.label();
        // ---- E7r1 fold D4: the delivery offer's unfunded list ----
        //
        // Server-authored, computed HERE because this is the only place on the
        // prepared path that holds a world, and unforgeable because
        // `merge_parameters` refuses a submitted key that collides with a fixed one.
        // `wo_deliver` refuses any order it names, so a direct offer submission can no
        // longer materialise output for an order whose budget reservation vanished —
        // the hole R1's own comment believed the `hold != 0` check had closed.
        let mut fixed = json!({ "now": engine.world.world_ic_now() });
        if self.verb == IndustryVerb::WoDeliver
            && let Some(map) = fixed.as_object_mut()
        {
            map.insert(
                crate::money::UNFUNDED_ORDERS.to_owned(),
                json!(unfunded_orders(engine, &self.families, actor)),
            );
        }
        // ---- The issuer gate, layer 3: the structural half ----
        //
        // The same server-authored trick, for the same reason: `merge_parameters`
        // refuses a submitted key that collides with a fixed one, so these three
        // reach `file()` exactly as this projection wrote them. The `Choice` above
        // already refuses a forged issuer on the offer route; this is what makes
        // the rule structural rather than presentational — `file()` re-derives the
        // authority from the offer that produced the invocation instead of
        // trusting the parameter beside it.
        if self.verb == IndustryVerb::ClaimFile
            && let Some(map) = fixed.as_object_mut()
        {
            map.insert(CLAIM_FILED_BY.to_owned(), json!(actor.0));
            map.insert(CLAIM_FILE_STAFF.to_owned(), json!(staff));
            map.insert(CLAIM_ISSUERS.to_owned(), json!(claim_issuers));
        }
        // ---- The facility gate, layer 3: the structural half (WS02-S4) ----
        //
        // Same trick, same reason: `merge_parameters` refuses a submitted key
        // that collides with a fixed one, so these five reach `wo_draft` exactly
        // as this projection wrote them and a direct offer submission cannot
        // forge itself a facility, a revision pair or a recipe it was not
        // offered. Deterministic given an unchanged world: the books are sorted
        // maps over sorted sources, so two projections in one IC second produce
        // byte-identical fixed parameters — and therefore the identical offer id.
        if let Some(books) = wo_books
            && let Some(map) = fixed.as_object_mut()
        {
            map.insert(WO_DRAFTER.to_owned(), json!(actor.0));
            // WS02-S5: diagnostic, not a gate. It records that this book was
            // built with staff reach (every facility, not just the drafter's);
            // `wo_draft_authority` no longer shortcuts on it, because there is
            // nothing left to shortcut past — the numbers it used to wave
            // through are read out of the books themselves now.
            map.insert(WO_DRAFT_STAFF.to_owned(), json!(staff));
            map.insert(WO_FACILITIES.to_owned(), Value::Object(books.facilities));
            map.insert(WO_ENDPOINTS.to_owned(), json!(books.endpoints));
            map.insert(WO_ACCOUNTS.to_owned(), json!(books.accounts));
            map.insert(WO_RECIPES.to_owned(), Value::Object(books.recipes));
            map.insert(WO_RECIPES_TRUNCATED.to_owned(), json!(books.truncated));
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            // The IC stamp rides the offer's FIXED parameters, so preparation
            // never reaches for a clock it does not have and two replays of one
            // request stamp the identical marker.
            fixed_parameters: fixed,
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "industry parameter schema is invalid",
                )
            })?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(self.verb.verb(), label, help).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "industry presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "industry offer guards are invalid",
                )
            })?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &CorePrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(error(
                ActionErrorCode::Integrity,
                "industry receipt carries the wrong durable schema",
            ));
        }
        let actor = outcome
            .data()
            .get("actor")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| {
                error(
                    ActionErrorCode::Integrity,
                    "industry receipt names no actor",
                )
            })?;
        if viewer != &(CorePrincipalId::Object { id: actor }) {
            return Err(error(
                ActionErrorCode::Integrity,
                "industry receipt disagrees with its eligible viewer",
            ));
        }
        let summary = outcome
            .data()
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("Done.")
            .to_owned();
        mush_proto::ProjectedActionResult::new(self.verb.id(), summary, outcome.data().clone())
            .map(Some)
            .map_err(|_| {
                error(
                    ActionErrorCode::BoundsExceeded,
                    "industry receipt exceeds public bounds",
                )
            })
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = self.parameters(invocation)?.clone();
        let now = prepare_now(&parameters)?;
        let actor = invocation.actor();
        let mut data = BTreeMap::<String, Value>::new();
        data.insert("actor".to_owned(), json!(actor.0));
        data.insert("verb".to_owned(), json!(self.verb.id()));

        let summary = match self.verb {
            IndustryVerb::DepositOpen => self.open(txn, invocation, &parameters, &mut data)?,
            IndustryVerb::DepositAccess => self.access(txn, &parameters, &mut data)?,
            IndustryVerb::ClaimFile => self.file(txn, invocation, &parameters, &now, &mut data)?,
            IndustryVerb::ClaimMove => self.mv(txn, &parameters, &mut data)?,
            IndustryVerb::ClaimContest => self.contest(txn, &parameters, &now, actor, &mut data)?,
            IndustryVerb::ExtractStart => self.start(txn, &parameters, actor, &mut data)?,
            IndustryVerb::ExtractCycle => {
                self.cycle(txn, invocation, &parameters, &now, actor, &mut data)?
            }
            IndustryVerb::ExtractStop => self.stop(txn, &parameters, &mut data)?,
            IndustryVerb::WoDraft => {
                self.wo_draft(txn, invocation, &parameters, actor, &mut data)?
            }
            IndustryVerb::WoSubmit => self.wo_submit(txn, &parameters, &now, actor, &mut data)?,
            IndustryVerb::WoAdvance => self.wo_advance(txn, &parameters, &mut data)?,
            IndustryVerb::WoDeliver => {
                self.wo_deliver(txn, invocation, &parameters, &now, actor, &mut data)?
            }
            IndustryVerb::WoCancel => self.wo_cancel(txn, &parameters, actor, &mut data)?,
            IndustryVerb::WoRestamp => self.wo_restamp(txn, &parameters, &now, actor, &mut data)?,
        };
        data.insert("summary".to_owned(), json!(summary));
        let outcome = RulesValue::new(self.outcome.clone(), json!(data)).map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "industry outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl IndustryProvider {
    /// B2 — open one ore body and seed its endpoint pool, in one bundle.
    fn open(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let place = obj_field(parameters, "place")?;
        let commodity = commodity_field(parameters)?;
        let grade = parameters
            .get("grade")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .and_then(|value| mush_inventory::Grade::new(value).ok())
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, "that is not a grade"))?;
        let accessibility = accessibility_field(parameters)?;
        let grade_class = band_field(parameters, "grade_class")?;
        let reserve_class = band_field(parameters, "reserve_class")?;
        let curve = parse_curve(&text_field(parameters, "curve")?)?;
        let endpoint = endpoint_field(parameters, "endpoint")?;
        let reserve = u64_field(parameters, "reserve", MAX_ORE_RESERVE)?;
        let id = derive_deposit_id(invocation.request(), 0).map_err(|source| refuse(&source))?;
        let deposit = OreDeposit::new(
            id,
            place,
            commodity,
            grade,
            accessibility,
            grade_class,
            reserve_class,
            curve,
            endpoint,
            reserve,
        )
        .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))?;
        self.families
            .open_deposit(txn, deposit)
            .map_err(|source| refuse(&source))?;
        data.insert("deposit".to_owned(), json!(id.to_string()));
        Ok(format!("Opened {id}."))
    }

    /// B2 — the attributed journaled campaign event.
    ///
    /// It moves the **next** cycle's effective yield and never a completed one:
    /// a completed cycle's quantity is already a lot, and this arm writes no
    /// quantity at all.
    fn access(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let deposit = deposit_field(parameters, "deposit")?;
        let accessibility = accessibility_field(parameters)?;
        let event = text_field(parameters, "event")?;
        if event.is_empty() || event.len() > MAX_REASON_BYTES {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                format!("an accessibility event is named in 1..={MAX_REASON_BYTES} bytes"),
            ));
        }
        let live = self
            .families
            .deposit(txn, deposit)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        let next = self
            .families
            .set_accessibility(txn, deposit, live.revision(), accessibility)
            .map_err(|source| refuse(&source))?;
        data.insert("deposit".to_owned(), json!(deposit.to_string()));
        data.insert("event".to_owned(), json!(event));
        data.insert(
            "accessibility".to_owned(),
            json!(next.accessibility().get()),
        );
        Ok(format!(
            "{deposit} accessibility is now {}%.",
            next.accessibility().get()
        ))
    }

    /// B4 — the record half of filing a claim.
    ///
    /// The **title** half is E4's and was issued by the command handler through
    /// `mush_economy::issue_title` before this ran: cut 3 defines no second
    /// title record and no second transfer path. A crash between the two leaves
    /// a titled object with no claim record, which authorizes **nobody** — the
    /// fail-closed direction, chosen deliberately.
    fn file(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let subject = subject_field(parameters)?;
        let issuer = principal_field(parameters, "issuer")?;
        authorize_claim_file(invocation, parameters, &issuer)?;
        let holder = principal_field(parameters, "holder")?;
        let object = obj_field(parameters, "object")?;
        let exclusivity = match text_field(parameters, "exclusivity")?.as_str() {
            "exclusive" => Exclusivity::Exclusive,
            "shared" => Exclusivity::Shared,
            _ => {
                return Err(error(
                    ActionErrorCode::InvalidRequest,
                    "exclusivity is `exclusive` or `shared`",
                ));
            }
        };
        let transferability = match text_field(parameters, "transferability")?.as_str() {
            "transferable" => Transferability::Transferable,
            "bound" => Transferability::Bound,
            _ => {
                return Err(error(
                    ActionErrorCode::InvalidRequest,
                    "transferability is `transferable` or `bound`",
                ));
            }
        };
        let priority = parameters
            .get("priority")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value <= MAX_CLAIM_PRIORITY)
            .ok_or_else(|| {
                error(
                    ActionErrorCode::InvalidRequest,
                    format!("a claim priority is 0..={MAX_CLAIM_PRIORITY}"),
                )
            })?;
        let term = term_field(parameters, now)?;
        let id = derive_claim_id(invocation.request(), 0).map_err(|source| refuse(&source))?;

        // Exclusivity is enforced against the live set, not against a count:
        // one live exclusive claim per issuer per subject. Two governments may
        // still each issue one over the same ground — that is two issuers, and
        // it is exactly what §3.3 asks for without a `ControlKind`.
        for existing in self
            .families
            .claims_over(txn, subject)
            .map_err(|source| refuse(&source))?
        {
            if existing.issuer() == &issuer
                && existing.contest_state().authorizes()
                && (exclusivity == Exclusivity::Exclusive
                    || existing.exclusivity() == Exclusivity::Exclusive)
            {
                return Err(error(
                    ActionErrorCode::ProviderRejected,
                    "that authority already holds an exclusive claim over that subject",
                ));
            }
        }
        let claim = ClaimRecord::new(
            id,
            subject,
            issuer,
            holder,
            object,
            exclusivity,
            priority,
            term,
            transferability,
        )
        .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))?;
        self.families
            .file_claim(txn, claim)
            .map_err(|source| refuse(&source))?;
        data.insert("claim".to_owned(), json!(id.to_string()));
        data.insert("object".to_owned(), json!(object.0));
        Ok(format!("Filed {id}."))
    }

    /// B4 — the record half of a transfer.
    ///
    /// The **authority** half is E4's `AssetOp::TransferTitle` on the claim's
    /// titled object, which the command handler posts first and which refuses
    /// with `EconomyError::Encumbered` while **any** encumbrance stands. That is
    /// why a pledged claim cannot be transferred *by E4's rule, not by a second
    /// one*: this arm never runs.
    fn mv(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = claim_field(parameters, "claim")?;
        let holder = principal_field(parameters, "holder")?;
        let revision = revision_field(parameters)?;
        let live = self
            .families
            .claim(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        if live.transferability() != Transferability::Transferable {
            return Err(error(
                ActionErrorCode::ProviderRejected,
                "that tenure is personal to its holder",
            ));
        }
        let next = live
            .with_holder(holder)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_claim(txn, revision, next)
            .map_err(|source| refuse(&source))?;
        data.insert("claim".to_owned(), json!(id.to_string()));
        Ok(format!("{id} has a new holder."))
    }

    /// B4 — the contest-state machine.
    ///
    /// The pledge itself is E4's `Encumbrance::Collateral` on the titled
    /// object, and the command handler is the only place that can read an
    /// asset — so `Seized` on a **pledged** claim is refused there and pages
    /// level 2 with the command filled in, because seizure would silently
    /// destroy a lender's collateral.
    fn contest(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = claim_field(parameters, "claim")?;
        let revision = revision_field(parameters)?;
        let live = self
            .families
            .claim(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        // E7c3 verify fold C1: the contest is attributed to the **acting**
        // principal, not forged as the claim's issuer. `project_offer` has
        // already gated this verb to staff or an actor who `acts_for` the
        // issuer, so the acting object is the honest author of a suspension or
        // seizure — the previous `by: live.issuer()` attributed every griefed
        // suspension to an authority that never acted.
        let state = match text_field(parameters, "state")?.as_str() {
            "uncontested" => ContestState::Uncontested,
            "contested" => {
                let contract = parameters
                    .get("contract")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        error(
                            ActionErrorCode::InvalidRequest,
                            "a contest names its adjudicating contract",
                        )
                    })?;
                ContestState::Contested {
                    contract: ContractId(contract),
                }
            }
            "suspended" => ContestState::Suspended {
                by: PrincipalId::Object(actor),
            },
            "seized" => ContestState::Seized {
                by: PrincipalId::Object(actor),
                at: now.clone(),
            },
            _ => {
                return Err(error(
                    ActionErrorCode::InvalidRequest,
                    "a contest state is `uncontested`, `contested`, `suspended` or `seized`",
                ));
            }
        };
        let next = live
            .with_contest_state(state)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_claim(txn, revision, next)
            .map_err(|source| refuse(&source))?;
        data.insert("claim".to_owned(), json!(id.to_string()));
        Ok(format!("{id} moved."))
    }

    /// B6 — open a run. Writes no quantity.
    fn start(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let deposit = deposit_field(parameters, "deposit")?;
        let endpoint = endpoint_field(parameters, "endpoint")?;
        if self
            .families
            .deposit(txn, deposit)
            .map_err(|source| refuse(&source))?
            .is_none()
        {
            return Err(error(ActionErrorCode::ProviderRejected, REFUSED));
        }
        let run = ExtractionRun::new(deposit, PrincipalId::Object(actor), endpoint)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_run(txn, run)
            .map_err(|source| refuse(&source))?;
        data.insert("deposit".to_owned(), json!(deposit.to_string()));
        Ok(format!("Extraction opened at {deposit}."))
    }

    /// B6 — **one cycle, one bundle** (the D lens).
    fn cycle(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let deposit_id = deposit_field(parameters, "deposit")?;
        let deposit = self
            .families
            .deposit(txn, deposit_id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        let run = self
            .families
            .run(txn, deposit_id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        let holder = invocation.idempotency().clone();
        // Replay-stable and derived, never drawn: the commit token that marks a
        // consumed hold is this request's own identity, so a replayed cycle
        // presents the identical token and cut 2's own rule ("a `Consumed`
        // reservation replayed with the same commit token under a different
        // holder refuses") does the rest.
        let commit = mush_core::CommitToken::from_bytes(*invocation.request().as_bytes());
        let source = self.source(deposit_id, run.next_cycle());
        let cycled: Cycled = self
            .families
            .run_cycle(
                txn,
                &deposit,
                &run,
                actor,
                invocation.request(),
                &holder,
                now,
                commit,
                source,
            )
            .map_err(|source| refuse(&source))?;
        data.insert("deposit".to_owned(), json!(deposit_id.to_string()));
        data.insert("cycle".to_owned(), json!(cycled.cycle));
        data.insert("quantity".to_owned(), json!(cycled.quantity));
        data.insert("lot".to_owned(), json!(cycled.lot.to_string()));
        // Deliberately NOT the remaining reserve: that is the number the whole
        // fog budget was spent closing. The operator learns what they
        // extracted, never what is left.
        Ok(format!(
            "Cycle {} produced {} into {}.",
            cycled.cycle, cycled.quantity, cycled.lot
        ))
    }

    /// B6 — stop a run. The record stays.
    fn stop(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let deposit = deposit_field(parameters, "deposit")?;
        let run = self
            .families
            .run(txn, deposit)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, REFUSED))?;
        if run.state() == RunState::Stopped {
            return Err(error(ActionErrorCode::ProviderRejected, REFUSED));
        }
        let next = run
            .stopped()
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_run(txn, next)
            .map_err(|source| refuse(&source))?;
        data.insert("deposit".to_owned(), json!(deposit.to_string()));
        Ok(format!("Extraction stopped at {deposit}."))
    }

    /// The `ExternalRef` a minted lot's `Lineage::Origin` names.
    ///
    /// §4 rule 3: the retryable key shape is named here in the doc, not in a
    /// code review — `extract/{deposit}/{cycle}`, and cut 3 defines no second
    /// key shape.
    fn source(&self, deposit: OreDepositId, cycle: u64) -> ExternalRef {
        ExternalRef {
            provider: SmolStr::new_static(NS),
            kind: SmolStr::new_static("extraction.cycle"),
            id: SmolStr::new(format!("extract/{deposit}/{cycle:x}")),
            revision: Some(1),
        }
    }

    // ---- E7c4: the work-order lifecycle ----

    /// B3 — write one Draft work order from the resolved inputs the draft command
    /// computed (world available there; the provider has no world), with every
    /// number that governs production read back out of the offer's own books.
    ///
    /// ---- WS02-S5: `pinned` was the line nobody compared ----
    ///
    /// `wo_deliver` mints `order.pinned().outputs()` straight into the destination
    /// endpoint. Upstream grew two digest re-checks — one at `@wo/submit`, one at
    /// the plan-stage advance — and both compare the **digest**, so a forged
    /// digest fails closed. Neither has ever looked at the pinned lines. An honest
    /// `pack`/`version`/`recipe`/`digest` carried beside a `pinned` whose output
    /// units are multiplied passes both and mints the inflated amount: free
    /// matter, from a parameter the client wrote.
    ///
    /// So the pinned lines are checked against the authority here, at the only
    /// seam that has the book: decode, re-serialize through `serde_json::to_value`
    /// and hash under the same `industry.pinned` domain the projection used.
    /// Re-ordered keys and re-spaced JSON therefore hash identically and are
    /// accepted; anything that moves a unit count is a different hash and refuses.
    /// The refusal is [`ActionErrorCode::Stale`], because the honest cause is a
    /// pack republished between the projection and the submission and a driver
    /// should discard-and-redraft rather than treat it as a malformed request —
    /// and the player-visible text is the byte-identical `WO_REFUSED`, so a
    /// forgery and a drift read the same.
    fn wo_draft(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let refused = || error(ActionErrorCode::Unauthorized, WO_REFUSED);
        let facility = obj_field(parameters, "facility")?;
        let pack = canonical_field(parameters, "pack", 64)?;
        let version = u32::try_from(u64_field(parameters, "version", u64::from(u32::MAX))?)
            .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let recipe_id = canonical_field(parameters, "recipe", 64)?;
        // The book key is spelled from the submitted parameters and looked up; an
        // unpublished recipe refuses with the byte-identical line, so the book is
        // no catalog oracle either.
        let key = recipe_book_key(&pack, version, &recipe_id);
        let authority = wo_draft_authority(parameters, actor, facility, &key)?;
        let quantity = u32::try_from(u64_field(
            parameters,
            "quantity",
            u64::from(crate::workorder::MAX_WORK_ORDER_QUANTITY),
        )?)
        .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let account = AccountId(u64_field(parameters, "account", u64::MAX)?);
        let source = optional_endpoint(parameters, "source")?;
        let destination = DeliveryTarget::Endpoint {
            // Z4: read as the integer it is declared and staged as, never as text.
            endpoint: required_endpoint(parameters, "destination")?,
        };
        // `wo_submit` reserves every material input out of the order's `source`
        // and `wo_deliver` mints every output into its `destination`, neither with
        // an ownership test — free text here let a submission drain a rival's
        // endpoint or dump lots into one. Both, and the budget account, are
        // members of the drafter's own server-authored books or nothing.
        let endpoints = parameters
            .get(WO_ENDPOINTS)
            .and_then(Value::as_array)
            .ok_or_else(refused)?;
        let member = |wanted: u64| {
            endpoints
                .iter()
                .filter_map(Value::as_u64)
                .any(|id| id == wanted)
        };
        if !member(destination.endpoint().0.0) {
            return Err(refused());
        }
        if let Some(from) = source
            && !member(from.0.0)
        {
            return Err(refused());
        }
        if !parameters
            .get(WO_ACCOUNTS)
            .and_then(Value::as_array)
            .is_some_and(|book| {
                book.iter()
                    .filter_map(Value::as_u64)
                    .any(|id| id == account.0)
            })
        {
            return Err(refused());
        }
        let beneficiary = principal_field(parameters, "beneficiary")?;
        // Decoding is itself a gate: `PinnedRecipe`'s deserializer runs `new`, so a
        // zero-input recipe with output still refuses at `RecipeMintsFromNothing`.
        let pinned: PinnedRecipe = serde_json::from_str(&text_field(parameters, "pinned")?)
            .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        if pinned_hash(&pinned)? != authority.pinned_hash {
            return Err(error(ActionErrorCode::Stale, WO_REFUSED));
        }
        // The capability test the command has always run, applied on the route the
        // command does not stand in front of, against the book's capabilities.
        if !pinned.requires().is_subset(&authority.capabilities) {
            return Err(refused());
        }
        let recipe = RecipeRef::new(pack, version, recipe_id, authority.digest)
            .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))?;
        let id = derive_work_order_id(invocation.request(), 0).map_err(|source| refuse(&source))?;
        let order = WorkOrder::draft(
            id,
            PrincipalId::Object(actor),
            beneficiary,
            facility,
            authority.facility_slots,
            authority.capacity_revision,
            authority.policy_revision,
            recipe,
            pinned,
            quantity,
            account,
            source,
            destination,
            EconomicVisibility::Parties,
        )
        .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))?;
        self.families
            .open_work_order(txn, order)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Drafted {id}."))
    }

    /// B4 — acquire and start: reserve every material input (cut 2), one capacity
    /// slot (B1), open the production schedule, and transition Draft → Running —
    /// **one bundle**. The acquisition order is fixed and every reservation
    /// carries the work order's deterministic holder key.
    fn wo_submit(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = work_order_field(parameters, "order")?;
        let order = self
            .families
            .work_order(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, WO_REFUSED))?;
        self.require_requester(&order, actor)?;
        if *order.state() != WorkOrderState::Draft {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        // ---- E7r1: R1 — the money leg is not optional ----
        //
        // The submit command places the hold through `mush_economy::post` before it
        // submits, and passes the placed `HoldId` here. A prepare has no world and
        // cannot check that the id is live — the authority on that is
        // `money::live_hold`, found by purpose, which the sweep arm and `@wo/deliver`
        // both read. This check is therefore a **shape assertion**, not the ingress
        // guard, and the fold says so rather than leaving the old sentence
        // (*"that is what keeps the direct-offer ingress from acquiring resources for
        // free"*) to be believed (independent verify fold, B15).
        //
        // What actually closes the offer route is the **parameter schema**:
        // `ProjectedParameterSchema::validate` refuses any supplied parameter a
        // projection does not declare, and `IndustryVerb::WoSubmit`'s projection
        // declares only `order` — so an offer-route submission can never carry
        // `hold` at all, always reads 0 here, and always refuses. The corollary,
        // which nothing recorded: `WoSubmit`'s offer projection is **unusable by
        // construction**, and an `OfferRouteTable` entry naming it offers an action
        // that can only ever refuse. If the ingress is ever opened, either project
        // `hold` and bind it to the purpose in a guard that HAS a world, or have the
        // command carry the **purpose** rather than the id. `WoDeliver`'s own single
        // `order` parameter WAS enough to invoke it, which is why the delivery arm
        // grew `money::UNFUNDED_ORDERS` in the same fold.
        //
        // The refusal is `WO_REFUSED`, byte-identical to an unknown order.
        let hold = u64_field(parameters, "hold", u64::MAX).unwrap_or(0);
        if hold == 0 {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        let holder = wo_holder(id).map_err(|source| refuse(&source))?;
        let mut reservations = Vec::new();
        // money Hold → materials → capacity → destination (the fixed order). The
        // Hold is the submit command's E4b1 economy post; its id is recorded on
        // the order below.
        if !order.pinned().inputs().is_empty() {
            let source = order.material_source().ok_or_else(|| {
                error(
                    ActionErrorCode::ProviderRejected,
                    "a recipe with inputs needs a material source",
                )
            })?;
            for line in order.pinned().inputs() {
                let amount = line
                    .units()
                    .checked_mul(u64::from(order.quantity()))
                    .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?;
                let rid = self
                    .families
                    .inventory
                    .reserve(txn, source, line.commodity(), &holder, amount, now)
                    .map_err(|error| refuse(&IndustryError::Inventory(error)))?;
                reservations.push(rid);
            }
        }
        let capacity = self
            .families
            .reserve_slot(txn, order.facility(), order.facility_slots(), &holder, now)
            .map_err(|source| refuse(&source))?;
        reservations.push(capacity);

        let total_units = order
            .pinned()
            .work_units()
            .checked_mul(u64::from(order.quantity()))
            .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?;
        let production = self.open_schedule(txn, &order, now, actor, total_units)?;
        // The placed hold, denormalized onto the record. It is an INDEX, never the
        // authority: `money::live_hold` finds the reservation by purpose, so an
        // `AlreadyPosted` retry and a lost document both converge.
        let mut holds = order.holds().to_vec();
        let hold = mush_economy::HoldId(hold);
        if !holds.contains(&hold) {
            holds.push(hold);
        }
        let next = order
            .acquired(
                holds,
                reservations,
                Some(production),
                WorkOrderState::Running,
            )
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Submitted {id}; production started."))
    }

    /// Open the production schedule in the same bundle as the transition to
    /// `Running` (B5): a `DurableSchedule` on `ClockRef::WorldIc`, a job sidecar
    /// keyed by `ScheduleId`, and an occupancy check before reuse (handoff's
    /// pattern).
    fn open_schedule(
        &self,
        txn: &mut ActionTxn<'_>,
        order: &WorkOrder,
        now: &ClockStamp,
        actor: ObjId,
        total_units: u64,
    ) -> Result<ProductionSchedule, ActionError> {
        let schedule_id = production_schedule_id(order.id())?;
        // Occupancy: a prior schedule at this id may be reused only if it is
        // Completed and no live job stands (handoff.rs's rule).
        let prior = txn
            .get(&self.families.schedules, &schedule_id)
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?
            .cloned();
        if prior
            .as_ref()
            .is_some_and(|prior| prior.state() != mush_core::ScheduleState::Completed)
            || txn
                .get(&self.families.jobs, &schedule_id)
                .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?
                .is_some()
        {
            return Err(error(
                ActionErrorCode::ProviderRejected,
                "that facility slot's production schedule is still occupied",
            ));
        }
        let next_due = ClockStamp {
            clock: now.clock.clone(),
            at: now
                .at
                .checked_add(PRODUCTION_CADENCE_SECS)
                .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?,
        };
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: schedule_id,
            owner: crate::record::owner(),
            clock: now.clock.clone(),
            next_due,
            cadence: Some(PRODUCTION_CADENCE_SECS),
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: PRODUCTION_MAX_CATCH_UP,
        })
        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        if prior.is_some() {
            txn.replace(&self.families.schedules, &schedule_id, schedule)
        } else {
            txn.insert(&self.families.schedules, &schedule_id, schedule)
        }
        .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        let job = ProductionJob::new(
            schedule_id,
            order.id(),
            actor,
            PRODUCTION_UNITS_PER_OCCURRENCE,
            1,
        )
        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        txn.insert(&self.families.jobs, &schedule_id, job)
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        let policy = CanonicalId::new("production")
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        ProductionSchedule::new(schedule_id, now.clone(), policy, total_units, 0)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))
    }

    /// B6 — one scheduled production occurrence's progress. Driven only by an
    /// occurrence (the schedule adapter), never by observation. When production
    /// completes, the order moves to `AwaitingDelivery`.
    fn wo_advance(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = work_order_field(parameters, "order")?;
        let units = u64_field(parameters, "units", crate::workorder::MAX_RECIPE_WORK_UNITS)?;
        let order = self
            .families
            .work_order(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, WO_REFUSED))?;
        if *order.state() != WorkOrderState::Running {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        let mut next = order
            .with_progress(units)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        if next.schedule().is_some_and(ProductionSchedule::is_complete) {
            next = next
                .with_state(WorkOrderState::AwaitingDelivery)
                .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
            // R1: production is finished — the order leaves `Running`, so no more
            // occurrences are wanted. Tear the schedule and job down in this same
            // bundle, or the next cadence tick refuses (state != Running) and
            // freezes the world.
            self.families
                .tear_down_production(txn, production_schedule_id(id)?)
                .map_err(|source| refuse(&source))?;
        }
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Advanced {id}."))
    }

    /// B7 — the terminal transition: consume every material reservation (cut 2),
    /// release the capacity slot (B1), materialize the output lot at the
    /// destination (cut 2's `mint_origin`), write the completion marker the fact
    /// answers from, and transition to `Completed { commit }` — **one plan, one
    /// batch, no fallible step after the first install**. A retry from
    /// `AwaitingDelivery` re-runs neither production nor consumption (the marker
    /// and the derived lot are the replay guards).
    fn wo_deliver(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = work_order_field(parameters, "order")?;
        // ---- E7r1 fold: D4 — the money attestation on the DELIVERY ingress ----
        //
        // `wo_deliver` is a prepare: no `&World`, no `Ctx`, no `EconomyStore`, so it
        // cannot read a hold. Before the fold its whole money story was that nobody
        // had thought about it — `IndustryVerb::WoDeliver`'s offer projection declares
        // exactly `order`, which is all this function needs, so an offer-route
        // invocation consumed every material reservation and materialised the output
        // lot with no money check at all: production for free on precisely the ingress
        // R1 guarded on the submit side.
        //
        // A *supplied* parameter cannot close it: a command's submission and a direct
        // offer submission take the same route through `resolve_legal_action`, so an
        // attestation the command sets is one a hand-built submission sets too. The
        // gate is therefore `project_offer`'s **fixed** parameter
        // `money::UNFUNDED_ORDERS`, authored server-side where a world exists and
        // protected by `merge_parameters`, which refuses a submitted key colliding
        // with a fixed one. The refusal is `WO_REFUSED`, byte-identical to an unknown
        // order, so it discloses nothing a stranger did not already type.
        if parameters
            .get(crate::money::UNFUNDED_ORDERS)
            .and_then(Value::as_array)
            .is_some_and(|unfunded| {
                unfunded
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|named| named == id.0.to_string())
            })
        {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        let order = self
            .families
            .work_order(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, WO_REFUSED))?;
        self.require_requester(&order, actor)?;
        // Deliverable only once production has finished, and only from a live
        // resource-holding state; a second deliver is a marker HIT below.
        match order.state() {
            WorkOrderState::Running | WorkOrderState::AwaitingDelivery => {}
            _ => return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED)),
        }
        // ---- E7r1: R5(b) — `InterruptionCause::TimelineChanged`'s ONE writer ----
        //
        // 10b §15.1 item 8: the variant was declared at cut 4 and **set nowhere**, so
        // the D-E7-8 default it names — *a new rate generation with no registered
        // restamp pauses the order* — was a promise with no producer. This is it.
        //
        // It lives here rather than on the production schedule's own advance for a
        // reason the tree already decided: cut 8's `schedule_scan_clock_is_skippable`
        // makes the kernel **skip** a stale-generation schedule, so `wo_advance`
        // never fires to notice. The next thing that touches the order does, and
        // `@wo/restamp` — which already exists, and already resumes an `Interrupted`
        // order to `Running` — is the exit.
        if stale_generation(&order, now) {
            let paused = order
                .with_state(WorkOrderState::Interrupted {
                    cause: InterruptionCause::TimelineChanged,
                })
                .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
            self.families
                .put_work_order(txn, order.revision(), paused)
                .map_err(|source| refuse(&source))?;
            data.insert("order".to_owned(), json!(id.to_string()));
            return Ok(format!(
                "{id} was started on an earlier world-time generation and is paused; \
                 `@wo/restamp {id}` re-anchors it and resumes production."
            ));
        }
        if !order
            .schedule()
            .is_some_and(ProductionSchedule::is_complete)
        {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        let holder = wo_holder(id).map_err(|source| refuse(&source))?;
        let commit = CommitToken::from_bytes(*invocation.request().as_bytes());

        // The completion marker is the epoch-free replay guard: it REFUSES an
        // existing marker rather than rewriting it, so a retry cannot mint twice.
        let marker_key = marker_id(invocation.request(), now.at);
        if txn
            .get(&self.families.wo_applied, &marker_key)
            .map_err(|error| refuse(&IndustryError::from(error)))?
            .is_some()
        {
            return Err(error(ActionErrorCode::ProviderRejected, WO_ALREADY));
        }

        // Consume every material reservation.
        if !order.pinned().inputs().is_empty() {
            let source = order.material_source().ok_or_else(|| {
                error(
                    ActionErrorCode::Integrity,
                    "a running order lost its source",
                )
            })?;
            for line in order.pinned().inputs() {
                let resource = mush_inventory::pool_resource(source, line.commodity())
                    .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
                let rid = ReservationId::derive(&holder, &resource)
                    .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
                let record = self
                    .families
                    .inventory
                    .reservation(txn, source, line.commodity(), rid)
                    .map_err(|error| refuse(&IndustryError::Inventory(error)))?
                    .ok_or_else(|| error(ActionErrorCode::Integrity, WO_REFUSED))?;
                self.families
                    .inventory
                    .consume(
                        txn,
                        source,
                        line.commodity(),
                        rid,
                        record.revision(),
                        now,
                        commit,
                    )
                    .map_err(|error| refuse(&IndustryError::Inventory(error)))?;
            }
        }
        // Release the capacity slot (it returns to the facility).
        let cap_resource = crate::facility::facility_resource(order.facility())
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        let cap_id = ReservationId::derive(&holder, &cap_resource)
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        if let Some(record) = self
            .families
            .slot_reservation(txn, order.facility(), cap_id)
            .map_err(|source| refuse(&source))?
            && record.state() == mush_core::ReservationState::Held
        {
            self.families
                .release_slot(txn, order.facility(), cap_id, record.revision())
                .map_err(|source| refuse(&source))?;
        }
        // Materialize the output at the destination. The commodity output is a
        // cut-2 lot; a unit-representation output's title/maintenance is cut 5's
        // (issue_title needs a world the provider does not have).
        let dest = order.destination().endpoint();
        let grade = Grade::new(0).map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        for (ordinal, line) in order.pinned().outputs().iter().enumerate() {
            let amount = line
                .units()
                .checked_mul(u64::from(order.quantity()))
                .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?;
            let lot = mush_inventory::derive_lot_id(
                invocation.request(),
                u32::try_from(ordinal)
                    .map_err(|_| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?,
            )
            .map_err(|source| refuse(&IndustryError::Inventory(source)))?;
            self.families
                .inventory
                .mint_origin(
                    txn,
                    dest,
                    line.commodity(),
                    amount,
                    grade,
                    self.wo_source(id),
                    lot,
                )
                .map_err(|error| refuse(&IndustryError::Inventory(error)))?;
        }
        let marker = WorkOrderMarker::new(FACT_WORKORDER_COMPLETED, id, actor, now.at)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        // R3 (independent verify fold): the completion marker rides a per-facility
        // FIFO so `industry.wo_applied` stays bounded and the fact mirror never
        // truncates (an unbounded family blacked out EVERY industry fact past the
        // cap). The refuse-an-existing-marker guard above and the request-derived
        // lot id remain the durable replay guards.
        self.families
            .record_wo_completion(txn, order.facility(), marker_key, marker)
            .map_err(|source| refuse(&source))?;
        // R1: the schedule was already torn down when production completed
        // (`wo_advance` → AwaitingDelivery); this is idempotent belt-and-suspenders
        // for a deliver that raced straight from `Running`, so nothing survives to
        // tick against a terminal order.
        self.families
            .tear_down_production(txn, production_schedule_id(id)?)
            .map_err(|source| refuse(&source))?;
        let next = order
            .with_state(WorkOrderState::Completed { commit })
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Delivered {id}."))
    }

    /// B7 — cancel: release every reservation exactly once and mark `Cancelled`.
    /// A second cancel is a HIT (the order is already terminal).
    fn wo_cancel(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = work_order_field(parameters, "order")?;
        let order = self
            .families
            .work_order(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, WO_REFUSED))?;
        self.require_requester(&order, actor)?;
        if order.state().is_terminal() {
            // A second cancel resurrects nothing and releases nothing again.
            data.insert("order".to_owned(), json!(id.to_string()));
            return Ok(format!("{id} was already terminal."));
        }
        let holder = wo_holder(id).map_err(|source| refuse(&source))?;
        if order.state().holds_resources() {
            if !order.pinned().inputs().is_empty()
                && let Some(source) = order.material_source()
            {
                for line in order.pinned().inputs() {
                    let resource = mush_inventory::pool_resource(source, line.commodity())
                        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
                    let rid = ReservationId::derive(&holder, &resource)
                        .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
                    if let Some(record) = self
                        .families
                        .inventory
                        .reservation(txn, source, line.commodity(), rid)
                        .map_err(|error| refuse(&IndustryError::Inventory(error)))?
                        && record.state() == mush_core::ReservationState::Held
                    {
                        self.families
                            .inventory
                            .release(txn, source, line.commodity(), rid, record.revision())
                            .map_err(|error| refuse(&IndustryError::Inventory(error)))?;
                    }
                }
            }
            let cap_resource = crate::facility::facility_resource(order.facility())
                .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
            let cap_id = ReservationId::derive(&holder, &cap_resource)
                .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
            if let Some(record) = self
                .families
                .slot_reservation(txn, order.facility(), cap_id)
                .map_err(|source| refuse(&source))?
                && record.state() == mush_core::ReservationState::Held
            {
                self.families
                    .release_slot(txn, order.facility(), cap_id, record.revision())
                    .map_err(|source| refuse(&source))?;
            }
        }
        // R1: cancelling from any non-terminal state ends the production schedule
        // and its job in the same bundle, so no post-cancel tick raises
        // `ProviderRejected`.
        self.families
            .tear_down_production(txn, production_schedule_id(id)?)
            .map_err(|source| refuse(&source))?;
        let next = order
            .with_state(WorkOrderState::Cancelled)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Cancelled {id}."))
    }

    /// B6 / R2 (independent verify fold) — restamp reconciles a paused production
    /// schedule to the **current** `WorldIc` generation, the default policy a
    /// generation change needs to recover. A generation bump leaves the schedule
    /// pinned to a now-stale generation; the shared scan SKIPS it (it no longer
    /// freezes the world), so it is paused until this rebuilds it at `now`'s clock
    /// and resumes an interrupted order. Requester-gated, like every other
    /// work-order transition.
    fn wo_restamp(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &serde_json::Map<String, Value>,
        now: &ClockStamp,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let id = work_order_field(parameters, "order")?;
        let order = self
            .families
            .work_order(txn, id)
            .map_err(|source| refuse(&source))?
            .ok_or_else(|| error(ActionErrorCode::ProviderRejected, WO_REFUSED))?;
        self.require_requester(&order, actor)?;
        // Meaningful only for a live order still bound to a schedule.
        if order.schedule().is_none() {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        match order.state() {
            WorkOrderState::Running | WorkOrderState::Interrupted { .. } => {}
            _ => return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED)),
        }
        // ---- E7r1 fold Z3: the MATERIAL reservations are re-anchored too ----
        //
        // Also found by the fold's own grounding while driving C2 end to end, and it
        // is what kept `@wo/restamp` from being an exit even after the schedule stamp
        // was rewritten: `mush_inventory::reserve` stamps
        // `reservation_deadline(now)` with `now`'s generation, and
        // `ReservationBook::checked_consume` runs `require_future(expires, now)`,
        // whose `ClockStamp::checked_cmp` refuses two **incomparable** generations
        // outright. So an order whose recipe has inputs met
        // `"reservation deadline clock mismatch: WorldIc { generation: 1 } versus
        // WorldIc { generation: 2 }"` on the very next delivery — a rate change
        // bricked it one layer below the pause.
        //
        // The E-side adapter is release-then-reserve inside this same bundle, using
        // the seams that already exist: `release` takes no clock, and `reserve`
        // re-derives the SAME `ReservationId` from the order's holder and resource,
        // so `order.reservations()` needs no edit and the amount stays the recipe's
        // own. Contention is re-run by `checked_hold` against the freed stock, so a
        // restamp that can no longer be funded in materials refuses honestly rather
        // than resuming an order that cannot deliver. The capacity slot needs no
        // re-anchor: `release_slot` takes no clock at all.
        let holder = wo_holder(id).map_err(|source| refuse(&source))?;
        if !order.pinned().inputs().is_empty()
            && let Some(source) = order.material_source()
        {
            for line in order.pinned().inputs() {
                let resource = mush_inventory::pool_resource(source, line.commodity())
                    .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
                let rid = ReservationId::derive(&holder, &resource)
                    .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
                let Some(record) = self
                    .families
                    .inventory
                    .reservation(txn, source, line.commodity(), rid)
                    .map_err(|error| refuse(&IndustryError::Inventory(error)))?
                else {
                    continue;
                };
                if record.state() != mush_core::ReservationState::Held {
                    continue;
                }
                self.families
                    .inventory
                    .renew(txn, source, line.commodity(), rid, record.revision(), now)
                    .map_err(|error| refuse(&IndustryError::Inventory(error)))?;
            }
        }
        let schedule_id = production_schedule_id(id)?;
        let prior = txn
            .get(&self.families.schedules, &schedule_id)
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?
            .cloned();
        // Nothing to reconcile if a live schedule already runs on the current
        // generation and the order is not interrupted.
        if let Some(prior) = prior.as_ref()
            && prior.clock() == &now.clock
            && *order.state() == WorkOrderState::Running
        {
            return Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED));
        }
        let next_due = ClockStamp {
            clock: now.clock.clone(),
            at: now
                .at
                .checked_add(PRODUCTION_CADENCE_SECS)
                .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, WO_REFUSED))?,
        };
        let generation = prior
            .as_ref()
            .map_or(1, |p| p.generation().saturating_add(1));
        let next_ordinal = prior.as_ref().map_or(0, DurableSchedule::next_ordinal);
        let restamped = DurableSchedule::new(DurableScheduleDraft {
            id: schedule_id,
            owner: crate::record::owner(),
            clock: now.clock.clone(),
            next_due,
            cadence: Some(PRODUCTION_CADENCE_SECS),
            generation,
            next_ordinal,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: PRODUCTION_MAX_CATCH_UP,
        })
        .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        if prior.is_some() {
            txn.replace(&self.families.schedules, &schedule_id, restamped)
        } else {
            txn.insert(&self.families.schedules, &schedule_id, restamped)
        }
        .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        // ---- E7r1 fold: C2 — the restamp REWRITES the stamp the pause reads ----
        //
        // `stale_generation` compares `order.schedule().started().clock` — the stamp
        // on the **`WorkOrder` record's own `ProductionSchedule`** — against `now`.
        // Its single writer in the tree is `open_schedule`'s
        // `ProductionSchedule::new(.., now, ..)`, called only from `wo_submit` via
        // `order.acquired(..)`. Before the fold this arm rebuilt the kernel's
        // `DurableSchedule` and then only `with_state(Running)`, and `with_state`
        // clones the record — schedule included — so the predicate was still true on
        // the very next `@wo/deliver` and the order re-paused: restamp, deliver,
        // restamp, deliver, for ever, with its budget hold, its material reservations
        // and its facility slot held the whole time and nothing paging, because
        // `Interrupted` holds resources and is not terminal. One `@tacsim/time rate=`
        // bricked every in-flight order, and §15.1 row 8's declared exit did not
        // exist.
        //
        // The predicate is evaluated **per attempt** over an input no other verb
        // rewrites, so it is a permanent filter unless its exit rewrites that input.
        // This is that rewrite: a fresh `ProductionSchedule` at `now`'s clock,
        // carrying the schedule's own `progress_units()` (the fifth argument, which
        // is exactly what preserves production progress), written back through
        // `acquired` under the checked revision in the same bundle as the
        // `DurableSchedule` replace. No field is added and no wire changes.
        let reanchored = match order.schedule() {
            Some(schedule) => Some(
                ProductionSchedule::new(
                    schedule.schedule(),
                    now.clone(),
                    schedule.clock_policy().clone(),
                    schedule.total_units(),
                    schedule.progress_units(),
                )
                .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?,
            ),
            None => None,
        };
        let next = order
            .acquired(
                order.holds().to_vec(),
                order.reservations().to_vec(),
                reanchored,
                WorkOrderState::Running,
            )
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Restamped {id} to the current generation."))
    }

    fn require_requester(&self, order: &WorkOrder, actor: ObjId) -> Result<(), ActionError> {
        if order.requester() == &PrincipalId::Object(actor) {
            Ok(())
        } else {
            Err(error(ActionErrorCode::ProviderRejected, WO_REFUSED))
        }
    }

    /// The `ExternalRef` a delivered lot's `Lineage::Origin` names.
    fn wo_source(&self, order: WorkOrderId) -> ExternalRef {
        ExternalRef {
            provider: SmolStr::new_static(NS),
            kind: SmolStr::new_static("workorder.output"),
            id: SmolStr::new(format!("wo/{:016x}", order.0)),
            revision: Some(1),
        }
    }
}

/// **Whether this order's production was started on a world-time generation that is
/// no longer live** (E7r1 R5(b)).
///
/// A pure comparison of two recorded stamps: the schedule's own `started` clock and
/// the `now` the prepare was handed. No world is read, nothing is drawn, so two
/// evaluations of one order agree — which is what lets the pause be idempotent.
/// An order with no schedule is not stale; it is not producing.
#[must_use]
fn stale_generation(order: &WorkOrder, now: &ClockStamp) -> bool {
    order
        .schedule()
        .is_some_and(|schedule| schedule.started().clock != now.clock)
}

fn obj_field(parameters: &serde_json::Map<String, Value>, key: &str) -> Result<ObjId, ActionError> {
    parameters
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .map(ObjId)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("an industry action needs an object for {key}"),
            )
        })
}

fn accessibility_field(
    parameters: &serde_json::Map<String, Value>,
) -> Result<Accessibility, ActionError> {
    parameters
        .get("accessibility")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .and_then(|value| Accessibility::new(value).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                format!("accessibility is a percentage 0..={MAX_ACCESSIBILITY}"),
            )
        })
}

fn revision_field(parameters: &serde_json::Map<String, Value>) -> Result<u64, ActionError> {
    parameters
        .get("revision")
        .and_then(Value::as_u64)
        .filter(|value| *value >= 1)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "an industry action names the revision it read",
            )
        })
}

/// A claim's term, carried on the wire as the **absolute IC second** it expires
/// at and rebuilt here into the [`ClockStamp`] the record keeps.
///
/// ---- WS02-A2: `@claim/file` never resolved on a real server ----
///
/// [`required_endpoint`]'s defect, one arm over. `IndustryVerb::ClaimFile`'s
/// projection declares `integer("term", …)` and `@claim/file` staged a
/// `ClockStamp` **object**, so `ProjectedParameterSchema::validate` answered
/// `InvalidParameters` on every submission that actually went through
/// `resolve_legal_action` — no player could file tenure at all. Nothing caught
/// it because the shipped tests file a `ClaimRecord` by hand or build the
/// invocation themselves and never resolve it.
///
/// So the wire is an integer end to end, and the generation comes from the
/// offer's own fixed `now` rather than from the client: a submitted stamp could
/// have named any generation it liked, and this one cannot name a generation the
/// world is not living in. The old object shape stays refused — `Value::as_i64`
/// is None for it — which is what keeps the declaration, the staging and the
/// read one shape rather than two that happen to agree.
fn term_field(
    parameters: &serde_json::Map<String, Value>,
    now: &ClockStamp,
) -> Result<ClockStamp, ActionError> {
    let at = parameters
        .get("term")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "a claim names the IC second its term expires at",
            )
        })?;
    Ok(ClockStamp {
        clock: now.clock.clone(),
        at,
    })
}

fn subject_field(parameters: &serde_json::Map<String, Value>) -> Result<ClaimSubject, ActionError> {
    let raw = text_field(parameters, "subject")?;
    if let Some(rest) = raw.strip_prefix("place:") {
        return rest
            .trim()
            .parse::<u32>()
            .ok()
            .map(|id| ClaimSubject::Place { place: ObjId(id) })
            .ok_or_else(|| error(ActionErrorCode::InvalidRequest, REFUSED));
    }
    OreDepositId::parse(raw.strip_prefix("deposit:").unwrap_or(&raw))
        .map(|deposit| ClaimSubject::Deposit { deposit })
        .ok_or_else(|| error(ActionErrorCode::InvalidRequest, REFUSED))
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(|error| ModError::msg(error.to_string()))
}

fn exact_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                (
                    descriptor.payload().clone(),
                    references,
                    descriptor.is_current(),
                ),
            )
        })
        .collect();
    move |context| {
        let definition = context.definition();
        let key = (
            definition.reference().key().id.clone(),
            definition.reference().version(),
        );
        let Some((payload, references, current)) = expected.get(&key) else {
            return Err("unknown industry definition".to_owned());
        };
        if definition.payload() != payload
            || definition.references() != references
            || definition.is_current() != *current
        {
            return Err("industry definition shape mismatch".to_owned());
        }
        Ok(())
    }
}

/// Register the outcome, the eight actions and their providers.
///
/// # Errors
/// [`ModError`] if any definition or provider registration is refused.
pub fn register(
    reg: &mut Registrar<'_>,
    families: &IndustryFamilies,
    recipes: Option<std::sync::Arc<dyn crate::workorder::RecipeAuthority>>,
) -> Result<(), ModError> {
    register_with_core_origins(reg, families, recipes, None, None)
}

pub(crate) fn register_with_core_origins(
    reg: &mut Registrar<'_>,
    families: &IndustryFamilies,
    recipes: Option<std::sync::Arc<dyn crate::workorder::RecipeAuthority>>,
    origins: Option<&crate::core_production::CoreOriginConfig>,
    factory: Option<&crate::core_production::integration::CoreFactoryConfig>,
) -> Result<(), ModError> {
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({ "fields": ["actor", "verb", "summary"] }),
    )?;
    let mut outcomes = vec![outcome_descriptor.clone()];
    if let Some(config) = origins {
        outcomes.push(config.outcome_descriptor()?);
    }
    if factory.is_some() {
        outcomes.push(crate::core_production::integration::runtime::outcome_descriptor()?);
    }
    if factory.and_then(|c| c.refit.as_ref()).is_some() {
        outcomes.push(crate::core_production::refit::runtime::outcome_descriptor()?);
    }
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&outcomes),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;

    let verbs = IndustryVerb::all();
    let descriptors = verbs
        .iter()
        .map(|verb| {
            descriptor(
                "action",
                verb.id(),
                json!({ "subject": "self_actor", "targets": 0, "family": verb.family() }),
            )
            .map(|descriptor| descriptor.with_references(vec![outcome.clone()]))
        })
        .collect::<Result<Vec<_>, ModError>>()?;
    let mut action_shapes = descriptors.clone();
    if let Some(config) = origins {
        action_shapes.extend([config.policy.action(true)?, config.policy.action(false)?]);
    }
    if let Some(config) = factory {
        if config.payment().delivery.is_some() {
            action_shapes.push(
                config
                    .payment()
                    .action_descriptor(crate::core_production::integration::delivery::ACTION)
                    .map_err(|e| ModError::msg(e.to_string()))?,
            );
        }
        if let Some(refit) = &config.refit {
            for name in crate::core_production::refit::ACTIONS {
                action_shapes.push(
                    refit
                        .action_descriptor(name)
                        .map_err(|e| ModError::msg(e.to_string()))?,
                );
            }
        }
        for name in crate::core_production::integration::ACTIONS {
            action_shapes.push(
                config
                    .payment()
                    .action_descriptor(name)
                    .map_err(|e| ModError::msg(e.to_string()))?,
            );
        }
    }
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&action_shapes),
    )?;
    let mut advance_action: Option<DefinitionRef> = None;
    for (verb, descriptor) in verbs.into_iter().zip(descriptors) {
        let action = reg.definition(descriptor)?;
        if verb == IndustryVerb::WoAdvance {
            advance_action = Some(action.clone());
        }
        reg.action_provider(
            action.clone(),
            IndustryProvider {
                verb,
                action,
                outcome: outcome.clone(),
                families: families.clone(),
                recipes: recipes.clone(),
                catalog: std::sync::Mutex::new(None),
                truncation_warned: std::sync::Mutex::new(None),
            },
        )?;
    }

    // ---- E7c4: B5 — the production schedule source, on `handoff.rs`'s shape ----
    // D-E7-16's "cut 8 gets the first production schedule source" is corrected:
    // the tree already has one (`mush-sim/src/handoff.rs`), which this copies as
    // the pattern. One source per family, under `mush-industry`'s own `OwnerId`.
    let advance_action = advance_action
        .ok_or_else(|| ModError::msg("industry: the production advance action is unregistered"))?;
    reg.schedule_source(
        CanonicalId::new("production").map_err(|error| ModError::msg(error.to_string()))?,
        advance_action,
        families.schedules.clone(),
        ProductionScheduleAdapter {
            schedules: families.schedules.clone(),
            jobs: families.jobs.clone(),
            work_orders: families.work_orders.clone(),
            recipes,
        },
    )?;
    Ok(())
}

/// The production schedule's adapter (the `HandoffScheduleAdapter` shape): `plan`
/// re-derives the candidate from stored state and **refuses a stale one** rather
/// than acting on it, then builds the `WoAdvance` draft on the requester's actor
/// with the definition guard the kernel requires.
#[derive(Clone)]
struct ProductionScheduleAdapter {
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    jobs: StateFamilyHandle<ScheduleId, ProductionJob>,
    work_orders: StateFamilyHandle<WorkOrderId, WorkOrder>,
    recipes: Option<std::sync::Arc<dyn crate::workorder::RecipeAuthority>>,
}

impl ScheduleActionAdapter for ProductionScheduleAdapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let current = engine
            .action_state()
            .get(&self.schedules, &advance.schedule())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "production schedule is unavailable",
                )
            })?;
        // Re-derive: the candidate is current only if re-planning the live cursor
        // yields the identical advance (handoff's `candidate_is_current`).
        let candidate_is_current = current.is_some_and(|schedule| {
            matches!(
                schedule.plan_due(schedule.revision(), advance.observed_at(), advance.cause()),
                Ok(ScheduleEvaluation::Advance(planned)) if planned.as_ref() == advance
            )
        });
        if !candidate_is_current {
            return Err(error(
                ActionErrorCode::Stale,
                "production schedule candidate is no longer current",
            ));
        }
        let job = engine
            .action_state()
            .get(&self.jobs, &advance.schedule())
            .map_err(|_| error(ActionErrorCode::Integrity, "production job is unavailable"))?
            .ok_or_else(|| error(ActionErrorCode::Integrity, "production job is missing"))?;
        // R1 (belt-and-suspenders for a historical orphan the teardown never
        // reached) + C1 (the recipe authority is world-available here, at `plan`,
        // where `prepare` is not): the order this schedule drives must be live and
        // `Running`, and its pinned recipe digest must still match the authority's
        // published one. A terminal/missing order or a drifted recipe is `Stale`,
        // which the driver DISCARDS rather than freezing on — so a re-pinned recipe
        // never mints and a stale schedule never raises `ProviderRejected`.
        let order = engine
            .action_state()
            .get(&self.work_orders, &job.order())
            .map_err(|_| error(ActionErrorCode::Integrity, "work order is unavailable"))?;
        let Some(order) = order else {
            return Err(error(
                ActionErrorCode::Stale,
                "production schedule outlived its work order",
            ));
        };
        if *order.state() != WorkOrderState::Running {
            return Err(error(
                ActionErrorCode::Stale,
                "production schedule's work order is no longer running",
            ));
        }
        if let Some(recipes) = self.recipes.as_ref()
            && recipes.digest_of(&engine.world, order.recipe()) != Some(order.recipe().digest())
        {
            return Err(error(
                ActionErrorCode::Stale,
                "production recipe has drifted from the authority; the order will not advance",
            ));
        }
        let occurrences = u64::try_from(advance.occurrences().len())
            .map_err(|_| error(ActionErrorCode::BoundsExceeded, "too many occurrences"))?;
        let units = occurrences
            .checked_mul(job.units_per_occurrence())
            .ok_or_else(|| error(ActionErrorCode::BoundsExceeded, "occurrence overflow"))?;
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| error(ActionErrorCode::Integrity, "guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "definition guard is unavailable",
                )
            })?;
        Ok(ScheduleActionDraft {
            // The invocation's actor is the requester (from the job), while its
            // principal is the scheduler — the `HandoffScheduleAdapter` shape.
            actor: job.actor(),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "order": job.order().to_string(),
                "units": units,
                "now": advance.observed_at(),
            }),
            guards: guards
                .finish()
                .map_err(|_| error(ActionErrorCode::Integrity, "production guards are invalid"))?,
        })
    }
}

/// The registered `DefinitionRef` for one verb, rebuilt from the frozen
/// registry so a command handler can name it without holding a handle.
#[must_use]
pub fn action_ref(registry: &mush_core::Registry, verb: IndustryVerb) -> Option<DefinitionRef> {
    let definitions = registry.definitions()?;
    let id = DefinitionId::new(verb.id()).ok()?;
    let kind = DefinitionKind::new("action").ok()?;
    definitions
        .current(&mush_core::DefinitionKey {
            owner: crate::record::owner(),
            kind,
            id,
        })
        .cloned()
}
