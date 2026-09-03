//! `plan_hash` — the canonical SHA-256 of a plan's economic identity (E2a, R-B3).
//!
//! It is the linchpin of idempotency (same key + same plan ⇒ same result) and of
//! the future E3 approval round-trip (approvals bind to the hash). Two properties
//! make it safe:
//!
//! - **Determinism.** The hash is a pure function of the plan ALONE (never the
//!   snapshot), over field-ordered structs with the lines **sorted by a total key**
//!   (account, delta, memo) — so reordered request lines and any map-iteration
//!   difference hash identically. No `HashMap`-backed value is ever hashed.
//! - **Stability across a retry.** It EXCLUDES ephemeral concurrency metadata —
//!   `expected_revisions`, timestamps, and the idempotency key itself — so a retry
//!   rebuilt against a re-fetched snapshot still matches, and "same key, different
//!   plan" (a real conflict) is the only thing that changes the hash.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ids::TransactionId;
use crate::plan::{AssetOp, CustodyOp, EconomyPlan, HoldOp};
use crate::record::EconomicVisibility;
use mush_org::PrincipalId;

#[derive(Serialize)]
struct CanonicalLine<'a> {
    account: u64,
    delta: i64,
    memo: &'a str,
}

#[derive(Serialize)]
struct CanonicalPlan<'a> {
    actor: &'a PrincipalId,
    cause: &'a crate::record::EconomicCause,
    description: &'a str,
    /// By reference since E4b2 (an audience list is not `Copy`). The serialized
    /// bytes are identical either way, which is what keeps both digest pins green.
    visibility: &'a EconomicVisibility,
    lines: Vec<CanonicalLine<'a>>,
    /// v2 (E3a). Skipped when empty, so an E2-shaped plan hashes **byte-identically**
    /// under E3 and journal-derived idempotency for pre-upgrade entries keeps working.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    asset_ops: Vec<&'a AssetOp>,
    /// v2 (E3a). Skipped when empty, as above.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    custody_ops: Vec<&'a CustodyOp>,
    /// v3 (E4a, invariant 27). Skipped when empty, so an E2- *or* E3-shaped plan
    /// hashes byte-identically under E4 - both hard-coded digest pins below prove it,
    /// and every stored idempotency record keeps resolving.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hold_ops: Vec<&'a HoldOp>,
}

/// The canonical SHA-256 hash of `plan`'s economic identity.
#[must_use]
pub fn plan_hash(plan: &EconomyPlan) -> [u8; 32] {
    let mut lines: Vec<CanonicalLine<'_>> = plan
        .requested_lines
        .iter()
        .map(|l| CanonicalLine { account: l.account.0, delta: l.delta.minor(), memo: l.memo.as_str() })
        .collect();
    // Total order over (account, delta, memo) — reordered request lines hash the same.
    lines.sort_by(|a, b| {
        (a.account, a.delta, a.memo).cmp(&(b.account, b.delta, b.memo))
    });
    // Total order over the ops (B7): asset ops by (object, discriminant, canonical
    // bytes) — the byte tiebreak makes it total even for two ops of the same kind on
    // the same object; custody ops by (object, vault, discriminant), which is total
    // because at most one custody op per object survives validation.
    let mut asset_ops: Vec<(AssetOpKey, &AssetOp)> =
        plan.asset_ops.iter().map(|op| (asset_op_key(op), op)).collect();
    asset_ops.sort_by(|a, b| a.0.cmp(&b.0));
    let mut custody_ops: Vec<(CustodyOpKey, &CustodyOp)> =
        plan.custody_ops.iter().map(|op| (custody_op_key(op), op)).collect();
    custody_ops.sort_by_key(|(k, _)| *k);
    // Hold ops by (hold-or-account id, discriminant, canonical bytes) - the byte
    // tiebreak makes it total, and the sort is STABLE so two identical `Place`s (two
    // genuinely distinct reservations with the same shape) keep their relative order
    // instead of being an unordered pair.
    let mut hold_ops: Vec<(HoldOpKey, &HoldOp)> =
        plan.hold_ops.iter().map(|op| (hold_op_key(op), op)).collect();
    hold_ops.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical = CanonicalPlan {
        actor: &plan.actor,
        cause: &plan.cause,
        description: &plan.description,
        visibility: &plan.visibility,
        lines,
        asset_ops: asset_ops.iter().map(|(_, op)| *op).collect(),
        custody_ops: custody_ops.iter().map(|(_, op)| *op).collect(),
        hold_ops: hold_ops.iter().map(|(_, op)| *op).collect(),
    };
    // serde_json over field-ordered structs + a sorted Vec is deterministic here
    // (no floats, no maps). This is the tactics `canon()` discipline for money.
    let bytes = serde_json::to_vec(&canonical).expect("canonical plan serializes");
    let out: [u8; 32] = Sha256::digest(&bytes).into();
    out
}

