//! The bond (E7 cut 9 B5) — **funded, never infinite, and cut 9 owns no mint path**
//! (doc 10 §12 item 7A; T4; D-E7-2).
//!
//! The bond is an **E4 money `Hold` on the attacker's own account**, placed from the
//! **command** through `mush_economy::post`. It is placed from the command and not
//! from a provider because a `prepare_authoritative` provider has no `&World`,
//! `Ctx` or `Outbox` and therefore **cannot** place a hold (cut 4's As-built
//! deviation (a); cut 3's precedent, "the economy posts from the command"; 08i
//! answer 16's owner-exported `pub fn` seam). That is why `@op/activate` publishes
//! `LegacyPostImage` rather than `Authoritative`: it rides `mush_economy::post`'s
//! own carve-out lane, not the authoritative ingress.
//!
//! ## What this module does NOT do
//!
//! No `Amount` arithmetic. No `create_system_obligation`, no credit line, no
//! faucet, no minting `AssetOp`. A capture spends **the live hold's own amount**,
//! read from the snapshot, never a number this crate computed. The never-negative
//! gate is the engine's own: `validate` refuses a placement whose account's
//! available funds would go negative — `EconomyError::InsufficientAvailable`,
//! invariant 26 — and cut 9 writes no second check for it, because a second check
//! is a second opinion.
//!
//! ## Exactly once
//!
//! Every post carries a deterministic key: `campaign.op.bond/{op}` for the
//! placement and `campaign.resolve/{op}/{consumer}` for the settlement. A repeat is
//! `AlreadyPosted` — the same transaction id, no second effect — so a bond released
//! twice across a restart is unreachable rather than merely unlikely.

use mush_core::Ctx;
use mush_economy::{
    Amount, EconomicCause, EconomyError, EconomyPlan, EconomyStore, Hold, HoldId, HoldOp,
    HoldPurpose, IdempotencyKey, PostOutcome, RequestedLine,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::ids::OperationId;
use crate::record::{Bond, ConsumerId, OperationTerms};

/// The `HoldPurpose::Reservation` namespace every campaign bond is placed under.
pub const BOND_PURPOSE: &str = "campaign.operation";

/// The deterministic key one operation's bond placement posts under.
#[must_use]
pub fn bond_key(operation: OperationId) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("campaign.op.bond"),
        SmolStr::new(operation.0.to_string()),
    )
}

/// The deterministic key one consumer transition posts under, keyed off the root.
#[must_use]
pub fn consumer_key(operation: OperationId, consumer: ConsumerId) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("campaign.resolve"),
        SmolStr::new(format!("{}/{}", operation.0, consumer.as_str())),
    )
}

/// What a bond operation refused on.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BondError {
    /// The economy is not composed, or is frozen.
    #[error("the economy is not available")]
    NoEconomy,
    /// The engine refused the post. `InsufficientAvailable` is the never-negative
    /// gate firing and is reported as such.
    #[error("the economy refused: {0}")]
    Refused(String),
    /// The placement posted but no hold with this operation's purpose is live,
    /// which can only mean the post was an `AlreadyPosted` for a hold that has
    /// since settled. Never a silent success.
    #[error("no live hold carries this operation's purpose")]
    NoHold,
    /// A post is needed but the plan would need approval first.
    #[error("the account's approval rule fired")]
    NeedsApproval,
}

/// The purpose every campaign bond carries. One value, built in one place, so the
/// placement and the lookup can never disagree.
#[must_use]
pub fn purpose(operation: OperationId) -> HoldPurpose {
    HoldPurpose::Reservation {
        kind: SmolStr::new_static(BOND_PURPOSE),
        id: operation.0,
    }
}

/// The live hold backing this operation's bond, found **by purpose** rather than by
/// a stored id, so an `AlreadyPosted` retry and a lost document both converge.
#[must_use]
pub fn live_hold(ctx: &Ctx<'_>, operation: OperationId) -> Option<Hold> {
    let want = purpose(operation);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .holds()
        .find(|h| h.purpose == want && h.status.is_held())
        .cloned()
}

/// Any hold this operation ever placed, live or settled — the idempotency evidence
/// the settlement reads before deciding it still owes a release.
#[must_use]
pub fn any_hold(ctx: &Ctx<'_>, operation: OperationId) -> Option<Hold> {
    let want = purpose(operation);
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .holds()
        .find(|h| h.purpose == want)
        .cloned()
}

fn refuse(error: EconomyError) -> BondError {
    match error {
        EconomyError::EconomyFrozen => BondError::NoEconomy,
        other => BondError::Refused(other.to_string()),
    }
}

