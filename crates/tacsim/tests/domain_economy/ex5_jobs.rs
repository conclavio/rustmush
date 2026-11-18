//! **Doc 48 cut EX.5 — the service arm and open jobs** (Q-48-3, -11, -12, -13, -58), over the
//! composed world (`driving`'s rig).
//!
//! - **The board.** `+market/jobs` at a venue lists its open jobs as typed kinds: a retrieval
//!   bound for Ceres, Occator's survey commissions at the programme port, and a traveller
//!   waiting at Stickney for passage. Each row names the verb that takes it.
//! - **The join.** A stranger takes a retrieval whose fee is above 10,000 cr, and the join
//!   escrows a bond of a fifth of the fee from their account; one who cannot cover it takes
//!   nothing.
//! - **For the org.** A member holding the org's spending capability takes a retrieval for the
//!   org, paid into its account; one without it is refused.
//!
//! **What is a fixture.** As RS.7's stories: a Co-op rig moved to the poster by
//! `@asset/transfer`, money by `@econ/mint`, the org and its grant by staff.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, EconomyStore};
use mush_market::{MarketId, ServiceKind, ServiceTerm};
use mush_org::PrincipalId;

use crate::driving_rig::{Rig, port_map, run, stocked_tuned};
use crate::field_host_rig::tune;
use tacsim::surface::retrieval;

/// The fee the stories post: above Q-48-13's 10,000 cr, so the job asks a bond.
const FEE_CR: i64 = 20_000;

fn venue(word: &str) -> MarketId {
    MarketId::new(word).expect("a venue")
}

fn account(rig: &Rig, who: &PrincipalId) -> AccountId {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(who)
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("a credit account")
}

fn held(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).map(|a| a.held_amount.minor()))
        .expect("the account")
}

fn mint(rig: &mut Rig, to: AccountId, minor: i64) {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", to.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

/// A poster with a Co-op rig of their own and the money for a retrieval of it; the posted job.
fn posted_job(rig: &mut Rig, who: &str, seed: i64, rig_index: usize) -> (ObjId, ObjId) {
    let (poster, _) = crate::driving_rig::prospector(rig, who, seed, &format!("{who} Moth"));
    let map = port_map(&rig.tw, "ceres_port");
    let hull = ObjId(map.vehicles.as_ref().expect("the rigs").rigs[rig_index].tv);
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, poster.0),
    );
    assert!(!moved.contains("Permission denied"), "{moved}");
    let wallet = account(rig, &PrincipalId::Object(poster));
    mint(rig, wallet, FEE_CR * 100);
    let out = run(
        &mut rig.tw,
        poster,
        &format!("+salvage/retrieve #{}=ceres fee={FEE_CR} go", hull.0),
    );
    assert!(out.contains("posted"), "{out}");
    (poster, hull)
}

/// **The board lists retrieval, passage and survey as typed kinds**, each with the verb that
/// takes it, and `+market/jobs` prints them within 120 columns.
#[test]
fn ex5_the_board_lists_retrieval_passage_and_survey_as_typed_kinds() {
    let mut rig = stocked_tuned(tune);
    let (_, hull) = posted_job(&mut rig, "Poster", 81, 0);
    let ceres = mush_market::services_at(rig.tw.world(), &venue("ceres"), None);
    let job = ceres
        .iter()
        .find(|s| s.kind == ServiceKind::Retrieval)
        .unwrap_or_else(|| panic!("a retrieval on Ceres's board: {ceres:?}"));
    let order = retrieval::order_of(rig.tw.world(), hull).expect("the job");
    assert_eq!(job.contract.map(|c| c.0), Some(order.contract));
    assert_eq!(job.pay.minor(), FEE_CR * 100);
    assert_eq!(
        job.bond.map(|b| b.minor()),
        Some(FEE_CR * 100 / 5),
        "a fifth of a fee above 10,000 cr"
    );
    assert_eq!(job.take.as_str(), format!("+salvage/accept #{}", hull.0));
    let survey = ceres
        .iter()
        .find(|s| s.kind == ServiceKind::Survey)
        .unwrap_or_else(|| panic!("Occator's commissions on Ceres's board: {ceres:?}"));
    assert!(
        matches!(survey.term, ServiceTerm::FromAcceptance { ic_secs } if ic_secs == 28 * 86_400),
        "a survey's term runs 28 IC days from acceptance: {survey:?}"
    );
    assert!(survey.take.starts_with("+survey/accept "), "{survey:?}");
    let stickney = mush_market::services_at(rig.tw.world(), &venue("stickney"), None);
    let passage = stickney
        .iter()
        .find(|s| s.kind == ServiceKind::Passage)
        .unwrap_or_else(|| panic!("a traveller on Stickney's board: {stickney:?}"));
    assert!(passage.negotiable, "a fare to open");
    assert!(passage.take.starts_with("+passage/offer #"), "{passage:?}");
    // A kind filter narrows it; the rendered board stays within 120 columns.
    let only = mush_market::services_at(rig.tw.world(), &venue("ceres"), Some(ServiceKind::Survey));
    assert!(only.iter().all(|s| s.kind == ServiceKind::Survey) && !only.is_empty());
    let viewer = crate::driving_rig::pilot(&mut rig, "Viewer", 82);
    let board = run(&mut rig.tw, viewer, "+market/jobs market=ceres");
    assert!(board.contains("Retrieval: bring "), "{board}");
    assert!(board.contains("Survey: fresh soundings of "), "{board}");
    assert!(
        board.contains(&format!("Take it: +salvage/accept #{}", hull.0)),
        "{board}"
    );
    for line in board.lines() {
        assert!(line.chars().count() <= 120, "over 120 columns: {line}");
    }
    let board = run(&mut rig.tw, viewer, "+market/jobs passage market=stickney");
    assert!(board.contains("Passage: carry "), "{board}");
    assert!(!board.contains("Retrieval:"), "{board}");
}

