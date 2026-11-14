//! The House renewal family's tests: the House rig, and every scenario of
//! `crate::renewal_twin::tests` run for the House issuer.
//!
//! The rig mirrors the personal one: the same zero balances, the same clock (1000) and offer
//! window (100 IC seconds). What differs is the subject: the charter's title belongs to an
//! organisation on one of its scopes, the remainder is paid to that organisation's treasury
//! there, and there is no lineage and no royalty parent. The organisation and its scope are
//! made by the real Org commands, so the Org owner the House terms read is the registered one.
use super::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};
use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use crate::record::{
    Account, AccountStatus, AssetLifecycle, CreditPolicy, EconomicAsset, InstrumentDef,
    InstrumentKind, InstrumentStatus, NegativePolicy,
};
use crate::renewal_twin::House;
use crate::renewal_twin::history::OfferBook;
use crate::renewal_twin::tests::{self as twin, Fixture, Rig, Seed, TestResult, token};
use crate::{AccountId, Amount, EconomyModule, EconomyStore, InstrumentId};
use mush_core::{ClockStamp, DefinitionDigest, GOD, ObjId, ObjKind};
use mush_org::{OrgId, OrgModule, OrgScopeId, OrgStore, PrincipalId};
use mush_testkit::TestWorld;

fn account(
    id: u64,
    owner: PrincipalId,
    instrument: &InstrumentId,
    scope: Option<OrgScopeId>,
) -> Account {
    Account {
        id: AccountId(id),
        owner,
        instrument: instrument.clone(),
        scope,
        name: "house renewal fixture".into(),
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        last_hold_seq: 0,
        revision: 1,
        approval: None,
    }
}

/// The House seed. `first` runs over the Org world before it is taken apart, with the
/// offer's scope, so that a House-only test can suspend the organisation or retire the
/// scope by the real commands.
#[inline(never)]
fn seeded(first: fn(&mut TestWorld, OrgScopeId)) -> TestResult<Seed<House>> {
    let mut org = TestWorld::with_modules(vec![
        Box::new(OrgModule::new()),
        Box::new(EconomyModule::new()),
    ]);
    org.exec_ok(GOD, "@org/create House");
    org.exec_ok(GOD, "@org/scope create House=renewal");
    // A second scope of the same organisation, for a retry on another line.
    org.exec_ok(GOD, "@org/scope create House=other");
    let (organization, scope) = {
        let state = org
            .world()
            .module_state::<OrgStore>()
            .ok_or("org owner")?
            .state();
        let organization = state.orgs().next().ok_or("organization")?.id;
        let scope = state
            .scopes_in(organization)
            .find(|s| s.name.as_str() == "renewal")
            .ok_or("scope")?
            .id;
        (organization, scope)
    };
    first(&mut org, scope);
    let mut world = org.into_engine().world;
    world.set_now(1000);
    let buyer = world.create(ObjKind::Player, "House renewal buyer", GOD, None)?;
    let charter = world.create(ObjKind::Thing, "House renewal charter", GOD, None)?;
    let instrument = InstrumentId::new("cr").ok_or("instrument")?;
    let registry =
        PrincipalId::System(mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?);
    let house = PrincipalId::Organization(organization);
    let mut state = EconomyStore::default();
    state.snapshot.put_instrument(InstrumentDef {
        id: instrument.clone(),
        ruleset: "test".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::Forbid,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    });
    state
        .snapshot
        .put_account(account(0, PrincipalId::Object(buyer), &instrument, None));
    state
        .snapshot
        .put_account(account(1, house.clone(), &instrument, Some(scope)));
    state
        .snapshot
        .put_account(account(2, registry.clone(), &instrument, None));
    state.snapshot.put_asset(EconomicAsset {
        object: charter,
        title_owner: house.clone(),
        scope: Some(scope),
        assignment: None,
        maintenance_account: None,
        catalog: None,
        encumbrances: Vec::new(),
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 0,
        revision: 1,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    });
    let at = world.world_ic_now();
    let offer = RenewalOffer {
        format: 1,
        basis: RenewalBasisV1 {
            format: 1,
            licence: 7,
            predecessor_generation: 1,
            next_generation: 2,
            predecessor_operation: token(1),
            prior_quote: token(2),
            prior_quote_digest: DefinitionDigest::from_bytes([1; 32]),
            product_digest: DefinitionDigest::from_bytes([2; 32]),
            template_digest: DefinitionDigest::from_bytes([3; 32]),
            grant_economics_digest: DefinitionDigest::from_bytes([4; 32]),
        },
        terms: RenewalOfferTermsV1 {
            format: 1,
            buyer: buyer.into(),
            organization,
            scope,
            charter,
            title_revision: 1,
            payer: AccountId(0),
            instrument,
            price: Amount::from_minor(100),
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: AccountId(2),
                    owner: registry,
                }),
            },
            venue: None,
            remainder: Recipient {
                account: AccountId(1),
                owner: house,
            },
            not_before: at.clone(),
            expires_at: ClockStamp {
                clock: at.clock,
                at: at.at + 100,
            },
        },
    };
    Ok(Seed {
        world,
        state,
        offer,
    })
}

