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
    AuthoritativeActionOutput, CanonicalId, ClockStamp, CommitToken, DefinitionDescriptor,
    DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    DurableSchedule, DurableScheduleDraft, Engine, LegalActionQuery, ModError, ObjId, PerceivedRef,
    PrincipalId as CorePrincipalId, Registrar, ReservationId, RevisionSet, RulesValue,
    ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance, ScheduleCatchUpOrder,
    ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy, StateFamilyHandle, SubjectKey,
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
    DeliveryTarget, PRODUCTION_CADENCE_SECS, PRODUCTION_MAX_CATCH_UP,
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
}

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded industry action error")
}

/// **One message, whatever the reason.** Depletion, a missing licence, a
/// suspended claim and a body the actor may not know exists all answer the same
/// sentence to a non-staff principal (08c §6.2's D1 shape, §4 rule 4): two
/// refusals a viewer may not distinguish are byte-identical.
const REFUSED: &str = "You cannot work that here.";

/// The one line every work-order refusal a viewer may not distinguish renders.
const WO_REFUSED: &str = "You cannot do that with that work order.";
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
                parameters.push(text("issuer", 64)?);
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
                // The draft command (world available) resolves the recipe from the
                // authority and pins the facility policy, then submits these as
                // parameters; the provider re-checks the shapes and writes the
                // Draft work order. `pinned` carries the resolved lines as a JSON
                // blob and `digest` the pack digest.
                parameters.push(integer("facility", 0, i64::from(u32::MAX))?);
                parameters.push(integer(
                    "facility_slots",
                    1,
                    i64::from(crate::facility::MAX_FACILITY_SLOTS),
                )?);
                parameters.push(integer("capacity_revision", 1, i64::MAX)?);
                parameters.push(integer("policy_revision", 1, i64::MAX)?);
                parameters.push(text("pack", 64)?);
                parameters.push(integer("version", 1, i64::from(u32::MAX))?);
                parameters.push(text("recipe", 64)?);
                parameters.push(text("digest", 64)?);
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
        Ok(Some(mush_core::ProviderOfferDraft {
            // The IC stamp rides the offer's FIXED parameters, so preparation
            // never reaches for a clock it does not have and two replays of one
            // request stamp the identical marker.
            fixed_parameters: json!({ "now": engine.world.world_ic_now() }),
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
            IndustryVerb::ClaimFile => self.file(txn, invocation, &parameters, &mut data)?,
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
            IndustryVerb::WoRestamp => self.wo_restamp(txn, &parameters, actor, &mut data)?,
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
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let subject = subject_field(parameters)?;
        let issuer = principal_field(parameters, "issuer")?;
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
        let term = term_field(parameters)?;
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

    /// B3 — write one Draft work order from the resolved, pinned inputs the draft
    /// command computed (world available there; the provider has no world).
    fn wo_draft(
        &self,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        parameters: &serde_json::Map<String, Value>,
        actor: ObjId,
        data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        let facility = obj_field(parameters, "facility")?;
        let facility_slots = u32::try_from(u64_field(
            parameters,
            "facility_slots",
            u64::from(crate::facility::MAX_FACILITY_SLOTS),
        )?)
        .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let capacity_revision = u64_field(parameters, "capacity_revision", u64::MAX)?;
        let policy_revision = u64_field(parameters, "policy_revision", u64::MAX)?;
        let pack = canonical_field(parameters, "pack", 64)?;
        let version = u32::try_from(u64_field(parameters, "version", u64::from(u32::MAX))?)
            .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let recipe_id = canonical_field(parameters, "recipe", 64)?;
        let digest = DefinitionDigest::from_hex(&text_field(parameters, "digest")?)
            .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let quantity = u32::try_from(u64_field(
            parameters,
            "quantity",
            u64::from(crate::workorder::MAX_WORK_ORDER_QUANTITY),
        )?)
        .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let account = AccountId(u64_field(parameters, "account", u64::MAX)?);
        let source = optional_endpoint(parameters, "source")?;
        let destination = DeliveryTarget::Endpoint {
            endpoint: endpoint_field(parameters, "destination")?,
        };
        let beneficiary = principal_field(parameters, "beneficiary")?;
        let pinned: PinnedRecipe = serde_json::from_str(&text_field(parameters, "pinned")?)
            .map_err(|_| error(ActionErrorCode::InvalidRequest, WO_REFUSED))?;
        let recipe = RecipeRef::new(pack, version, recipe_id, digest)
            .map_err(|source| error(ActionErrorCode::InvalidRequest, source.to_string()))?;
        let id = derive_work_order_id(invocation.request(), 0).map_err(|source| refuse(&source))?;
        let order = WorkOrder::draft(
            id,
            PrincipalId::Object(actor),
            beneficiary,
            facility,
            facility_slots,
            capacity_revision,
            policy_revision,
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
        let holder = wo_holder(id).map_err(|source| refuse(&source))?;
        let mut reservations = Vec::new();
        // money Hold → materials → capacity → destination (the fixed order). The
        // Hold is the submit command's E4b1 economy post; its id is recorded on
        // the order (kept empty here when no hold was supplied).
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
        let next = order
            .acquired(
                order.holds().to_vec(),
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
        let local = CanonicalId::new(format!("production.{:016x}", order.id().0))
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
        let schedule_id = ScheduleId::derive(&crate::record::owner(), &local)
            .map_err(|_| error(ActionErrorCode::Integrity, WO_REFUSED))?;
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
        txn.insert(&self.families.wo_applied, &marker_key, marker)
            .map_err(|error| refuse(&IndustryError::from(error)))?;
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
        let next = order
            .with_state(WorkOrderState::Cancelled)
            .map_err(|source| error(ActionErrorCode::Integrity, source.to_string()))?;
        self.families
            .put_work_order(txn, order.revision(), next)
            .map_err(|source| refuse(&source))?;
        data.insert("order".to_owned(), json!(id.to_string()));
        Ok(format!("Cancelled {id}."))
    }

    /// B6 — restamp is campaign authority and refuses without a registered
    /// policy. Cut 4 ships the pause default and the registration seam only (§9
    /// q8), so there is no policy yet: it always refuses, naming the seam.
    fn wo_restamp(
        &self,
        _txn: &mut ActionTxn<'_>,
        _parameters: &serde_json::Map<String, Value>,
        _actor: ObjId,
        _data: &mut BTreeMap<String, Value>,
    ) -> Result<String, ActionError> {
        Err(error(
            ActionErrorCode::ProviderRejected,
            "no restamp policy is registered; the campaign layer (cut 9) registers one",
        ))
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

fn term_field(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
    parameters
        .get("term")
        .and_then(|value| serde_json::from_value::<ClockStamp>(value.clone()).ok())
        .ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "a claim names the stamp its term expires at",
            )
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
pub fn register(reg: &mut Registrar<'_>, families: &IndustryFamilies) -> Result<(), ModError> {
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({ "fields": ["actor", "verb", "summary"] }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(std::slice::from_ref(&outcome_descriptor)),
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
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&descriptors),
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
