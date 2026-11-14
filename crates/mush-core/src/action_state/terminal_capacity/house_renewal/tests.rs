//! The House renewal's tests (doc 46 CT.3). The bodies are `renewal_twin/tests.rs`, run
//! here for [`House`] under the names they have always had. This file holds what is the
//! House's alone: its rig, its identity pin, and the three tests of the House birth and of
//! the two protocols declared side by side.
use super::*;
use crate::resolution::PrepareCtx;
use renewal_twin::tests::{self as twin, Rig, Seed, TestResult};
use renewal_twin::{LICENCE, licence_key, operation_key, pair, required};

impl Rig for House {
    const BIRTH: &'static str = "roll_mind_house_purchase";
    const BIRTH_MISSING: &'static str = "House birth";

    fn birth_seed() -> TestResult<Seed> {
        house::tests::renewal_birth_seed()
    }
    fn follow_birth_seed() -> TestResult<Seed> {
        house::tests::follow_renewal_birth_seed()
    }

    const BIRTH_AUDITS_REFUSED: &'static str = "house renewal requires its closed birth audit";
    fn without_birth_audits(registry: &StateFamilyRegistry) -> Vec<StateFamilyRegistry> {
        let mut no_house = registry.clone();
        no_house.house_purchase = None;
        vec![no_house]
    }

    fn inherit(
        builder: &mut StateFamilyRegistryBuilder,
        birth: &StateFamilyRegistry,
    ) -> TestResult {
        let legacy = renewal::tests::coexistence_registry_seed()?;
        for (key, entry) in &legacy.entries {
            if let Some(existing) = builder.entries.get(key) {
                assert_eq!(
                    existing.descriptor().canonical_value(),
                    entry.descriptor().canonical_value()
                );
            } else {
                builder.entries.insert(key.clone(), entry.clone());
            }
        }
        builder.mind_renewal = legacy.mind_renewal.clone();
        for (owner, _) in builder.entries.keys() {
            *builder.owner_counts.entry(owner.clone()).or_default() += 1;
        }
        builder.delegations = birth.delegations.clone();
        builder.family_delegations = birth.family_delegations.clone();
        builder.delegations.extend(legacy.delegations.clone());
        builder
            .family_delegations
            .extend(legacy.family_delegations.clone());
        builder.original_purchase = birth.original_purchase.clone();
        builder.paid_resale = birth.paid_resale.clone();
        // The House birth seed has no personal entitlement profile. Preserve
        // the closed purchase audits required by the coexisting renewal profile.
        builder.entitlement_purchase = legacy.entitlement_purchase.clone();
        builder.house_purchase = birth.house_purchase.clone();
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
            Some(service) => builder.house_mind_renewal_capacity_with_service(
                market,
                manifest,
                participants,
                actions,
                service,
            ),
            None => builder.house_mind_renewal_capacity(market, manifest, participants, actions),
        }
    }

    // A House offer names the issuing organisation and its scope, and the quote the
    // licence was born under; it has no seller, lineage, parents or royalty chain.
    fn disclose(
        birth: &Value,
        _grant: &Value,
        binding: &mut Value,
        basis: &mut Value,
        terms: &mut Value,
    ) -> TestResult {
        binding["issuer"] = field(birth, "issuer")?.clone();
        basis["prior_quote"] = field(birth, "quote")?.clone();
        basis["prior_quote_digest"] = field(birth, "quote_digest")?.clone();
        terms["organization"] = json!(0);
        terms["scope"] = json!(0);
        terms["remainder"] = json!({"account":2,"owner":{"kind":"organization","id":0}});
        Ok(())
    }

    fn reserve(ctx: &mut PrepareCtx<'_>, txn: &mut ActionTxn<'_>) -> Result<(), ActionStateError> {
        ctx.reserve_house_mind_renewal_completion(txn)
    }
    fn advance(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), ActionStateError> {
        ctx.advance_house_mind_renewal_completion(txn, operation)
    }
}

twin::instantiate!(House);