/// The canonical serialization of one op — the byte tiebreak of the asset-op total
/// order. Deterministic for the same reason the whole plan is: field-ordered structs,
/// no floats, no maps.
fn canonical_bytes<T: Serialize>(op: &T) -> Vec<u8> {
    serde_json::to_vec(op).expect("canonical op serializes")
}

/// The total sort key of an asset op — see [`asset_op_key`].
pub(crate) type AssetOpKey = (u32, u8, Vec<u8>);
/// The total sort key of a custody op — see [`custody_op_key`].
pub(crate) type CustodyOpKey = (u32, u64, u8);
/// The total sort key of a hold op — see [`hold_op_key`].
pub(crate) type HoldOpKey = (u64, u8, Vec<u8>);

/// The total sort key of an asset op: `(object, op ordinal, canonical bytes)`. The
/// byte tiebreak is what makes the order TOTAL — two `Release`s on the same object
/// differ only in their payload (B7). The engine sorts by the same key, so the order
/// ops are APPLIED in is exactly the order they are HASHED in.
pub(crate) fn asset_op_key(op: &AssetOp) -> AssetOpKey {
    (op.object().0, op.ordinal(), canonical_bytes(op))
}

/// The total sort key of a custody op: `(object, vault, direction)`. Total because
/// validation admits at most one custody op per object.
pub(crate) fn custody_op_key(op: &CustodyOp) -> CustodyOpKey {
    (op.object().0, op.vault().0, op.ordinal())
}

/// The total sort key of a hold op (E4a, invariant 27): `(hold-or-account id, op
/// ordinal, canonical bytes)`.
///
/// The **subject id sorts first**, and a `Place` has no hold id yet, so it sorts by
/// the account it reserves against — an id from an unrelated space. So this is
/// emphatically *not* "every `Place` before every capture": a `Place` on account 9
/// sorts after a capture of hold 3. The ordinal only breaks ties between ops that
/// share a subject id, and the canonical bytes break the rest, which is all
/// invariant 27 needs: the order is **total and deterministic**, so two plans
/// differing only in op order are one plan and the order ops APPLY in (the engine
/// sorts by this same key) is the order they are HASHED in.
pub(crate) fn hold_op_key(op: &HoldOp) -> HoldOpKey {
    (op.subject(), op.ordinal(), canonical_bytes(op))
}

