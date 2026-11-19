//! **Doc 48 cut EX.6 — the escort** (Q-48-46, -58; doc 48a A-4, doc 48b W-48-3a), over the
//! composed world (`driving`'s keyed rig on Ahuna Flank), with the arena's egress pumped by hand
//! as `field_host` pumps it.
//!
//! Three parties at Ahuna Flank: a miner's two prospectors work the ground, an escort's armed hull
//! stands by them, and a raider's armed hull comes in from the west.
//!
//! 1. The miner posts an escort of each rig for its next sortie in the field; the escort takes
//!    both with their hull. Nothing is watched until a field opens.
//! 2. The raider fires: the site promotes and everyone on the ground is admitted. The host starts
//!    both watches.
//! 3. The raider's fire wrecks the second rig: that escort fails at once, whatever its presence.
//! 4. The miner brings the first rig in by the ramp: it leaves the field, and with the escort in
//!    the field the whole time that escort is kept. The fact holds, and the sweep pays the escort.

use mush_core::ObjId;
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::services::escort::{self, EscortStage};

use crate::driving_rig::{
    RAMP_CELL, Rig, act_go, command, finish, land, launched, on_pad, on_pad_as, port_map, run,
    stocked_tuned, to_cell, warm,
};
use crate::field_host_rig::{
    armed, egress, fragile, frame, left_on_hold, seat, second, task, tune, until, weapon_of,
};

fn account(rig: &Rig, who: ObjId) -> mush_economy::AccountId {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&PrincipalId::Object(who))
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("a credit account")
}

fn balance(rig: &Rig, who: ObjId) -> i64 {
    let id = account(rig, who);
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(id)
                .map(|a| a.posted_balance.minor() - a.held_amount.minor())
        })
        .expect("the account")
}

fn mint(rig: &mut Rig, who: ObjId, minor: i64) {
    let to = account(rig, who);
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/mint {treasury}={} {minor}", to.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

fn job_of(rig: &Rig, protected: ObjId) -> escort::EscortJob {
    escort::escorts(rig.tw.world())
        .into_iter()
        .find(|j| j.protected == protected)
        .expect("an escort of her")
}

fn state(rig: &Rig, contract: u64) -> ContractState {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract)).map(|c| c.state))
        .expect("the contract")
}

