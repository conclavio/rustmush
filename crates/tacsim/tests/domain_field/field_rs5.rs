//! **Doc 41 cut RS.5 — the retrieval contract and the charge (story c, the charter half)**
//! (Q-41-8..14, Q-38-6), over the composed world (`driving`'s keyed rig), on both grounds of
//! `grounds_rig` (Ahuna Flank at 256, Urvara Reach at 512).
//!
//! 1. Captain A charters a Dray-P, leases a prospector, lands, launches it, and leaves it on the
//!    ground damaged (its power out, its chassis well past a tenth of the rig's points down);
//!    the Dray-P sails home to Ceres without it and A spends their money down.
//! 2. A week without a departure: the Co-op's dormancy recall takes the Dray-P back and
//!    repossesses the prospector **where it lies** — Occator's again, on the site — posting its
//!    retrieval at Tycho Mutual's quote; the deposit pays the restoration (less the wear
//!    allowance) and the fee as far as it goes, and the rest is a one-term debt.
//! 3. Player B reads the job on `+salvage/jobs`, accepts it, tows and loads the prospector into
//!    B's own bay as retrieval custody (no salvage row, no claim, no hand-back on the site), sails
//!    to Ceres, where the shed's hand-back runs at the docking and posts the delivery; the
//!    contract settles and B is paid the fee.
//! 4. A at Occator's counter: the debt owed, 28 IC days' grace; past it, delinquent, the desk refuses a lease naming the amount
//!    and reinstates it; `+debts` lists it, `+debts/pay` refuses short and pays once funded.
//!
//! The clerk's windows (Q-41-11) are a second, single-ground test: a job nobody accepts is
//! Tycho Mutual's after 28 IC days; a player who accepts and lapses after 28 more loses the job
//! to Tycho on a fresh contract.
//!
//! **What is a fixture.** The prospector's damage, by staff hand (GC.3's precedent), and its row
//! on the site's index, which the field that did it would have written (GC.5's fixture; a rig a
//! departure leaves idle is indexed by nothing yet, owed in RS.5's cut note); the world
//! rate pinned to 1 for the dormancy week, so a week of real time is a week of IC time (at the
//! rig's rate the 28-day write-off and the lease's term would fall inside it); staff money
//! after the debt. The windows test posts Occator's job on a shed rig by the module's own
//! `post` seam (the repossession's poster), since the windows, not the posting, are its subject.
//! Every other act is the production command path.

use mush_core::ObjId;
use mush_org::PrincipalId;
use tacsim::port::tv::TvLeaseStatus;
use tacsim::surface::retrieval::{RetrievalStage, order_of};

use crate::driving_rig::{
    Rig, command, file_and_launch, fire, land, launched_at, on_pad_as_at, port_map, prospector,
    ramp_at, run, set_rate, staff_hand, stocked_tuned, store, to_cell_on, to_ic,
};
use crate::field_host_rig::{second, set_section, site_record_at, tune};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};

/// Short tow stages (`field_gc5`'s `quick`).
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// The rig's power destroyed (`field_gc4`'s `powerless`): a casualty a tug can take.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs5-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Seconds on until `done` (`field_gc4`'s `seconds_until`).
fn seconds_until(rig: &mut Rig, limit: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let _ = second(rig);
    }
    done(rig)
}

/// The account `who` holds.
fn account(rig: &Rig, who: ObjId) -> mush_economy::AccountId {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .map(|a| a.id)
        .expect("an account")
}

/// An account's posted balance, minor.
fn balance(rig: &Rig, id: mush_economy::AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(id).map(|a| a.posted_balance.minor()))
        .expect("the account")
}

/// An account's available funds, minor.
fn available(rig: &Rig, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .and_then(|a| a.available())
        .map(|a| a.minor())
        .expect("available")
}

/// The contract's state.
fn contract_state(rig: &Rig, id: u64) -> mush_economy::ContractState {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(id))
                .map(|c| c.state)
        })
        .expect("the contract")
}

/// Staff money for `who` (`field_rs2`'s `mint`), major units.
fn mint(rig: &mut Rig, who: ObjId, major: u64) {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let to = account(rig, who).0;
    let _ = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/mint {treasury}={to} {major}"),
    );
}