/// The canonical hash of the reversal of `target` — derived from the TARGET alone,
/// not the caller's attribution. A reversal's identity is which transaction it
/// undoes (its lines are the target's, negated); the staff reason and actor are
/// attribution, not identity. So two `reverse` calls for the same target hash
/// identically and the second cleanly resolves to `AlreadyPosted` rather than a
/// spurious idempotency conflict.
#[must_use]
pub fn reversal_hash(target: TransactionId) -> [u8; 32] {
    let out: [u8; 32] = Sha256::digest(format!("econ.reverse:{}", target.0).as_bytes()).into();
    out
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;
    use mush_org::PrincipalId;

    use super::*;
    use crate::amount::Amount;
    use crate::ids::{AccountId, VaultId};
    use crate::plan::RequestedLine;
    use crate::record::{EconomicCause, Encumbrance, IdempotencyKey};

    /// The digest of [`pin_plan_v3`] computed on the **pre-v3** (E3a) hash function
    /// and hard-coded here (E4a invariant 27), exactly as `PIN_DIGEST` pins E2's:
    /// v3 adds `hold_ops`, and an E3-shaped plan must keep hashing byte-identically
    /// or every stored idempotency record for a title/custody post is orphaned.
    const PIN_DIGEST_V3: &str = "b780e3a0974e641bbe381dace3aba60a1b3b57f0dcfef5e39472e41a7b185949";

    #[test]
    fn v3_reproduces_the_hard_coded_e3_digest_for_a_title_and_custody_plan() {
        assert_eq!(hex(&plan_hash(&pin_plan_v3())), PIN_DIGEST_V3);
    }

    /// The digest of [`pin_plan`] computed on the **pre-v2** (E2a) hash function and
    /// hard-coded here (D-E3-10 / C7): a later field reorder, a renamed variant, or a
    /// v2 op vector that stops skipping when empty all break this, loudly, instead of
    /// silently orphaning every stored idempotency record.
    const PIN_DIGEST: &str = "9c917eee84e001912480fdb8306b9a762ed2e3f50d1014e34ae38601c490d74e";

    fn pin_plan() -> EconomyPlan {
        let mut p = EconomyPlan::new(
            IdempotencyKey::new("pin", "v2"),
            PrincipalId::Object(ObjId(7)),
            EconomicCause::Staff { reason: "pin".into() },
            vec![
                RequestedLine {
                    account: AccountId(1),
                    delta: Amount::from_minor(-250),
                    memo: "out".into(),
                },
                RequestedLine {
                    account: AccountId(2),
                    delta: Amount::from_minor(250),
                    memo: "in".into(),
                },
            ],
        );
        p.description = "hash pin".into();
        p
    }

    /// The E3-shaped pin plan (money lines + title ops + a custody op) — the v3
    /// digest anchor. Its digest is computed on the **v2** hash function and
    /// hard-coded, so v3's new `hold_ops` field must stay skipped-when-empty.
    fn pin_plan_v3() -> EconomyPlan {
        let mut p = EconomyPlan::new(
            IdempotencyKey::new("pin", "v3"),
            PrincipalId::Object(ObjId(11)),
            EconomicCause::Staff { reason: "pin3".into() },
            vec![
                RequestedLine {
                    account: AccountId(3),
                    delta: Amount::from_minor(-700),
                    memo: "pay".into(),
                },
                RequestedLine {
                    account: AccountId(4),
                    delta: Amount::from_minor(700),
                    memo: "recv".into(),
                },
            ],
        );
        p.description = "hash pin v3".into();
        p.asset_ops = vec![
            AssetOp::TransferTitle {
                object: ObjId(21),
                to: PrincipalId::Object(ObjId(12)),
                to_scope: None,
            },
            AssetOp::Encumber {
                object: ObjId(22),
                encumbrance: Encumbrance::Lien {
                    creditor: PrincipalId::Object(ObjId(13)),
                    amount: Amount::from_minor(50),
                    instrument: crate::ids::InstrumentId::new("gold").unwrap(),
                },
            },
        ];
        p.custody_ops = vec![CustodyOp::Deposit { object: ObjId(23), vault: VaultId(5) }];
        p
    }

    fn hex(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn v2_reproduces_the_hard_coded_e2_digest_for_a_money_only_plan() {
        assert_eq!(hex(&plan_hash(&pin_plan())), PIN_DIGEST);
    }

    #[test]
    fn empty_op_vectors_are_skipped_entirely() {
        // Not merely "the same as an empty array" — absent, which is what makes the
        // pinned E2 digest reproduce.
        let p = pin_plan();
        let json = serde_json::to_string(&CanonicalPlan {
            actor: &p.actor,
            cause: &p.cause,
            description: &p.description,
            visibility: &p.visibility,
            lines: Vec::new(),
            asset_ops: Vec::new(),
            custody_ops: Vec::new(),
            hold_ops: Vec::new(),
        })
        .unwrap();
        assert!(!json.contains("asset_ops"), "empty asset_ops must not serialize");
        assert!(!json.contains("custody_ops"), "empty custody_ops must not serialize");
        assert!(!json.contains("hold_ops"), "empty hold_ops must not serialize");
    }
}