// House only: the personal issuer has no organisation, scope or House allocation to cross.
#[test]
fn house_birth_and_scope_are_required_through_later_term_recovery() -> TestResult {
    let mut f = twin::fixture::<House>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = twin::draft(&mut f, 31, None)?;
    twin::install(&mut f, p)?;
    let p = twin::advance(&mut f, operation, 1, 32)?;
    twin::install(&mut f, p)?;
    let p = twin::advance(&mut f, operation, 2, 33)?;
    twin::install(&mut f, p)?;
    twin::hydrate(&f, &f.disk)?;
    let birth = token(field(
        &required(&twin::view(&f), &licence_key(LICENCE, 1)?)?,
        "operation",
    )?)?;
    for corrupt in 0..4 {
        let mut disk = f.disk.clone();
        let raw = operation_key("roll_mind_house_purchase", birth)?;
        let resolved = raw.resolve(&f.store.registry)?;
        let key = (raw.owner.as_str().into(), resolved.persistence_key());
        if corrupt == 0 {
            assert!(disk.remove(&key).is_some());
        } else {
            let row = disk.get_mut(&key).ok_or("House birth row")?;
            match corrupt {
                1 => row["value"]["issuer"]["organization"] = json!(1),
                2 => row["value"]["issuer"]["scope"] = json!(1),
                _ => row["value"]["protocol"] = json!("personal_v1"),
            }
        }
        assert!(
            twin::hydrate(&f, &disk).is_err(),
            "crossed or missing House birth admitted"
        );
    }
    Ok(())
}

// House only: `require_house_birth` is the House's birth route, and the refusal of a
// royalty chain is the personal issuer's comparison inverted.
#[test]
fn house_binding_requires_exact_org_scope_quote_and_no_personal_subject() -> TestResult {
    let mut f = twin::fixture::<House>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = twin::draft(&mut f, 31, None)?;
    twin::install(&mut f, p)?;
    let p = pair::<House>(&twin::view(&f), operation)?;
    let offer = field(&p.market, "offer")?;
    let binding = p.binding()?;
    let basis = field(offer, "basis")?;
    let terms = field(offer, "terms")?;
    require_house_birth(&twin::view(&f), binding, basis, terms)?;
    for name in ["organization", "scope"] {
        let mut wrong = terms.clone();
        wrong[name] = json!(1);
        assert!(require_house_birth(&twin::view(&f), binding, basis, &wrong).is_err());
    }
    for name in ["seller", "lineage", "parents"] {
        let mut wrong = terms.clone();
        wrong[name] = json!(7);
        assert!(require_house_birth(&twin::view(&f), binding, basis, &wrong).is_err());
    }
    for name in ["prior_quote", "prior_quote_digest", "royalty_chain_digest"] {
        let mut wrong = basis.clone();
        wrong[name] = json!("foreign");
        assert!(require_house_birth(&twin::view(&f), binding, &wrong, terms).is_err());
    }
    Ok(())
}

// House only: the House fixture is the one that declares both protocols in one registry.
#[test]
fn personal_profile_rejects_house_aliases_and_resolves_each_real_owner() -> TestResult {
    let mut f = twin::fixture::<House>()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = twin::draft(&mut f, 31, None)?;
    twin::install(&mut f, p)?;
    // The personal recipe must not accept an operation already owned by House.
    assert!(
        renewal_twin::require_disjoint_operation::<Personal>(&twin::view(&f), operation).is_err()
    );
    // Both Market and Roll lookup ownership must resolve to their actual owner.
    for family in [House::MARKET, House::ROLL] {
        assert!(optional_family_value(&twin::view(&f), family, operation)?.is_some());
    }
    assert_eq!(
        operation_key("market_mind_renewal_operation", operation)?
            .owner
            .as_str(),
        "market"
    );
    assert_eq!(
        operation_key("roll_mind_renewal_operation", operation)?
            .owner
            .as_str(),
        "roll"
    );
    Ok(())
}

