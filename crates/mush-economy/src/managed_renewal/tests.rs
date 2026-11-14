//! The personal renewal family's tests: the personal rig, and every scenario of
//! `crate::renewal_twin::tests` run for the personal issuer under the names and paths the
//! family's tests always had.
use super::{RenewalBasisV1, RenewalLineageV1, RenewalOffer, RenewalOfferTermsV1};
use crate::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::record::{
    Account, AccountStatus, AssetLifecycle, CreditPolicy, EconomicAsset, InstrumentDef,
    InstrumentKind, InstrumentStatus, NegativePolicy,
};
use crate::renewal_twin::Personal;
use crate::renewal_twin::history::OfferBook;
use crate::renewal_twin::tests::{self as twin, Fixture, Rig, Seed, TestResult, token};
use crate::{AccountId, Amount, EconomyStore, InstrumentId};
use mush_core::{DefinitionDigest, GOD, ObjId, ObjKind, World};
use mush_org::PrincipalId;

impl Rig for Personal {
    const ROLES: usize = 6;
    const TITLES: usize = 4;

    #[inline(never)]
    fn seed() -> TestResult<Seed<Self>> {
        let mut world = World::new_minimal();
        world.set_now(1000);
        let buyer = world.create(ObjKind::Player, "Renewal buyer", GOD, None)?;
        let seller = world.create(ObjKind::Player, "Renewal seller", GOD, None)?;
        let charter = world.create(ObjKind::Thing, "Renewal charter", GOD, None)?;
        let instrument = InstrumentId::new("cr").ok_or("instrument")?;
        let registry =
            PrincipalId::System(mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?);
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
        // Zero cash is deliberate: constructing an unaccepted offer must not
        // reserve, debit, or require credit to stand in for escrow.
        for (id, owner) in [
            (0, PrincipalId::Object(buyer)),
            (1, PrincipalId::Object(seller)),
            (2, registry.clone()),
        ] {
            state.snapshot.put_account(Account {
                id: AccountId(id),
                owner,
                instrument: instrument.clone(),
                scope: None,
                name: "renewal fixture".into(),
                status: AccountStatus::Active,
                posted_balance: Amount::ZERO,
                held_amount: Amount::ZERO,
                credit: CreditPolicy::forbid(),
                last_journal_seq: 0,
                last_hold_seq: 0,
                revision: 1,
                approval: None,
            });
        }
        state.snapshot.put_asset(EconomicAsset {
            object: charter,
            title_owner: PrincipalId::Object(seller),
            scope: None,
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
                royalty_chain_digest: DefinitionDigest::from_bytes([5; 32]),
            },
            terms: RenewalOfferTermsV1 {
                format: 1,
                lineage: RenewalLineageV1::Original,
                buyer: buyer.into(),
                seller,
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
                parents: Vec::new(),
                remainder: Recipient {
                    account: AccountId(1),
                    owner: PrincipalId::Object(seller),
                },
                not_before: at.clone(),
                expires_at: mush_core::ClockStamp {
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

    /// Registry, venue, three royalty parents on a derived lineage, and the seller.
    fn widen(f: &mut Fixture<Self>) -> TestResult {
        f.offer.terms.lineage = RenewalLineageV1::Derived;
        for ordinal in 3..=6 {
            let owner = f.world.create(
                ObjKind::Player,
                &format!("Renewal recipient {ordinal}"),
                GOD,
                None,
            )?;
            let mut account = f
                .state
                .snapshot
                .account(AccountId(1))
                .ok_or("account")?
                .clone();
            account.id = AccountId(ordinal);
            account.owner = PrincipalId::Object(owner);
            f.state.snapshot.put_account(account);
            let recipient = Recipient {
                account: AccountId(ordinal),
                owner: PrincipalId::Object(owner),
            };
            if ordinal == 3 {
                f.offer.terms.venue = Some(GrossRecipientCut {
                    permille: 50,
                    recipient: Some(recipient),
                });
            } else {
                let charter =
                    f.world
                        .create(ObjKind::Thing, "Renewal parent charter", GOD, None)?;
                let mut title = f
                    .state
                    .snapshot
                    .asset(f.offer.terms.charter)
                    .ok_or("title")?
                    .clone();
                title.object = charter;
                title.title_owner = PrincipalId::Object(owner);
                f.state.snapshot.put_asset(title);
                f.offer.terms.parents.push(RoyaltyRecipientCut {
                    charter,
                    permille: 100,
                    title: Some(RoyaltyTitleRecipient {
                        title_revision: 1,
                        recipient,
                    }),
                });
            }
        }
        Ok(())
    }

    twin::views!();

    fn book_mut(state: &mut EconomyStore) -> &mut OfferBook<Self> {
        &mut state.renewal_offers
    }
    fn absent(state: &EconomyStore) -> Result<(), super::RenewalAuditError> {
        state.require_no_renewal_history()
    }
    const FOLD_KEY: &'static str = "renewal_receipts";
    /// The seller: a real character, and no party to the buyer's consent.
    fn stranger(f: &mut Fixture<Self>) -> TestResult<ObjId> {
        Ok(f.offer.terms.seller)
    }
    /// The seller leaves the game.
    fn lapse(f: &mut Fixture<Self>) -> TestResult {
        f.world
            .get_mut(f.offer.terms.seller)
            .ok_or("seller")?
            .flags
            .insert(mush_core::Flags::GOING);
        Ok(())
    }
    /// A derived lineage, with the royalty parents it then needs.
    fn other_line(f: &mut Fixture<Self>) -> TestResult {
        Self::widen(f)
    }
    /// The seller, and every royalty parent's charter, revision and recipient.
    fn widest(offer: &mut RenewalOffer) -> TestResult {
        offer.terms.seller = ObjId(u32::MAX - 1);
        offer.terms.remainder.owner = PrincipalId::Object(offer.terms.seller);
        for (n, cut) in offer.terms.parents.iter_mut().enumerate() {
            cut.charter = ObjId(u32::MAX - 3 - u32::try_from(n)?);
            let title = cut.title.as_mut().ok_or("title")?;
            title.title_revision = u64::MAX;
            title.recipient.account = AccountId(u64::MAX - 4 - u64::try_from(n)?);
            title.recipient.owner = PrincipalId::Object(ObjId(u32::MAX - 10 - u32::try_from(n)?));
        }
        Ok(())
    }
    /// A royalty parent must write its `title` out, and a fourth parent is one too many.
    fn own_wire_is_strict(offer: &RenewalOffer) -> TestResult {
        let mut marked = offer.clone();
        marked.terms.lineage = RenewalLineageV1::Derived;
        marked.terms.parents.push(RoyaltyRecipientCut {
            charter: ObjId(900),
            permille: 0,
            title: None,
        });
        let mut missing = serde_json::to_value(&marked)?;
        missing
            .get_mut("terms")
            .and_then(|v| v.get_mut("parents"))
            .and_then(|v| v.as_array_mut())
            .and_then(|v| v.first_mut())
            .and_then(|v| v.as_object_mut())
            .ok_or("parent")?
            .remove("title");
        check!(serde_json::from_value::<RenewalOffer>(missing).is_err());
        let mut too_many = serde_json::to_value(&marked)?;
        let rows = too_many
            .get_mut("terms")
            .and_then(|v| v.get_mut("parents"))
            .and_then(|v| v.as_array_mut())
            .ok_or("parents")?;
        let first = rows.first().cloned().ok_or("parent")?;
        rows.resize(4, first);
        check!(RenewalOffer::decode(&serde_json::to_vec(&too_many)?).is_err());
        Ok(())
    }
}

twin::instantiate!(crate::renewal_twin::Personal);

/// Every identity string of the issuer against a literal. The shared scenarios read the
/// issuer's constants, so a constant that moved would move the rig with it and they
/// would still pass; this second copy of the strings, and the CT.0 snapshot, would not.
#[test]
fn personal_identity_strings_hold_todays_bytes() {
    use crate::renewal_twin::Issuer;
    assert_eq!(Personal::DRAFT_ACTION, "draft_mind_renewal");
    assert_eq!(Personal::CLOSE_ACTION, "close_mind_renewal");
    assert_eq!(Personal::FUND_ACTION, "fund_mind_renewal");
    assert_eq!(Personal::GRANT_ACTION, "settle_mind_renewal");
    assert_eq!(Personal::REFUND_ACTION, "refund_mind_renewal");
    assert_eq!(Personal::SERVICE_CLOSE_ACTION, "close_mind_renewal_service");
    assert_eq!(Personal::NAMESPACE, "market.mind.renewal.v1");
    assert_eq!(Personal::OFFER_PREFIX, "renewal-offer:");
    assert_eq!(Personal::SERVICE_PREFIX, "renewal-service-close:");
    assert_eq!(Personal::PROTOCOL, "personal_mind_renewal_v1");
    assert_eq!(
        Personal::SERVICE_PROTOCOL,
        "personal_mind_renewal_service_v1"
    );
    assert_eq!(Personal::OFFER_DOMAIN, "economy.mind_renewal_offer.v1");
    assert_eq!(Personal::CONTRACT_NAME, "Mind renewal offer");
    assert_eq!(Personal::FUNDING_DESCRIPTION, "mind renewal funding");
    assert_eq!(
        Personal::BIRTH_ROUTE,
        crate::managed_paid_resale::PaidBirthRouteV1::Entitlement
    );
    // The names the rest of the crate and the other crates read are the same strings.
    assert_eq!(super::DRAFT_ACTION, Personal::DRAFT_ACTION);
    assert_eq!(super::CLOSE_ACTION, Personal::CLOSE_ACTION);
    assert_eq!(super::FUND_ACTION, Personal::FUND_ACTION);
    assert_eq!(super::GRANT_ACTION, Personal::GRANT_ACTION);
    assert_eq!(super::REFUND_ACTION, Personal::REFUND_ACTION);
    assert_eq!(super::SERVICE_CLOSE_ACTION, Personal::SERVICE_CLOSE_ACTION);
    assert_eq!(super::NAMESPACE, Personal::NAMESPACE);
    assert_eq!(super::service::PREFIX, Personal::SERVICE_PREFIX);
}

/// Personal only: the seller's title is a personal one. A title that sits on an
/// organisation's scope is refused even when it names the seller as its owner; a scoped title
/// is the House issuer's, whose terms say the scope.
#[test]
fn a_personal_offer_refuses_a_title_on_a_scope() -> TestResult {
    let mut f = Fixture::<Personal>::new()?;
    f.prepare()?;
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.scope = Some(mush_org::OrgScopeId(1));
    f.state.snapshot.put_asset(title);
    check!(f.prepare().is_err());
    Ok(())
}

/// Personal only: a personal renewal is filed under no scope. The House's is filed under
/// the issuing organisation's (`managed_house_renewal::tests`).
#[test]
fn a_personal_contract_is_filed_under_no_scope() -> TestResult {
    let mut f = Fixture::<Personal>::new()?;
    let (out, _) = f.publish()?;
    let contract = f
        .state
        .snapshot
        .contract(out.contract())
        .ok_or("contract")?;
    check_eq!(contract.consent.scope, None);
    Ok(())
}

#[path = "tests/ct0_snapshot.rs"]
mod ct0_snapshot;