/// Occator's account (the rig lessor's org row).
fn occator_account(rig: &Rig) -> mush_economy::AccountId {
    let map = port_map(&rig.tw, "ceres_port");
    map.orgs
        .iter()
        .find(|(id, _)| id.as_str() == "occator")
        .and_then(|(_, row)| row.account)
        .map(mush_economy::AccountId)
        .expect("Occator's account")
}

/// Occator's principal.
fn occator(rig: &Rig) -> PrincipalId {
    let map = port_map(&rig.tw, "ceres_port");
    map.orgs
        .iter()
        .find(|(id, _)| id.as_str() == "occator")
        .map(|(_, row)| PrincipalId::Organization(mush_org::OrgId(row.org)))
        .expect("Occator")
}

/// The tacsim debts `who` owes.
fn debts(rig: &Rig, who: ObjId) -> Vec<mush_economy::Obligation> {
    mush_economy::debts_of(
        rig.tw.world(),
        &PrincipalId::Object(who),
        &[tacsim::surface::retrieval::DEBT_NAMESPACE],
    )
}

/// **RS.5's story on `ground`.**
#[allow(clippy::too_many_lines)] // one story, in order
fn charter_on(ground: &Ground) {
    let mut rig = stocked_tuned(quick);
    // (1) A's party: the Dray-P on the site's pad, the prospector leased into its bay.
    let (a, row, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ground.site(), 1);
    let prospect = tvs[0];
    let hull = ObjId(row.hull);
    let lease = store(&rig.tw)
        .tv_lease_on(prospect)
        .cloned()
        .expect("the lease");
    launched_at(&mut rig, ground.entry, a, prospect);
    let (rx, ry, _) = ramp_at(&rig.tw, ground.entry);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell_on(&mut rig, ground.entry, prospect, (ramp.0 + 1, ramp.1));
    // The damage: its power out, and its chassis 80 of the rig's 450 points down.
    powerless(&mut rig, prospect);
    set_section(&mut rig, prospect, "chassis", Some("damaged"), Some(40));
    assert!(
        !tacsim::port::tv::wrecked(rig.tw.world(), prospect),
        "damaged, not a wreck"
    );
    // The field that did it would have put it on the site's index; staff do (GC.5's fixture). A
    // rig left idle by a departure is not indexed by anything yet: owed, RS.5's cut note.
    let s = crate::site_record_rig::staff_member(&mut rig);
    crate::site_record_rig::staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {} leave #{} casualty {},{}",
            ground.entry,
            prospect.0,
            ramp.0 + 1,
            ramp.1
        ),
    );
    // B's party lands on the same pad before A leaves (B's hull is not the one A's recall frees).
    let (b, b_row, b_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ground.site(), 1);
    let t = file_and_launch(&mut rig, a, hull, "+ship/undock ceres leave=1");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "ceres_port"
        ),
        "home at Ceres"
    );
    // A spends down to 100 cr.
    // `@econ/pay` takes the instrument's minor units.
    let spare = available(&rig, a) - 10_000;
    let sink = occator_account(&rig);
    if spare > 0 {
        let from = account(&rig, a).0;
        let paid = run(
            &mut rig.tw,
            a,
            &format!("@econ/pay {from}={} {spare}", sink.0),
        );
        assert!(
            available(&rig, a) <= 10_000,
            "spent down to {}: {paid}",
            available(&rig, a)
        );
    }

    // (2) A week without a departure, at rate 1: the dormancy recall repossesses the rig where it
    // lies.
    set_rate(&mut rig.tw, 1);
    let restoration = {
        let w = rig.tw.world();
        let port = tacsim::port::coop::programme_port(w).expect("the programme port");
        tacsim::port::tv::restoration_charge(w, &port, prospect).expect("a quote")
    };
    assert!(
        restoration.allowance_minor > 0 && restoration.total_minor > 0,
        "the wear allowance shows and the charge is real: {restoration:?}"
    );
    let t0 = rig.tw.world().now() + 8 * 86_400;
    rig.tw.set_now(t0);
    let mut heard = String::new();
    for _ in 0..20 {
        let events = rig.tw.fire_due(rig.tw.world().now().saturating_mul(1_000));
        heard.push_str(&crate::driving_rig::said(&rig.tw, &events, a));
        let queued = crate::driving_rig::drain(&mut rig.tw);
        crate::driving_rig::land_quietly(&mut rig, &queued);
        if store(&rig.tw)
            .tv_leases_on(prospect)
            .any(|r| r.lease == lease.lease && r.status == TvLeaseStatus::Returned)
        {
            break;
        }
        let (events, queued) = fire(&mut rig);
        heard.push_str(&crate::driving_rig::said(&rig.tw, &events, a));
        crate::driving_rig::land_quietly(&mut rig, &queued);
    }
    let ended = store(&rig.tw)
        .tv_leases_on(prospect)
        .find(|r| r.lease == lease.lease)
        .cloned()
        .expect("the record");
    assert_eq!(
        ended.status,
        TvLeaseStatus::Returned,
        "repossessed: {heard}"
    );
    assert!(ended.repossessed && !ended.written_off, "{ended:?}");
    assert!(
        mush_sim::pose_of(rig.tw.world(), prospect).is_some(),
        "the prospector stays where it lies"
    );
    let order = order_of(rig.tw.world(), prospect).expect("the retrieval");
    assert_eq!(order.stage, RetrievalStage::Open);
    assert_eq!(
        order.fee_minor, order.floor_minor,
        "posted at Tycho's quote"
    );
    assert_eq!(order.poster, occator(&rig));
    assert_eq!(order.destination.as_str(), "ceres_port");
    let hold = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .hold(mush_economy::HoldId(lease.hold))
                .map(|h| h.status.clone())
        })
        .expect("the deposit hold");
    assert!(
        matches!(hold, mush_economy::HoldStatus::Consumed { .. }),
        "the deposit is taken: {hold:?}"
    );
    let owed = debts(&rig, a);
    assert_eq!(owed.len(), 1, "one debt: {owed:?}");
    let debt = owed[0].clone();
    assert_eq!(
        debt.amount.minor(),
        restoration.total_minor + order.fee_minor - lease.deposit_minor,
        "the restoration and the fee beyond the deposit"
    );
    assert!(
        heard.contains("wear allowance") && heard.contains("as a debt (+debts)"),
        "A is told the allowance and the debt: {heard}"
    );

    // (3) B takes the job, tows and loads it into B's own bay, and brings it home.

    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    let jobs = run(&mut rig.tw, b, "+salvage/jobs");
    assert!(
        jobs.contains(&format!("#{}", prospect.0)) && jobs.contains("Ceres Freeport"),
        "{jobs}"
    );
    let taken = run(
        &mut rig.tw,
        b,
        &format!("+salvage/accept #{} go", prospect.0),
    );
    assert!(taken.contains("You accept job"), "{taken}");
    let order = order_of(rig.tw.world(), prospect).expect("the retrieval");
    assert_eq!(order.stage, RetrievalStage::Accepted);
    assert_eq!(order.acceptor, Some(PrincipalId::Object(b)));
    let b_acct = account(&rig, b);
    let b_before = balance(&rig, b_acct);
    launched_at(&mut rig, ground.entry, b, tug);
    to_cell_on(&mut rig, ground.entry, tug, (ramp.0 + 1, ramp.1));
    let (out, queued) = command(&mut rig, b, "+tv/1 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    land(&mut rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| site_record_at(rig, ground.entry)
            .and_then(|r| r.tows.towing(tug).map(|op| op.casualty()))
            == Some(prospect)),
        "the tow attaches"
    );
    let preview = run(&mut rig.tw, b, "+tv/1 load");
    assert!(
        preview.contains("You hold the") && preview.contains("retrieval"),
        "{preview}"
    );
    let (out, queued) = command(&mut rig, b, "+tv/1 load go");
    assert_eq!(queued.len(), 1, "one load start: {out}");
    land(&mut rig, &queued[0]).expect("the load start lands");
    assert!(
        seconds_until(&mut rig, 180, |rig| tacsim::surface::tow::in_bay_of(
            rig.tw.world(),
            prospect,
            b_carrier
        ) && site_record_at(rig, ground.entry)
            .is_some_and(|r| r.tows.act_of(tug).is_none())),
        "loaded into B's bay"
    );
    let record = site_record_at(&mut rig, ground.entry).expect("the site record");
    assert!(
        record.custody.iter().all(|row| row.hull != prospect),
        "retrieval custody, not salvage"
    );
    assert!(
        record.unlawful.iter().all(|act| act.subject != prospect.0),
        "no unlawful recovery"
    );
    let bay = run(&mut rig.tw, b, &format!("+bay #{}", prospect.0));
    assert!(bay.contains("Your right: retrieval"), "{bay}");
    let t = file_and_launch(&mut rig, b, b_carrier, "+ship/undock ceres leave=1");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        tacsim::port::tv::in_shed(rig.tw.world(), prospect),
        "the shed's hand-back at the docking"
    );
    let order = order_of(rig.tw.world(), prospect).expect("the retrieval");
    assert!(
        matches!(
            order.stage,
            RetrievalStage::Delivered | RetrievalStage::Closed
        ),
        "{order:?}"
    );
    assert!(
        order
            .delivered
            .as_ref()
            .is_some_and(|d| d.port.as_str() == "ceres_port"),
        "{order:?}"
    );
    for _ in 0..30 {
        if order_of(rig.tw.world(), prospect).is_some_and(|o| o.stage == RetrievalStage::Closed) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        crate::driving_rig::land_quietly(&mut rig, &queued);
    }
    let order = order_of(rig.tw.world(), prospect).expect("the retrieval");
    assert_eq!(order.stage, RetrievalStage::Closed, "{order:?}");
    assert!(
        matches!(
            contract_state(&rig, order.contract),
            mush_economy::ContractState::Settled { .. }
        ),
        "the contract settled"
    );
    assert_eq!(
        balance(&rig, b_acct) - b_before,
        order.fee_minor,
        "B is paid the fee"
    );

    // (4) The grace (the owner's 2026-11-04 rule: any wait on a player's response is at least 28
    //     IC days): the debt is owed but not delinquent until it is 28 IC days unpaid; past them,
    //     the desk refuses the delinquent lessee, and the debt is paid.
    let within = debts(&rig, a)
        .into_iter()
        .find(|o| o.id == debt.id)
        .expect("still owed");
    assert!(
        !matches!(within.status, mush_economy::ObligationStatus::Delinquent),
        "within the grace the debt is not delinquent: {within:?}"
    );
    skip_and_fire(&mut rig, within.schedule.next_due + 3_600);
    let debt_now = debts(&rig, a)
        .into_iter()
        .find(|o| o.id == debt.id)
        .expect("still owed");
    assert_eq!(
        debt_now.status,
        mush_economy::ObligationStatus::Delinquent,
        "{debt_now:?}"
    );
    crate::driving_rig::at_counter(&mut rig, a);
    // The refusal commits the debt's reinstatement, so its line must ride the commit: read as the
    // server releases a committing step's output (doc 41 cut RS.9c).
    let refused = crate::driving_rig::run_committed(&mut rig.tw, a, "+tv/lease go");
    assert!(
        refused.contains("delinquent debts") && refused.contains("will not deal with you"),
        "{refused}"
    );
    let reinstated = debts(&rig, a)
        .into_iter()
        .find(|o| o.id == debt.id)
        .expect("still owed");
    assert_eq!(
        reinstated.status,
        mush_economy::ObligationStatus::Active,
        "the desk reinstates it"
    );
    let listed = run(&mut rig.tw, a, "+debts");
    assert!(listed.contains(&format!("#{}", debt.id.0)), "{listed}");
    let short = run(&mut rig.tw, a, &format!("+debts/pay {}", debt.id.0));
    assert!(short.contains("cannot cover"), "{short}");
    mint(&mut rig, a, 100_000);
    let paid = run(&mut rig.tw, a, &format!("+debts/pay {}", debt.id.0));
    assert!(paid.contains("paid"), "{paid}");
    assert!(debts(&rig, a).is_empty(), "nothing owed");
}