// House only (CT.3 mutation B7): `birth_product` is the House's hook, and the personal
// issuer's is empty. It is called here directly, as `require_house_birth` is above: through
// the validator its refusals are reached first by the birth route for every birth the two
// seeds can make (a first-recipe House birth and a two-seat Follow birth).
#[test]
fn house_birth_product_requires_the_allocation_and_its_declared_recipe() -> TestResult {
    let refusal = |result: Result<(), ActionStateError>| match result {
        Err(ActionStateError::TerminalCapacity(reason)) => reason,
        _ => "accepted",
    };
    let operation = CommitToken::from_bytes([31; 16]);
    let mut plain = twin::fixture::<House>()?;
    let p = twin::draft(&mut plain, 31, None)?;
    twin::install(&mut plain, p)?;
    let drafted = pair::<House>(&twin::view(&plain), operation)?;
    let binding = drafted.binding()?;
    House::birth_product(&twin::view(&plain), binding)?;
    let mut orphan = binding.clone();
    orphan["birth_operation"] = json!(CommitToken::from_bytes([99; 16]));
    assert_eq!(
        refusal(House::birth_product(&twin::view(&plain), &orphan)),
        "house renewal birth unavailable"
    );
    let mut undeclared = plain.store.registry.clone();
    undeclared.house_purchase = None;
    let view = View {
        registry: &undeclared,
        records: &plain.store.records,
        mutations: None,
    };
    assert_eq!(
        refusal(House::birth_product(&view, binding)),
        "house purchase recipe unavailable"
    );
    // A two-seat Follow birth, read under a registry that declares the first House recipe
    // alone: the allocation is there and well formed, and its recipe is not declared.
    let mut follow =
        twin::fixture_from_seed::<House>(1, false, false, House::follow_birth_seed()?)?;
    let p = twin::draft(&mut follow, 31, None)?;
    twin::install(&mut follow, p)?;
    let drafted = pair::<House>(&twin::view(&follow), operation)?;
    let binding = drafted.binding()?;
    House::birth_product(&twin::view(&follow), binding)?;
    let narrowed = View {
        registry: &plain.store.registry,
        records: &follow.store.records,
        mutations: None,
    };
    assert_eq!(
        refusal(House::birth_product(&narrowed, binding)),
        "house Follow recipe unavailable"
    );
    Ok(())
}

/// Doc 46 Q-46-2: every identity string of the House issuer, as a literal. The rig reads
/// the constants, so this is the one place a changed constant meets the value it must
/// hold; the CT.0 snapshot is the other.
#[test]
fn house_identity_strings_hold_todays_bytes() {
    assert_eq!(
        House::ACTIONS,
        [
            "draft_house_mind_renewal",
            "fund_house_mind_renewal",
            "settle_house_mind_renewal",
            "refund_house_mind_renewal",
            "close_house_mind_renewal",
        ]
    );
    assert_eq!(House::SERVICE_ACTION, "close_house_mind_renewal_service");
    assert_eq!(House::PROTOCOL, "house_mind_renewal_v1");
    assert_eq!(House::SERVICE_PROTOCOL, "house_mind_renewal_service_v1");
    assert_eq!(
        renewal_twin::families::<House>(),
        [
            ("market", "market_house_mind_renewal_operation", 32768),
            ("roll", "roll_house_mind_renewal_operation", 16384),
            ("roll", "roll_house_mind_renewal_head", 4096),
            ("roll", "roll_mind_personal_paid_licence", 4096),
            ("roll", "roll_mind_paid_term", 8192),
            ("roll", "roll_mind_house_term_economics", 8192),
            ("market", "market_house_mind_renewal_receipt", 8192),
        ]
    );
    assert_eq!(
        House::OTHER_ISSUER,
        [
            "roll_mind_renewal_operation",
            "market_mind_renewal_operation",
        ]
    );
    assert_eq!(
        House::PREDECESSOR_DOMAIN,
        "roll.mind.house_renewal_predecessor.v1"
    );
    assert_eq!(House::OFFER_DOMAIN, "economy.house_mind_renewal_offer.v1");
    assert_eq!(
        House::TEMPLATE_DOMAIN,
        "roll.mind.house_renewal_template.v1"
    );
    assert_eq!(
        House::CAPACITY_DOMAIN,
        "core.house_mind_renewal.capacity.v1"
    );
}

mod service_tests;