/// **Place the bond** (`@op/activate`). Posts one `HoldOp::Place` on the attacker's
/// own account under `campaign.op.bond/{op}` and returns the placed hold.
///
/// An attacker without funds cannot activate: `validate`'s invariant-26 check
/// refuses the placement with `InsufficientAvailable` **before** anything is
/// written, and the caller leaves the operation `Declared`.
pub fn place(ctx: &mut Ctx<'_>, terms: &OperationTerms, bond: &Bond) -> Result<HoldId, BondError> {
    if ctx.w.module_state::<EconomyStore>().is_none() {
        return Err(BondError::NoEconomy);
    }
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Place {
            account: bond.account,
            amount: bond.amount,
            beneficiary: bond.forfeit_to.as_ref().map(|(who, _)| who.clone()),
            purpose: purpose(terms.operation),
            expires: None,
        }],
        description: format!("campaign operation {} bond", terms.operation),
        ..EconomyPlan::new(
            bond_key(terms.operation),
            terms.attacker.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static("campaign.op.bond"),
                id: terms.operation.0,
            },
            Vec::new(),
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
        Ok(PostOutcome::NeedsApproval(_)) => return Err(BondError::NeedsApproval),
        Err(error) => return Err(refuse(error)),
    }
    live_hold(ctx, terms.operation)
        .map(|h| h.id)
        .ok_or(BondError::NoHold)
}

/// **Release the bond unspent.** Authorized because the plan's actor is the hold's
/// own placer (`WorldAuth::authorize_hold`'s first clause) — no staff, no
/// beneficiary, no second authority.
pub fn release(ctx: &mut Ctx<'_>, terms: &OperationTerms, hold: &Hold) -> Result<(), BondError> {
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Release { hold: hold.id }],
        description: format!("campaign operation {} bond released", terms.operation),
        ..EconomyPlan::new(
            consumer_key(terms.operation, ConsumerId::BondSettlement),
            terms.attacker.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static("campaign.op.bond"),
                id: terms.operation.0,
            },
            Vec::new(),
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        Ok(PostOutcome::NeedsApproval(_)) => Err(BondError::NeedsApproval),
        Err(error) => Err(refuse(error)),
    }
}

/// **Capture the bond.** The captured amount is the live hold's **own** amount,
/// read from the snapshot — cut 9 computes no number — so it is trivially
/// `≤ hold.amount` and the uncaptured remainder (zero) is released by the same
/// effect, exactly as the engine documents.
///
/// The beneficiary is the only principal the engine will let capture a
/// `Reservation` hold, so the plan posts as them. A beneficiary this actor cannot
/// act for is a refusal, which pauses and pages rather than falling back to a
/// release nobody asked for.
pub fn capture(
    ctx: &mut Ctx<'_>,
    terms: &OperationTerms,
    hold: &Hold,
    to: &(PrincipalId, mush_economy::AccountId),
) -> Result<(), BondError> {
    let (beneficiary, account) = to;
    let amount: Amount = hold.amount;
    let debit = amount.checked_neg().ok_or(BondError::NoHold)?;
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Capture {
            hold: hold.id,
            amount,
        }],
        description: format!("campaign operation {} bond forfeited", terms.operation),
        ..EconomyPlan::new(
            consumer_key(terms.operation, ConsumerId::BondSettlement),
            beneficiary.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static("campaign.op.bond"),
                id: terms.operation.0,
            },
            vec![
                RequestedLine {
                    account: hold.account,
                    delta: debit,
                    memo: SmolStr::new_static("campaign.bond.forfeit"),
                },
                RequestedLine {
                    account: *account,
                    delta: amount,
                    memo: SmolStr::new_static("campaign.bond.forfeit"),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        Ok(PostOutcome::NeedsApproval(_)) => Err(BondError::NeedsApproval),
        Err(error) => Err(refuse(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7c9_b5_the_two_key_shapes_are_deterministic_and_inside_the_kernel_cap() {
        let key = bond_key(OperationId(12));
        assert_eq!(
            (key.namespace.as_str(), key.key.as_str()),
            ("campaign.op.bond", "12")
        );
        assert_eq!(key, bond_key(OperationId(12)));
        for consumer in ConsumerId::ALL {
            let key = consumer_key(OperationId(u64::MAX), consumer);
            assert_eq!(key.namespace.as_str(), "campaign.resolve");
            assert_eq!(key, consumer_key(OperationId(u64::MAX), consumer));
            // The worst case a key can ever be: the widest id and the longest
            // consumer token, both namespaces included.
            let encoded = key.namespace.len() + 1 + key.key.len();
            assert!(
                encoded <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
                "{encoded} bytes is past the kernel's cap"
            );
        }
        // Two consumers on one operation are two keys; one consumer on two
        // operations is two keys.
        assert_ne!(
            consumer_key(OperationId(1), ConsumerId::BondSettlement),
            consumer_key(OperationId(1), ConsumerId::CampaignOutcome)
        );
        assert_ne!(
            consumer_key(OperationId(1), ConsumerId::BondSettlement),
            consumer_key(OperationId(2), ConsumerId::BondSettlement)
        );
    }

    #[test]
    fn e7c9_b5_the_bond_purpose_is_one_value_built_in_one_place() {
        assert_eq!(
            purpose(OperationId(5)),
            HoldPurpose::Reservation {
                kind: SmolStr::new_static("campaign.operation"),
                id: 5,
            }
        );
    }
}