/// **RS.5 at 256**: Ahuna Flank.
#[test]
fn rs5_ahuna_a_repossessed_rig_is_retrieved_by_a_player_who_is_paid_and_the_desk_refuses_the_debtor()
 {
    charter_on(&AHUNA_FLANK);
}

/// **RS.5 at 512**: Urvara Reach.
#[test]
fn rs5_urvara_a_repossessed_rig_is_retrieved_by_a_player_who_is_paid_and_the_desk_refuses_the_debtor()
 {
    charter_on(&URVARA_REACH);
}

/// Move the world on to IC `target` without firing (`field_gc5`'s `skip_to_ic`), then fire.
fn skip_and_fire(rig: &mut Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..3 {
        let (_, queued) = fire(rig);
        crate::driving_rig::land_quietly(rig, &queued);
    }
}

/// **The clerk's windows** (Q-41-11): an unaccepted job is Tycho's at 28 IC days on the same
/// contract; a player's lapse at 28 more hands the job to Tycho on a fresh one.
#[test]
#[allow(clippy::too_many_lines)]
fn rs5_the_clerk_hands_an_unaccepted_job_and_a_lapsed_one_to_tycho() {
    let mut rig = stocked_tuned(quick);
    let (c, _) = prospector(&mut rig, "Third", 3, "Grey Moth");
    let rigs: Vec<ObjId> = port_map(&rig.tw, "ceres_port")
        .vehicles
        .expect("Occator's rigs")
        .rigs
        .iter()
        .map(|r| ObjId(r.tv))
        .collect();
    let (one, two) = (rigs[0], rigs[1]);
    let poster = (occator(&rig), occator_account(&rig));
    let dest = mush_core::CanonicalId::new("ceres_port").expect("an id");
    for tv in [one, two] {
        let posted = staff_hand(&mut rig, |ctx| {
            tacsim::surface::retrieval::post(ctx, tv, poster.clone(), dest.clone(), None)
        });
        assert!(posted.is_ok(), "{posted:?}");
    }
    let house = tacsim::surface::retrieval::house(rig.tw.world()).expect("Tycho Mutual");
    let day = 86_400;
    let window = tacsim::surface::retrieval::RETRIEVAL_WINDOW_IC_DAYS * day;
    // A week on, C takes the second job; the first is nobody's.
    let posted_at = order_of(rig.tw.world(), one)
        .expect("the first job")
        .posted_ic;
    skip_and_fire(&mut rig, posted_at + 7 * day);
    let taken = run(&mut rig.tw, c, &format!("+salvage/accept #{} go", two.0));
    assert!(taken.contains("You accept job"), "{taken}");
    let first = order_of(rig.tw.world(), one).expect("the first job");
    let second_job = order_of(rig.tw.world(), two).expect("the second job");
    let perform_by = second_job.perform_by_ic.expect("C's window");
    // A day short of the acceptance window: still open.
    skip_and_fire(&mut rig, first.accept_by_ic - day);
    assert_eq!(
        order_of(rig.tw.world(), one).map(|o| o.stage),
        Some(RetrievalStage::Open)
    );
    // Past it: Tycho's, on the same contract.
    skip_and_fire(&mut rig, first.accept_by_ic + 60);
    let first_now = order_of(rig.tw.world(), one).expect("the first job");
    assert_eq!(first_now.stage, RetrievalStage::House, "{first_now:?}");
    assert_eq!(first_now.acceptor.as_ref(), Some(&house.org));
    assert_eq!(first_now.contract, first.contract, "the same contract");
    // C's job, still C's until C's own window ends.
    assert_eq!(
        order_of(rig.tw.world(), two).map(|o| o.stage),
        Some(RetrievalStage::Accepted)
    );
    assert!(perform_by > first.accept_by_ic, "C accepted after posting");
    // C lapses: the contract cancelled, a fresh one posted and taken by Tycho.
    skip_and_fire(&mut rig, perform_by + 60);
    let lapsed = order_of(rig.tw.world(), two).expect("the second job");
    assert_eq!(lapsed.stage, RetrievalStage::House, "{lapsed:?}");
    assert_eq!(lapsed.acceptor.as_ref(), Some(&house.org));
    assert_eq!(lapsed.serial, second_job.serial + 1, "a fresh contract");
    assert_ne!(lapsed.contract, second_job.contract);
    assert_eq!(
        contract_state(&rig, second_job.contract),
        mush_economy::ContractState::Cancelled,
        "the lapsed contract is cancelled"
    );
    let fresh = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(lapsed.contract))
                .cloned()
        })
        .expect("the fresh contract");
    assert_eq!(
        mush_economy::retrieval_acceptor(&fresh),
        Some(&house.org),
        "Tycho is the fresh contract's acceptor"
    );
    assert!(
        window > 0
            && fresh
                .consent
                .deadline
                .is_some_and(|d| d.at <= perform_by + 60 + window + 60),
        "the fresh contract runs one window"
    );
}