/// **On two workers, not the paused clock** (doc 43 TS.10a). The field closes before the miner's
/// `recover go` (the raider's side is out of it), so the journey ends with the field, and the
/// watch's last port tick (`services::escort::watch`) reads the escort out of a field that is
/// gone and counts that whole interval far. At five world seconds to the arena's one that
/// interval is under a tenth of the journey and the escort is kept (140 of 145 IC seconds near);
/// with world and arena on one clock, the production rate, it is five of 35 and the escort
/// fails. Whether the closing interval should count is a game question, owed to the owner;
/// until it is ruled the story keeps the clock it was written on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ex6_the_field_host_keeps_one_escort_and_fails_the_one_whose_rig_is_wrecked() {
    let mut rig = stocked_tuned(tune);
    // The miner's two rigs: the first by the ramp, the second east of the raider's hull.
    let (miner, _, rigs) = on_pad(&mut rig, 2);
    let (kept_rig, lost_rig) = (rigs[0], rigs[1]);
    launched(&mut rig, miner, kept_rig);
    to_cell(&mut rig, kept_rig, RAMP_CELL);
    let _ = act_go(&mut rig, miner, "+tv/2 launch go");
    let _ = finish(&mut rig, lost_rig, miner);
    to_cell(&mut rig, lost_rig, (114, 124));
    fragile(&mut rig, lost_rig);
    // Nobody sits in them: left on HOLD, they stand where they were left in a field.
    left_on_hold(&mut rig, miner, 1);
    left_on_hold(&mut rig, miner, 2);
    // The escort's armed hull between them, and the raider's to the west.
    let (guard, _, _) = on_pad_as(&mut rig, "Escort", 2, "Tally Moth", 1);
    let escort_hull = armed(&mut rig, "Grey Hound", (122, 120), 270_000);
    seat(&mut rig, escort_hull, guard);
    let (raider, _, _) = on_pad_as(&mut rig, "Raider", 3, "Lone Moth", 1);
    let raider_hull = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    seat(&mut rig, raider_hull, raider);
    mint(&mut rig, miner, 1_000_000);

    // ---- (1) The posts and the acceptances ----
    for tv in [kept_rig, lost_rig] {
        let out = run(
            &mut rig.tw,
            miner,
            &format!("+escort/post #{} field=ahuna_port fee=300 go", tv.0),
        );
        assert!(out.contains("posted: "), "{out}");
    }
    let again = run(
        &mut rig.tw,
        miner,
        &format!("+escort/post #{} field=ahuna_port fee=300 go", kept_rig.0),
    );
    assert!(
        again.contains("is open already"),
        "one escort of her at a time: {again}"
    );
    let kept = job_of(&rig, kept_rig);
    let lost = job_of(&rig, lost_rig);
    let not_theirs = run(
        &mut rig.tw,
        guard,
        &format!(
            "+escort/accept {} with=#{} go",
            kept.contract, raider_hull.0
        ),
    );
    assert!(
        not_theirs.contains("You hold no title to that hull"),
        "the escort escorts with a hull of their own: {not_theirs}"
    );
    for job in [&kept, &lost] {
        let out = run(
            &mut rig.tw,
            guard,
            &format!("+escort/accept {} with=#{} go", job.contract, escort_hull.0),
        );
        assert!(out.contains("You take escort"), "{out}");
    }
    let _ = second(&mut rig);
    assert_eq!(
        job_of(&rig, kept_rig).stage,
        EscortStage::Taken,
        "no field, no watch"
    );

    // ---- (2) The raider fires: the field opens, and the host watches ----
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, raider, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some()
            && [kept_rig, lost_rig, escort_hull, raider_hull].iter().all(
                |u| tacsim::field::field_of_unit(rig.tw.world(), *u).is_some()
            ))
        .await,
        "the field opens with all three parties in it"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| [
            kept_rig, lost_rig
        ]
        .iter()
        .all(|tv| job_of(rig, *tv).stage == EscortStage::Watching))
        .await,
        "the host starts both watches: {:?} / {:?} / {:?}",
        job_of(&rig, kept_rig),
        tacsim::field::field_of_unit(rig.tw.world(), kept_rig),
        state(&rig, kept.contract)
    );

    // ---- (3) The raider wrecks the second rig: its escort fails ----
    let gun = weapon_of(&rig, raider_hull);
    let wrecked = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        if let Some(contact) = frame(rig, raider).and_then(|f| {
            f.contacts
                .iter()
                .find(|c| {
                    c.cell
                        .is_some_and(|(x, y)| x.abs_diff(114) <= 2 && y.abs_diff(124) <= 2)
                })
                .cloned()
        }) && frame(rig, raider).is_some_and(|f| f.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, raider, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, raider, &format!("FIRE {gun}"));
        tacsim::port::tv::wrecked(rig.tw.world(), lost_rig)
    })
    .await;
    assert!(wrecked, "the raider's fire wrecks the second rig");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| job_of(
            rig, lost_rig
        )
        .stage
            == EscortStage::Failed)
        .await,
        "a wrecked rig fails her escort"
    );
    assert_eq!(
        job_of(&rig, lost_rig).why.as_deref(),
        Some(escort::WHY_WRECKED)
    );

    // ---- (4) The miner brings the first rig in: its escort is kept ----
    let asked = run(&mut rig.tw, miner, "+tv/1 recover go");
    assert!(
        asked.contains("ramp") && asked.contains("Queued."),
        "{asked}"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| job_of(
            rig, kept_rig
        )
        .stage
            != EscortStage::Watching)
        .await,
        "the host ends the watch when she leaves the field"
    );
    let job = job_of(&rig, kept_rig);
    assert_eq!(job.stage, EscortStage::Kept, "{job:?}");
    let watch = job.watch.clone().expect("the watch");
    assert!(
        watch.journey_ic > 0 && watch.permille() >= escort::NEAR_PERMILLE,
        "near her for the field: {watch:?}"
    );
    let before = balance(&rig, guard);
    for _ in 0..6 {
        crate::driving_rig::sweep(&mut rig.tw, 60);
        let _ = second(&mut rig);
    }
    assert!(
        matches!(state(&rig, job.contract), ContractState::Settled { .. }),
        "the host's fact settles the job: {:?}",
        state(&rig, job.contract)
    );
    assert_eq!(balance(&rig, guard) - before, 30_000, "the escort is paid");
    assert!(
        state(&rig, lost.contract).is_live(),
        "the failed escort's contract waits for its term, which forfeits it"
    );
}
