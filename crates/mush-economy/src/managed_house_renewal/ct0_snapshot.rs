#![cfg(test)]

//! Doc 46 CT.0: the House renewal twin's snapshot.
//!
//! The House family has no other in-crate test rig, so this one mirrors the
//! personal `Rig` (`managed_renewal::tests`): the same fields, the same zero
//! balances, the same clock (1000) and offer window (100 IC seconds). What
//! differs is the subject: the charter's title belongs to an organisation on
//! one of its scopes, the remainder is paid to that organisation's treasury
//! there, and there is no lineage and no royalty parent. The organisation and
//! its scope are made by the real Org commands, so the Org owner the House
//! terms read is the registered one. The walk itself is shared with the
//! personal twin (`managed_renewal/tests/ct0_walk.rs`).
use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use crate::managed_house_renewal as twin;
use crate::record::{
    Account, AccountStatus, AssetLifecycle, CreditPolicy, EconomicAsset, InstrumentDef,
    InstrumentKind, InstrumentStatus, NegativePolicy,
};
use crate::{AccountId, Amount, EconomyModule, EconomyStore, InstrumentId};
use mush_core::{ClockStamp, DefinitionDigest, GOD, ObjKind, World};
use mush_org::{OrgId, OrgModule, OrgScopeId, OrgStore, PrincipalId};
use mush_testkit::TestWorld;
use twin::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PROTOCOL: &str = "house_mind_renewal_v1";

struct Rig {
    world: World,
    state: EconomyStore,
    offer: RenewalOffer,
    principal: mush_core::PrincipalId,
    organization: OrgId,
}

fn token(n: u64) -> mush_core::CommitToken {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&n.to_be_bytes());
    mush_core::CommitToken::from_bytes(bytes)
}
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

#[inline(never)]
fn rig() -> TestResult<Box<Rig>> {
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
    Ok(Box::new(Rig {
        world,
        state,
        offer,
        principal: mush_core::PrincipalId::Object { id: buyer },
        organization,
    }))
}
/// The House twin's every payee role: registry, venue and the House remainder.
fn widen(r: &mut Rig) -> TestResult {
    let owner = r
        .world
        .create(ObjKind::Player, "House renewal venue", GOD, None)?;
    let mut venue = r
        .state
        .snapshot
        .account(AccountId(0))
        .ok_or("account")?
        .clone();
    venue.id = AccountId(3);
    venue.owner = PrincipalId::Object(owner);
    r.state.snapshot.put_account(venue);
    r.offer.terms.venue = Some(GrossRecipientCut {
        permille: 50,
        recipient: Some(Recipient {
            account: AccountId(3),
            owner: PrincipalId::Object(owner),
        }),
    });
    check_eq!(
        r.offer.terms.remainder.owner,
        PrincipalId::Organization(r.organization)
    );
    Ok(())
}
fn book(state: &EconomyStore) -> &twin::history::OfferBook {
    &state.house_renewal_offers
}
fn book_mut(state: &mut EconomyStore) -> &mut twin::history::OfferBook {
    &mut state.house_renewal_offers
}
fn receipts(fold: &crate::fold::FoldState) -> &twin::receipt::RenewalReceiptAudit {
    &fold.house_renewal_receipts
}
fn receipts_mut(fold: &mut crate::fold::FoldState) -> &mut twin::receipt::RenewalReceiptAudit {
    &mut fold.house_renewal_receipts
}

/// The walk both twins share (one source, so not a second `mod` of one file).
mod walk {
    include!("../managed_renewal/tests/ct0_walk.rs");
}

#[test]
fn ct0_renewal_house_snapshot() -> TestResult {
    let snap = walk::run()?;
    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/renewal_house.snap")?;
    Ok(())
}
