//! **The Isidis operation: the declaring arm and the resolution** (doc 50 cut OP.5; Q-50-18,
//! Q-50-19, Q-50-28, Q-50-29, Q-50-57).
//!
//! - **The declaring arm** (Q-50-19): a compiled arm on the Assembly's tick acts for the
//!   Republic. When the Concession Review fills, it declares one operation through OP.0's frame
//!   (`crate::operations::frame`) and activates it in the same step:
//!   - declared by the Republic's customs office ([`DECLARER_ORG`]), the Republic's treasury in
//!     the lane, with a bond of [`BOND_CR`] held on its account and forfeit to Occator's on its
//!     failure (Q-50-57 item 5);
//!   - two sides, the Republic and the Freeport, and the Freeport's backer, Occator, as defender;
//!   - [`NOTICE_IC_DAYS`] of notice and a window of [`WINDOW_IC_DAYS`] (Q-50-19: the arm sets the
//!     28-day window itself);
//!   - the objective is the tank farm's close probe (`crate::stockpile::TANK_FARM_PROBE`), read
//!     at the window's close: at its threshold the defenders hold (Q-50-28).
//!   - No player or staff step. Unwired (no frame composed), it declares nothing and waits.
//! - **The resolution** (Q-50-29): once the frame records who won, the arm writes it once. The
//!   Republic's success makes it the Concession's de facto holder through the one control write
//!   (`mush_faction::write_control`, Q-50-51); the defenders holding leaves the Freeport there. A
//!   partial or abandoned operation changes nothing. The import duty follows the holder through
//!   the schedule (`crate::port::import_duty`).
//!
//! **The restart argument.** The record (`tacsim:isidis_op`) holds the Review clock the
//! operation answers, the operation's id and whether its outcome was written. The declaration
//! and the record are one fire's batch; the outcome is the frame's, read every fire, and the
//! control write is guarded by the record and by the holder already standing there.

use mush_core::{CanonicalId, Ctx, World};
use mush_faction::ControlKind;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::operations::{
    MetFavors, OperationBond, OperationOutcome, OperationPhase, OperationRequest, OperationStatus,
};

/// The record's key.
pub const OP_KEY: &str = "tacsim:isidis_op";
/// Who declares for the Republic: its customs office, the Republic's treasury in the lane.
pub const DECLARER_ORG: &str = "republic_customs";
/// The defenders' backer, who takes the bond when the Republic fails (Q-50-22, Q-50-57 item 5).
pub const DEFENDER_ORG: &str = "occator";
/// The Concession's place, for its region.
pub const ISIDIS_PLACE: &str = "isidis_port";
/// The bond, whole credits (provisional).
pub const BOND_CR: i64 = 5_000;
/// The notice, IC days (Q-50-19).
pub const NOTICE_IC_DAYS: i64 = 28;
/// The window, IC days (Q-50-19).
pub const WINDOW_IC_DAYS: i64 = 28;
/// The legal-target classes the operation names (Q-50-26 reads them; OP.6).
pub const LEGAL_TARGETS: &[&str] = &["carrier"];
/// The objective's one line.
pub const SUMMARY: &str = "The Republic moves to assume the administration of the Isidis \
     Concession. It succeeds unless Occator's tank farm at Isidis stands at its stockpile \
     threshold when the window closes.";

/// **The arm's record**, written whole when it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsidisOp {
    /// The Review clock the operation answers, `ClockId.0`: a Review that fills again on a new
    /// clock declares again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<u64>,
    /// The operation, as the frame numbers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<u64>,
    /// The IC second it was declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_ic: Option<i64>,
    /// Who won, once written: `republic`, `freeport` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Why the last declaration was refused, for staff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// **Finished operations**, oldest first (doc 50 cut OP.7: `+operation` lists them with
    /// their result), at most `crate::op_page::MAX_PAST`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub past: Vec<PastOp>,
}

/// **One finished operation** (doc 50 cut OP.7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PastOp {
    /// The operation, as the frame numbers it.
    pub operation: u64,
    /// The IC second it was declared.
    pub declared_ic: i64,
    /// Who won: `republic`, `freeport` or `none`.
    pub outcome: String,
}

/// The arm's record, as the store holds it.
#[must_use]
pub fn record(w: &World) -> Option<&IsidisOp> {
    w.module_state::<crate::TacsimStore>()?.isidis_op()
}

/// The operation now, through the frame, when one is declared and a frame is composed.
#[must_use]
pub fn status(w: &World) -> Option<OperationStatus> {
    let operation = record(w)?.operation?;
    crate::operations::frame(w)?.status(w, operation)
}