/// The scope of that name in the organisation of that name.
fn scope_named(f: &Fixture<House>, organization: &str, scope: &str) -> TestResult<OrgScopeId> {
    let state = f
        .world
        .module_state::<OrgStore>()
        .ok_or("org owner")?
        .state();
    let organization = state
        .orgs()
        .find(|o| o.name.as_str() == organization)
        .ok_or("organization")?
        .id;
    Ok(state
        .scopes_in(organization)
        .find(|s| s.name.as_str() == scope)
        .ok_or("scope")?
        .id)
}
/// The offer, the charter's title and the treasury moved to another scope together.
fn move_to(f: &mut Fixture<House>, scope: OrgScopeId) -> TestResult {
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.scope = Some(scope);
    f.state.snapshot.put_asset(title);
    let mut treasury = f
        .state
        .snapshot
        .account(f.offer.terms.remainder.account)
        .ok_or("treasury")?
        .clone();
    treasury.scope = Some(scope);
    f.state.snapshot.put_account(treasury);
    f.offer.terms.scope = scope;
    Ok(())
}

impl Rig for House {
    const ROLES: usize = 3;
    const TITLES: usize = 1;

    fn seed() -> TestResult<Seed<Self>> {
        seeded(|_, _| {})
    }

    /// Registry, venue and the House's own remainder.
    fn widen(f: &mut Fixture<Self>) -> TestResult {
        let owner = f
            .world
            .create(ObjKind::Player, "House renewal venue", GOD, None)?;
        let mut venue = f
            .state
            .snapshot
            .account(AccountId(0))
            .ok_or("account")?
            .clone();
        venue.id = AccountId(3);
        venue.owner = PrincipalId::Object(owner);
        f.state.snapshot.put_account(venue);
        f.offer.terms.venue = Some(GrossRecipientCut {
            permille: 50,
            recipient: Some(Recipient {
                account: AccountId(3),
                owner: PrincipalId::Object(owner),
            }),
        });
        check_eq!(
            f.offer.terms.remainder.owner,
            PrincipalId::Organization(f.offer.terms.organization)
        );
        Ok(())
    }

    twin::views!();

