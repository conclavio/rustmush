//! The personal renewal's tests (doc 46 CT.3). The bodies are `renewal_twin/tests.rs`, run
//! here for [`Personal`] under the names they have always had. This file holds what is the
//! personal issuer's alone: its rig, its identity pin, and the two things other families'
//! tests take from a personal renewal.
use super::*;
use crate::resolution::PrepareCtx;
use renewal_twin::tests::{self as twin, Rig, Seed, TestResult};

impl Rig for Personal {
    const BIRTH: &'static str = "roll_mind_entitlement_purchase";
    const BIRTH_MISSING: &'static str = "personal birth";

    fn birth_seed() -> TestResult<Seed> {
        entitlement::tests::renewal_birth_seed()
    }
    fn follow_birth_seed() -> TestResult<Seed> {
        entitlement::tests::follow_renewal_birth_seed()
    }

    const BIRTH_AUDITS_REFUSED: &'static str = "renewal requires both closed purchase audits";
    fn without_birth_audits(registry: &StateFamilyRegistry) -> Vec<StateFamilyRegistry> {
        let mut no_original = registry.clone();
        no_original.original_purchase = None;
        let mut no_entitlement = registry.clone();
        no_entitlement.entitlement_purchase = None;
        vec![no_original, no_entitlement]
    }

    fn inherit(
        builder: &mut StateFamilyRegistryBuilder,
        birth: &StateFamilyRegistry,
    ) -> TestResult {
        for (owner, _) in builder.entries.keys() {
            *builder.owner_counts.entry(owner.clone()).or_default() += 1;
        }
        builder.delegations = birth.delegations.clone();
        builder.family_delegations = birth.family_delegations.clone();
        builder.original_purchase = birth.original_purchase.clone();
        builder.paid_resale = birth.paid_resale.clone();
        builder.entitlement_purchase = birth.entitlement_purchase.clone();
        Ok(())
    }

    fn declare(
        builder: &mut StateFamilyRegistryBuilder,
        market: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service: Option<DefinitionRef>,
    ) -> Result<(), ActionStateError> {
        match service {
            Some(service) => builder.mind_renewal_capacity_with_service(
                market,
                manifest,
                participants,
                actions,
                service,
            ),
            None => builder.mind_renewal_capacity(market, manifest, participants, actions),
        }
    }

    // A personal offer names a seller, a lineage and its parents, and carries the grant's
    // royalty chain; its prior quote is inert fixture data.
    fn disclose(
        _birth: &Value,
        grant: &Value,
        _binding: &mut Value,
        basis: &mut Value,
        terms: &mut Value,
    ) -> TestResult {
        basis["prior_quote"] = json!(CommitToken::from_bytes([3; 16]));
        basis["prior_quote_digest"] = json!(DefinitionDigest::from_bytes([5; 32]));
        basis["royalty_chain_digest"] = field(field(grant, "royalty_chain")?, "digest")?.clone();
        terms["lineage"] = json!("original");
        terms["seller"] = json!(8);
        terms["parents"] = json!([]);
        terms["remainder"] = json!({"account":2,"owner":{"kind":"object","id":8}});
        Ok(())
    }

    fn reserve(ctx: &mut PrepareCtx<'_>, txn: &mut ActionTxn<'_>) -> Result<(), ActionStateError> {
        ctx.reserve_mind_renewal_completion(txn)
    }
    fn advance(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), ActionStateError> {
        ctx.advance_mind_renewal_completion(txn, operation)
    }
}

twin::instantiate!(Personal);

/// 17g C2f: the owner-index rig. A fresh renewal of the entitlement birth
/// (holder 7, licence 1) drafted and funded, and its Grant prepared with
/// `rows`' extra writes (the Grant keeps the holder, so it writes none).
pub(in crate::action_state::terminal_capacity) fn renewal_grant(
    rows: Option<owner_index::tests::IndexWriter<'_>>,
) -> TestResult<PreparedTxn> {
    let mut f = twin::fixture::<Personal>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = twin::draft(&mut f, 31, None)?;
    twin::install(&mut f, p)?;
    let p = twin::advance(&mut f, operation, 1, 32)?;
    twin::install(&mut f, p)?;
    twin::advance_full(&mut f, operation, 2, 33, None, rows)
}

// Reuse the real family declaration of the historical personal protocol when
// exercising independent House transactions. This does not copy native state.
pub(in crate::action_state::terminal_capacity) fn coexistence_registry_seed()
-> TestResult<StateFamilyRegistry> {
    Ok(twin::fixture::<Personal>()?.store.registry.clone())
}

/// Doc 46 Q-46-2: every identity string of the personal issuer, as a literal. The rig
/// reads the constants, so this is the one place a changed constant meets the value it
/// must hold; the CT.0 snapshot is the other.
#[test]
fn personal_identity_strings_hold_todays_bytes() {
    assert_eq!(
        Personal::ACTIONS,
        [
            "draft_mind_renewal",
            "fund_mind_renewal",
            "settle_mind_renewal",
            "refund_mind_renewal",
            "close_mind_renewal",
        ]
    );
    assert_eq!(Personal::SERVICE_ACTION, "close_mind_renewal_service");
    assert_eq!(Personal::PROTOCOL, "personal_mind_renewal_v1");
    assert_eq!(
        Personal::SERVICE_PROTOCOL,
        "personal_mind_renewal_service_v1"
    );
    assert_eq!(
        renewal_twin::families::<Personal>(),
        [
            ("market", "market_mind_renewal_operation", 32768),
            ("roll", "roll_mind_renewal_operation", 16384),
            ("roll", "roll_mind_renewal_head", 4096),
            ("roll", "roll_mind_personal_paid_licence", 4096),
            ("roll", "roll_mind_paid_term", 8192),
            ("roll", "roll_mind_paid_term_economics", 8192),
            ("market", "market_mind_renewal_receipt", 8192),
        ]
    );
    assert_eq!(
        Personal::OTHER_ISSUER,
        [
            "roll_house_mind_renewal_operation",
            "market_house_mind_renewal_operation",
        ]
    );
    assert_eq!(
        Personal::PREDECESSOR_DOMAIN,
        "roll.mind.renewal_predecessor.v1"
    );
    assert_eq!(Personal::OFFER_DOMAIN, "economy.mind_renewal_offer.v1");
    assert_eq!(Personal::TEMPLATE_DOMAIN, "roll.mind.renewal_template.v1");
    assert_eq!(Personal::CAPACITY_DOMAIN, "core.mind_renewal.capacity.v1");
    assert_eq!(
        renewal_twin::TRANSITIONS,
        ["draft", "fund", "grant", "refund", "unfunded_close"]
    );
}

mod service_tests;