/// **The request the arm declares** (this module's head), or why it cannot be made.
///
/// # Errors
///
/// A lane organization or its account the lane does not have.
pub fn request(w: &World) -> Result<OperationRequest, String> {
    let id = |s: &str| CanonicalId::new(s).map_err(|e| e.to_string());
    let (declarer, account) = crate::polity::lane_org_account(w, &id(DECLARER_ORG)?)
        .ok_or_else(|| format!("{DECLARER_ORG} has no account"))?;
    let (defender, defender_account) = crate::polity::lane_org_account(w, &id(DEFENDER_ORG)?)
        .ok_or_else(|| format!("{DEFENDER_ORG} has no account"))?;
    let scale = 10_i64.pow(u32::from(crate::port::apply::credit_scale(w)));
    let declarer = PrincipalId::Organization(declarer);
    let defender = PrincipalId::Organization(defender);
    Ok(OperationRequest {
        declarer: declarer.clone(),
        summary: SUMMARY.to_owned(),
        probe: crate::stockpile::TANK_FARM_PROBE.to_owned(),
        met_favors: MetFavors::Defenders,
        sides: vec![
            ("republic".to_owned(), declarer),
            ("freeport".to_owned(), defender.clone()),
        ],
        defenders: vec![defender.clone()],
        legal_targets: LEGAL_TARGETS.iter().map(|s| (*s).to_owned()).collect(),
        bond: Some(OperationBond {
            account,
            amount: mush_economy::Amount::from_minor(BOND_CR.saturating_mul(scale)),
            forfeit_to: Some((defender, defender_account)),
        }),
        notice_secs: NOTICE_IC_DAYS.saturating_mul(crate::port::stock::IC_DAY),
        window_secs: WINDOW_IC_DAYS.saturating_mul(crate::port::stock::IC_DAY),
        public: true,
    })
}

/// **The arm's step** (on the Assembly's tick): declare on the Review's fill, then write the
/// outcome once.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let kept = record(ctx.w).cloned();
    let mut doc = kept.clone().unwrap_or_default();
    step(ctx, &mut doc);
    if kept.as_ref() != Some(&doc) {
        crate::store::put_isidis_op(ctx, &doc);
    }
}

fn step(ctx: &mut Ctx<'_>, doc: &mut IsidisOp) {
    let Some(review) = crate::concession::review_clock(ctx.w) else {
        return;
    };
    // 1. The Review filled on a clock this arm has not answered: declare.
    if review.completed && doc.clock != Some(review.id.0) {
        let Some(frame) = crate::operations::frame(ctx.w) else {
            return;
        };
        let request = match request(ctx.w) {
            Ok(request) => request,
            Err(why) => {
                doc.refused = Some(why);
                return;
            }
        };
        match frame.declare(ctx, &request) {
            Ok(operation) => {
                tracing::info!(
                    operation,
                    "tacsim: the Republic declares the Isidis operation"
                );
                // Doc 50 cut OP.7: a decided operation goes to the past before it is replaced.
                crate::op_page::keep_past(doc);
                *doc = IsidisOp {
                    clock: Some(review.id.0),
                    operation: Some(operation),
                    declared_ic: Some(ctx.w.world_ic_now().at),
                    outcome: None,
                    refused: None,
                    past: std::mem::take(&mut doc.past),
                };
                // Doc 50 cut OP.7 (Q-50-30): the lane hears of it at once.
                if let Some(status) = frame.status(ctx.w, operation) {
                    crate::op_page::tell_declared(ctx, &status);
                }
            }
            Err(why) => {
                tracing::warn!(%why, "tacsim: the Isidis operation could not be declared");
                doc.refused = Some(why);
            }
        }
        return;
    }
    // 2. Decided: write who won, once. Doc 50b item 4: only a settled outcome. While the
    //    operation is still settling, the campaign can still abort it, so an outcome shown then
    //    is not yet final.
    if doc.outcome.is_some() {
        return;
    }
    let Some(outcome) = status(ctx.w)
        .filter(|s| s.phase == OperationPhase::Settled)
        .and_then(|s| s.outcome)
    else {
        return;
    };
    let word = match outcome {
        OperationOutcome::DeclarerSucceeded => match republic_takes_isidis(ctx) {
            Ok(()) => "republic",
            Err(why) => {
                tracing::warn!(%why, "tacsim: the Isidis operation's outcome could not be written");
                return;
            }
        },
        OperationOutcome::DeclarerFailed => "freeport",
        OperationOutcome::Partial | OperationOutcome::Abandoned => "none",
    };
    doc.outcome = Some(word.to_owned());
    // Doc 50 cut OP.7 (Q-50-30): the lane hears who won at once.
    crate::op_page::tell_resolved(ctx, word);
}

/// **The Republic assumes the Concession's administration** (Q-50-29): its de facto control
/// record, through the one control write. Already there, nothing is written.
///
/// # Errors
///
/// The Concession's region, the Republic or the control write missing.
fn republic_takes_isidis(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let place = CanonicalId::new(ISIDIS_PLACE).map_err(|e| e.to_string())?;
    let control = crate::polity::control_at(ctx.w, &place)
        .ok_or_else(|| "the Concession has no region".to_owned())?;
    let republic = crate::polity::polity(ctx.w, "republic")
        .ok_or_else(|| "the Republic is not seeded".to_owned())?;
    if control.de_facto == Some(republic) {
        return Ok(());
    }
    mush_faction::write_control(
        ctx,
        mush_faction::ControlWrite::new(control.region, republic, ControlKind::DeFacto),
    )?;
    ctx.out.pin_deferred();
    Ok(())
}