    fn book_mut(state: &mut EconomyStore) -> &mut OfferBook<Self> {
        &mut state.house_renewal_offers
    }
    fn absent(state: &EconomyStore) -> Result<(), super::RenewalAuditError> {
        state.require_no_house_renewal_history()
    }
    const FOLD_KEY: &'static str = "house_renewal_receipts";
    /// A House has no seller, so the stranger is a character made for the purpose.
    fn stranger(f: &mut Fixture<Self>) -> TestResult<ObjId> {
        Ok(f.world
            .create(ObjKind::Player, "House renewal stranger", GOD, None)?)
    }
    /// The organisation's charter leaves the game.
    fn lapse(f: &mut Fixture<Self>) -> TestResult {
        f.world
            .get_mut(f.offer.terms.charter)
            .ok_or("charter")?
            .flags
            .insert(mush_core::Flags::GOING);
        Ok(())
    }
    /// Another scope of the same organisation: the title and the treasury move there with
    /// the offer, so the offer is current.
    fn other_line(f: &mut Fixture<Self>) -> TestResult {
        let other = f
            .world
            .module_state::<OrgStore>()
            .ok_or("org owner")?
            .state()
            .scopes_in(f.offer.terms.organization)
            .find(|s| s.name.as_str() == "other")
            .ok_or("other scope")?
            .id;
        check_ne!(other, f.offer.terms.scope);
        move_to(f, other)
    }
    /// The organisation and its scope.
    fn widest(offer: &mut RenewalOffer) -> TestResult {
        offer.terms.organization = OrgId(u64::MAX);
        offer.terms.scope = OrgScopeId(u64::MAX);
        offer.terms.remainder.owner = PrincipalId::Organization(offer.terms.organization);
        Ok(())
    }
    /// A House offer names its organisation and its scope.
    fn own_wire_is_strict(offer: &RenewalOffer) -> TestResult {
        for field in ["organization", "scope"] {
            let mut missing = serde_json::to_value(offer)?;
            missing
                .get_mut("terms")
                .and_then(|v| v.as_object_mut())
                .ok_or("terms")?
                .remove(field);
            check!(
                serde_json::from_value::<RenewalOffer>(missing).is_err(),
                "{field}"
            );
        }
        Ok(())
    }
}

twin::instantiate!(crate::renewal_twin::House);

/// Every identity string of the issuer against a literal. The shared scenarios read the
/// issuer's constants, so a constant that moved would move the rig with it and they
/// would still pass; this second copy of the strings, and the CT.0 snapshot, would not.
#[test]
fn house_identity_strings_hold_todays_bytes() {
    use crate::renewal_twin::Issuer;
    assert_eq!(House::DRAFT_ACTION, "draft_house_mind_renewal");
    assert_eq!(House::CLOSE_ACTION, "close_house_mind_renewal");
    assert_eq!(House::FUND_ACTION, "fund_house_mind_renewal");
    assert_eq!(House::GRANT_ACTION, "settle_house_mind_renewal");
    assert_eq!(House::REFUND_ACTION, "refund_house_mind_renewal");
    assert_eq!(
        House::SERVICE_CLOSE_ACTION,
        "close_house_mind_renewal_service"
    );
    assert_eq!(House::NAMESPACE, "market.mind.house_renewal.v1");
    assert_eq!(House::OFFER_PREFIX, "house-renewal-offer:");
    assert_eq!(House::SERVICE_PREFIX, "house-renewal-service-close:");
    assert_eq!(House::PROTOCOL, "house_mind_renewal_v1");
    assert_eq!(House::SERVICE_PROTOCOL, "house_mind_renewal_service_v1");
    assert_eq!(House::OFFER_DOMAIN, "economy.house_mind_renewal_offer.v1");
    assert_eq!(House::CONTRACT_NAME, "House mind renewal offer");
    assert_eq!(House::FUNDING_DESCRIPTION, "house mind renewal funding");
    assert_eq!(
        House::BIRTH_ROUTE,
        crate::managed_paid_resale::PaidBirthRouteV1::House
    );
    // The names the rest of the crate and the other crates read are the same strings.
    assert_eq!(super::DRAFT_ACTION, House::DRAFT_ACTION);
    assert_eq!(super::CLOSE_ACTION, House::CLOSE_ACTION);
    assert_eq!(super::FUND_ACTION, House::FUND_ACTION);
    assert_eq!(super::GRANT_ACTION, House::GRANT_ACTION);
    assert_eq!(super::REFUND_ACTION, House::REFUND_ACTION);
    assert_eq!(super::SERVICE_CLOSE_ACTION, House::SERVICE_CLOSE_ACTION);
    assert_eq!(super::NAMESPACE, House::NAMESPACE);
    assert_eq!(super::service::PREFIX, House::SERVICE_PREFIX);
}

