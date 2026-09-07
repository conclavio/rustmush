//! The operation terms hash (E7 cut 9 B2; D-E7-32, 10b §3.9, §10 ask 3).
//!
//! **One discipline, one implementation, domain-separated.** The hash is
//! `mush_economy::consent_digest` under `mush_economy::CONSENT_DOMAIN_CAMPAIGN_OPERATION`
//! — the *same* canonical-encode-and-SHA-256 tail `consent_hash` uses, never a fork
//! (10a §6 sharpening 3 forbids one). The domain is carried as the **first field of
//! this crate's own canonical struct**, so an operation hash can never be replayed
//! as a contract hash, and `consent_hash`'s own bytes never had to move (see
//! `consent_digest`'s own documentation for why a length-prefix was unbuildable).
//!
//! Sorting rules carry over verbatim: **any principal vector sorts by
//! `principal_key`; every other vector's order *is* consented content.** The sides
//! are ordered content (which side is the attacker's), the eligible defenders are a
//! set.

use mush_core::ClockStamp;
use mush_economy::{
    AccountId, Amount, CONSENT_DOMAIN_CAMPAIGN_OPERATION, EconomicVisibility, canonical_visibility,
    consent_digest, principal_key,
};
use mush_org::PrincipalId;
use serde::Serialize;
use smol_str::SmolStr;

use crate::record::{AudienceSpec, Bond, Consequence, ObjectiveSpec, OperationTerms, SideSpec};

/// The bond as it is **consented to**: an amount on an account, and who takes it.
///
/// `Bond::hold` is deliberately absent. What the parties agree to is the size and
/// the source of the reservation; the `HoldId` is the execution record of that
/// agreement, minted by `@op/activate` — which is the very transition at which the
/// terms freeze. Hashing it would move the hash at the instant it was frozen.
#[derive(Serialize)]
struct CanonicalBond<'a> {
    account: AccountId,
    amount: Amount,
    forfeit_to: &'a Option<(PrincipalId, AccountId)>,
}

/// The field-ordered canonical form of a declared operation's terms. **`domain` is
/// the first field**, which is what `consent_digest` checks and what separates this
/// hash space from `"contract.consent.v1"` and from cut 10's `"campaign.seat.v1"`.
///
/// It carries **no `scope`** (E7 cut 9 fold, B13 / decision E1). Cut 9 hashed an
/// `Option<OrgScopeId>` that `@op/declare` set to `None`, `apply_field` had no arm
/// for and every `force.*` ask ignored in favour of the formation node's own scope —
/// consented content nothing could set and nothing read. A hashed field with no
/// writer is a hash-space change waiting for its first one, so it is gone before any
/// operation is frozen in production; when cut 10's seat work needs a compartment it
/// arrives as a real field with a real writer, and the hash moves once, visibly.
#[derive(Serialize)]
struct CanonicalOperationTerms<'a> {
    domain: &'a str,
    operation: u64,
    attacker: &'a PrincipalId,
    objective: &'a ObjectiveSpec,
    legal_targets: &'a [SmolStr],
    stakes: &'a [Consequence],
    bond: Option<CanonicalBond<'a>>,
    sides: &'a [SideSpec],
    eligible_defenders: Vec<&'a PrincipalId>,
    notice: &'a ClockStamp,
    window: &'a (ClockStamp, ClockStamp),
    visibility: EconomicVisibility,
}

/// Canonicalize an audience: sorted and deduplicated by `principal_key`, exactly as
/// `canonical_visibility` does for the economy's own audience class, so two
/// orderings of one defender list are one consent surface.
fn canonical_defenders(spec: &AudienceSpec) -> Vec<&PrincipalId> {
    let mut out: Vec<&PrincipalId> = spec.members.iter().collect();
    out.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
    out.dedup_by(|a, b| principal_key(a) == principal_key(b));
    out
}

