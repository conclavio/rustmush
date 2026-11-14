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

impl Rig for House {
    const ROLES: usize = 3;

    #[inline(never)]
    fn seed() -> TestResult<Seed<Self>> {
        let mut org = TestWorld::with_modules(vec![
            Box::new(OrgModule::new()),
            Box::new(EconomyModule::new()),
        ]);
        org.exec_ok(GOD, "@org/create House");
        org.exec_ok(GOD, "@org/scope create House=renewal");
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