/// House only: the issuer's current authority is the organisation store's. The organisation
/// is active, the scope is active and is that organisation's, and the charter's title is the
/// organisation's on that scope. The personal issuer has no organisation; its counterpart is
/// the seller's personal title (`every_title_an_offer_stands_on_is_current_and_fenced`,
/// `managed_renewal::tests::a_personal_offer_refuses_a_title_on_a_scope`).
#[test]
fn a_house_offer_needs_its_organisation_scope_and_title_current() -> TestResult {
    Fixture::<House>::new()?.prepare()?;

    let f = Fixture::from_seed(seeded(|org, _| {
        org.exec_ok(GOD, "@org/status House=suspended");
    })?)?;
    check!(f.prepare().is_err(), "a suspended organisation");

    let f = Fixture::from_seed(seeded(|org, scope| {
        org.exec_ok(GOD, &format!("@org/scope retire House=#{}", scope.0));
    })?)?;
    check!(f.prepare().is_err(), "a retired scope");

    // Another organisation's scope, with the title and the treasury on it: only the scope's
    // owner is wrong.
    let mut f = Fixture::from_seed(seeded(|org, _| {
        org.exec_ok(GOD, "@org/create Other");
        org.exec_ok(GOD, "@org/scope create Other=elsewhere");
    })?)?;
    let elsewhere = scope_named(&f, "Other", "elsewhere")?;
    move_to(&mut f, elsewhere)?;
    check!(f.prepare().is_err(), "another organisation's scope");

    // The title on another scope of the same organisation.
    let mut f = Fixture::<House>::new()?;
    let other = scope_named(&f, "House", "other")?;
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.scope = Some(other);
    f.state.snapshot.put_asset(title);
    check!(f.prepare().is_err(), "a title on another scope");

    // The title held by somebody else, on the right scope.
    let mut f = Fixture::<House>::new()?;
    let stranger = House::stranger(&mut f)?;
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.title_owner = PrincipalId::Object(stranger);
    f.state.snapshot.put_asset(title);
    check!(
        f.prepare().is_err(),
        "a title the organisation does not hold"
    );
    Ok(())
}

/// House only: the remainder is paid to the organisation's treasury on the offer's scope.
/// The personal remainder is the seller's own account, which the purchase recipe both
/// families' births share already holds to the seller.
#[test]
fn a_house_offer_pays_its_remainder_to_the_scoped_treasury() -> TestResult {
    // An account that is somebody else's, named as such in the terms.
    let mut f = Fixture::<House>::new()?;
    let stranger = House::stranger(&mut f)?;
    let mut account = f
        .state
        .snapshot
        .account(f.offer.terms.remainder.account)
        .ok_or("treasury")?
        .clone();
    account.owner = PrincipalId::Object(stranger);
    f.state.snapshot.put_account(account);
    f.offer.terms.remainder.owner = PrincipalId::Object(stranger);
    check!(
        f.prepare().is_err(),
        "a remainder that is not the organisation's"
    );

    // The organisation's own account, off the scope or on another.
    let other = scope_named(&*Fixture::<House>::new()?, "House", "other")?;
    for scope in [None, Some(other)] {
        let mut f = Fixture::<House>::new()?;
        let mut account = f
            .state
            .snapshot
            .account(f.offer.terms.remainder.account)
            .ok_or("treasury")?
            .clone();
        account.scope = scope;
        f.state.snapshot.put_account(account);
        check!(
            f.prepare().is_err(),
            "a treasury that is not on the scope: {scope:?}"
        );
    }
    Ok(())
}

/// House only: a House renewal is filed under the issuing organisation's scope. A personal
/// one is filed under none (`managed_renewal::tests`).
#[test]
fn a_house_contract_is_filed_under_the_issuers_scope() -> TestResult {
    let mut f = Fixture::<House>::new()?;
    let (out, _) = f.publish()?;
    let contract = f
        .state
        .snapshot
        .contract(out.contract())
        .ok_or("contract")?;
    check_eq!(contract.consent.scope, Some(f.offer.terms.scope));
    Ok(())
}