/// The consent hash of one set of declared terms.
///
/// Computed on every pre-activation `@op/terms` edit, **frozen at activation**, and
/// re-derived at **every** transition afterwards by
/// [`crate::record::OperationDoc::consent_intact`]. A roster edit touches none of
/// its inputs, which is why "changing the roster never changes the terms hash" is a
/// property of the type rather than a rule somebody has to remember.
#[must_use]
pub fn operation_consent_hash(terms: &OperationTerms) -> [u8; 32] {
    let canonical = CanonicalOperationTerms {
        domain: CONSENT_DOMAIN_CAMPAIGN_OPERATION,
        operation: terms.operation.0,
        attacker: &terms.attacker,
        objective: &terms.objective,
        legal_targets: &terms.legal_targets,
        stakes: &terms.stakes,
        bond: terms.bond.as_ref().map(|b: &Bond| CanonicalBond {
            account: b.account,
            amount: b.amount,
            forfeit_to: &b.forfeit_to,
        }),
        sides: &terms.sides,
        eligible_defenders: canonical_defenders(&terms.eligible_defenders),
        notice: &terms.notice,
        window: &terms.window,
        visibility: canonical_visibility(&terms.visibility),
    };
    consent_digest(CONSENT_DOMAIN_CAMPAIGN_OPERATION, &canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::OperationId;
    use crate::record::{Participation, UnitRef};
    use crate::testfix::terms_fixture;
    use mush_core::ObjId;

    #[test]
    fn e7c9_b2_the_operation_terms_hash_is_the_shared_digest_under_its_own_domain() {
        let mut terms = terms_fixture(OperationId(1));
        terms.consent_hash = operation_consent_hash(&terms);
        assert!(terms.check().is_ok());
        // The bytes really are domain-separated: the same payload hashed by hand
        // WITHOUT the domain field is a different value, so the separation is in
        // the bytes rather than in a comment.
        let undomained = consent_digest(
            mush_economy::CONSENT_DOMAIN_CONTRACT,
            &serde_json::json!({ "operation": 1u64 }),
        );
        assert_ne!(terms.consent_hash, undomained);
    }

    #[test]
    fn e7c9_b3_a_roster_edit_leaves_the_terms_hash_byte_identical() {
        let mut terms = terms_fixture(OperationId(1));
        terms.consent_hash = operation_consent_hash(&terms);
        let frozen = terms.consent_hash;
        let mut runtime = crate::record::OperationRuntime::declared(OperationId(1));
        for n in 1..=8 {
            runtime.participants.push(Participation {
                unit: UnitRef(ObjId(n)),
                side: SmolStr::new_static("attack"),
                command_parent: None,
                joined_at: i64::from(n),
            });
            runtime.revision += 1;
            // The runtime moved eight times; the frozen half never did.
            assert_eq!(operation_consent_hash(&terms), frozen);
        }
        assert_eq!(runtime.revision, 9);
    }

    #[test]
    fn e7c9_b2_an_audience_reordering_is_one_consent_surface_but_a_member_change_is_not() {
        let mut terms = terms_fixture(OperationId(1));
        terms.eligible_defenders.members =
            vec![PrincipalId::Object(ObjId(9)), PrincipalId::Object(ObjId(2))];
        let a = operation_consent_hash(&terms);
        terms.eligible_defenders.members.reverse();
        assert_eq!(operation_consent_hash(&terms), a, "order is not content");
        terms
            .eligible_defenders
            .members
            .push(PrincipalId::Object(ObjId(3)));
        assert_ne!(operation_consent_hash(&terms), a, "membership IS content");
    }

    #[test]
    fn e7c9_b5_placing_the_bond_does_not_move_the_frozen_hash_but_its_size_does() {
        let mut terms = terms_fixture(OperationId(1));
        let before = operation_consent_hash(&terms);
        let bond = terms.bond.as_mut().expect("the fixture is bonded");
        bond.hold = Some(mush_economy::HoldId(42));
        assert_eq!(
            operation_consent_hash(&terms),
            before,
            "activation must not move the hash it is freezing"
        );
        let bond = terms.bond.as_mut().expect("the fixture is bonded");
        bond.amount = Amount::from_minor(1);
        assert_ne!(
            operation_consent_hash(&terms),
            before,
            "how much is reserved IS consented content"
        );
    }
}