/// **A stranger joins a bonded job**: one who cannot cover the bond takes nothing, and the job
/// stays open; one who can has the bond escrowed from their own account at the join.
#[test]
fn ex5_a_stranger_takes_a_bonded_retrieval_and_the_join_escrows_the_bond() {
    let mut rig = stocked_tuned(tune);
    let (_, hull) = posted_job(&mut rig, "Poster", 83, 0);
    let (taker, _) = crate::driving_rig::prospector(&mut rig, "Taker", 84, "Taker Moth");
    let wallet = account(&rig, &PrincipalId::Object(taker));
    let preview = run(&mut rig.tw, taker, &format!("+salvage/accept #{}", hull.0));
    assert!(
        preview.contains("You escrow a bond of 4000.00cr"),
        "{preview}"
    );
    // Spend the taker's wallet down to nothing: the bond cannot be covered.
    let spare = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(wallet)
                .and_then(mush_economy::Account::available)
                .map(mush_economy::Amount::minor)
        })
        .expect("the wallet");
    // What the charter already holds stays held; the bond is measured past it.
    let held_before = held(&rig, wallet);
    if spare > 0 {
        let treasury = port_map(&rig.tw, "ceres_port")
            .treasury
            .expect("a treasury");
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@econ/pay {}={treasury} {spare}", wallet.0),
        );
        assert!(!out.contains("Usage"), "{out}");
    }
    let refused = run(
        &mut rig.tw,
        taker,
        &format!("+salvage/accept #{} go", hull.0),
    );
    assert!(refused.contains("could not be accepted"), "{refused}");
    let order = retrieval::order_of(rig.tw.world(), hull).expect("the job");
    assert!(order.acceptor.is_none(), "nobody took it");
    assert_eq!(held(&rig, wallet), held_before, "no bond held");
    // Funded, the join escrows the bond.
    mint(&mut rig, wallet, 500_000);
    let taken = run(
        &mut rig.tw,
        taker,
        &format!("+salvage/accept #{} go", hull.0),
    );
    assert!(taken.contains("You accept job"), "{taken}");
    assert!(taken.contains("You escrow a bond of 4000.00cr"), "{taken}");
    assert_eq!(
        held(&rig, wallet) - held_before,
        FEE_CR * 100 / 5,
        "the bond is escrowed"
    );
    let order = retrieval::order_of(rig.tw.world(), hull).expect("the job");
    assert_eq!(order.acceptor, Some(PrincipalId::Object(taker)));
    let c = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(order.contract))
                .cloned()
        })
        .expect("the contract");
    assert_eq!(
        mush_economy::job_shape(&c)
            .and_then(|j| j.bond)
            .map(|b| b.minor()),
        Some(FEE_CR * 100 / 5)
    );
    // Taken, it leaves the board.
    let ceres = mush_market::services_at(
        rig.tw.world(),
        &venue("ceres"),
        Some(ServiceKind::Retrieval),
    );
    assert!(
        ceres
            .iter()
            .all(|s| s.take != format!("+salvage/accept #{}", hull.0))
    );
}

/// **Org members with the capability accept for their org** (Q-48-58): a member holding Brask's
/// spending capability takes the job for Brask, paid into Brask's account and its bond escrowed
/// from it; a pilot who holds no grant is refused, and nothing is accepted.
#[test]
fn ex5_an_org_member_with_the_capability_takes_a_job_for_the_org() {
    let mut rig = stocked_tuned(tune);
    let (_, hull) = posted_job(&mut rig, "Poster", 85, 0);
    let officer = crate::driving_rig::pilot(&mut rig, "Officer", 86);
    let outsider = crate::driving_rig::pilot(&mut rig, "Outsider", 87);
    for line in [
        "@org/create Brask".to_owned(),
        format!("@org/grant #{}=Brask economy.account.post", officer.0),
        format!("@account/open org:Brask={}", tacsim::CREDIT),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    let brask = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == "Brask")
                .map(|org| PrincipalId::Organization(org.id))
        })
        .expect("Brask");
    let brask_account = account(&rig, &brask);
    mint(&mut rig, brask_account, 500_000);
    let refused = run(
        &mut rig.tw,
        outsider,
        &format!("+salvage/accept #{} as=Brask go", hull.0),
    );
    assert!(refused.contains(retrieval::NOT_FOR_ORG), "{refused}");
    assert!(retrieval::order_of(rig.tw.world(), hull).is_some_and(|o| o.acceptor.is_none()));
    let taken = run(
        &mut rig.tw,
        officer,
        &format!("+salvage/accept #{} as=Brask go", hull.0),
    );
    assert!(taken.contains("You accept job"), "{taken}");
    let order = retrieval::order_of(rig.tw.world(), hull).expect("the job");
    assert_eq!(order.acceptor, Some(brask.clone()), "Brask took it");
    assert_eq!(held(&rig, brask_account), FEE_CR * 100 / 5, "Brask's bond");
    let c = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(order.contract))
                .cloned()
        })
        .expect("the contract");
    assert_eq!(c.consent.terms.remainder_to, brask_account, "paid to Brask");
    // The officer holds the job for Brask: the retrieval custody follows the org.
    assert!(retrieval::held_by(rig.tw.world(), hull, officer).is_some());
}
