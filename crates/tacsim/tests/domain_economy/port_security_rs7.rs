//! **Doc 41 cut RS.7 — port security (story b, extended)** (Q-41-24, Q-41-25, Q-38-20, doc 27
//! Q-S4), over the composed world (`driving`'s keyed rig) on Urvara Reach (512) and on Ahuna
//! Flank (256), with three parties:
//!
//! - **the owner** (T), whose carrier stands on the pad and whose two rigs `a` and `d` lie
//!   beside it on L's blocks;
//! - **the opportunist** (O), whose rig tows both into O's carrier with nobody's authority;
//! - **the landholder** (L), who holds the Registry's claims on those blocks.
//!
//! O files on `a`; T's `recovery` contest on T's own sensor record vacates it, which proves the
//! recovery and copies it to O's port-side record. `d` is left as bare evidence. O sails home to
//! Ceres with too little money: at the docking the Registry fines O 5,000 cr for `a` and nothing
//! for `d`, the fine stands as a debt, and O's carrier is held — `+ship/undock` refuses with
//! the quarantine's line — until O pays it with `+debts/pay`. T then reports `d`'s recovery to
//! the registry (`+salvage/report <site>=#<d> registry`), which proves it for O's next docking;
//! L, who saw nothing, cannot.
//!
//! **What is a fixture.** As RS.3's story: the rigs' power is destroyed and they are put on the
//! site's index by staff hand, `a` and `d` moved to T by `@asset/transfer`, and L's block claims
//! seeded as typed records; O's money spent down by `@econ/pay` and funded by `@econ/mint`.

use mush_core::{GOD, ObjId};
use mush_economy::EconomyStore;
use mush_org::PrincipalId;
use tacsim::surface::claim::UnlawfulKind;
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::security::Proof;
use tacsim::surface::site_record::SiteRecord;

use crate::driving_rig::{
    Rig, cid, command, file_and_launch, launched_at, on_pad_as_at, pilot, port_map, ramp_at, run,
    seed_typed, staff_hand, stocked_tuned, to_cell_on, to_ic,
};
use crate::field_host_rig::{second, site_record_at, tune};
use crate::site_record_rig::{staff, staff_member};

/// The compressed contest window, real seconds.
const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute contest window, on the field host's lab tempo.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_ic_s = WINDOW;
}

fn record(rig: &mut Rig, entry: &str) -> SiteRecord {
    site_record_at(rig, entry).expect("the site's record")
}

fn custody(rig: &mut Rig, entry: &str, hull: ObjId) -> Option<SalvageCustody> {
    record(rig, entry)
        .custody
        .into_iter()
        .find(|row| row.hull == hull)
}

/// `line` as `who`, every submission it queued landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert!(!queued.is_empty(), "{line}: a submission: {out}");
    for submit in &queued {
        crate::driving_rig::land(rig, submit).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    out
}

/// Seconds until `done` holds, at most `limit`, every tow landing accepted.
fn seconds_until(rig: &mut Rig, limit: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let errors: Vec<String> = second(rig)
            .into_iter()
            .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
            .collect();
        assert!(errors.is_empty(), "a tow landing refused: {errors:?}");
    }
    done(rig)
}

/// **Destroy `unit`'s power component** (GC.4's fixture): a casualty.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs7-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The block letter of `cell` on a site `side` cells a side.
fn letter_of(side: u16, cell: (i64, i64)) -> char {
    tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: cell.0 * 1_000 + 500,
            y_cm: cell.1 * 1_000 + 500,
        },
        tacsim::surface::address::Scheme::of_site(side),
    )
    .expect("a block")
}

/// L's claim on `letter` of `entry`, seeded (GC.6's fixture).
fn seed_block(rig: &mut Rig, entry: &str, letter: char, l: ObjId) {
    let place = port_map(&rig.tw, entry)
        .site
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(entry), letter, place);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: PrincipalId::Object(l),
        account: rig
            .tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(l)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * 86_400,
        worked_at_term_start: 0,
        discovered_ic: None,
        declared: None,
        fee: None,
        refund: None,
        lapsed_ic: None,
    });
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    seed_typed(
        rig,
        &handles.families.claims,
        &tacsim::surface::claims::BlockKey {
            site: cid(entry),
            letter,
        },
        tenure,
    );
}

/// O's rig tows `casualty` from beside it and loads it into O's carrier at the ramp.
fn recover(rig: &mut Rig, entry: &str, (o, tug): (ObjId, ObjId), casualty: ObjId, at: (i64, i64)) {
    to_cell_on(rig, entry, tug, (at.0 + 1, at.1));
    let _ = second(rig);
    let out = go(rig, o, &format!("+tv/1 tow #{} go", casualty.0));
    assert!(
        seconds_until(rig, 120, |rig| record(rig, entry)
            .tows
            .towing(tug)
            .map(|op| op.casualty())
            == Some(casualty)),
        "the tow attaches: {out}"
    );
    let ramp = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    to_cell_on(rig, entry, tug, ramp);
    let _ = second(rig);
    let out = go(rig, o, "+tv/1 load go");
    assert!(
        seconds_until(rig, 120, |rig| custody(rig, entry, casualty).is_some()
            && record(rig, entry).tows.act_of(tug).is_none()),
        "the load lands in custody: {out}"
    );
}

/// The account `who` holds.
fn account(rig: &Rig, who: ObjId) -> mush_economy::AccountId {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .map(|a| a.id)
        .expect("an account")
}

/// An account's available funds, minor.
fn available(rig: &Rig, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .and_then(|a| a.available())
        .map(|a| a.minor())
        .expect("available")
}

/// The Freeport Registry's account.
fn registry_account(rig: &Rig) -> mush_economy::AccountId {
    let map = port_map(&rig.tw, "ceres_port");
    map.orgs
        .iter()
        .find(|(id, _)| id.as_str() == "freeport_registry")
        .and_then(|(_, row)| row.account)
        .map(mush_economy::AccountId)
        .expect("the Registry's account")
}

/// The tacsim debts `who` owes.
fn debts(rig: &Rig, who: ObjId) -> Vec<mush_economy::Obligation> {
    mush_economy::debts_of(
        rig.tw.world(),
        &PrincipalId::Object(who),
        &[tacsim::surface::retrieval::DEBT_NAMESPACE],
    )
}

/// The obligation `id`'s status.
/// Move IC on to `target` without firing, then fire a few times (the obligation sweep runs).
fn skip_ic(rig: &mut Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..3 {
        let (_, queued) = crate::driving_rig::fire(rig);
        crate::driving_rig::land_quietly(rig, &queued);
    }
}

fn status(rig: &Rig, id: u64) -> Option<mush_economy::ObligationStatus> {
    rig.tw.world().module_state::<EconomyStore>().and_then(|e| {
        e.snapshot()
            .obligation(mush_economy::ObligationId(id))
            .map(|o| o.status.clone())
    })
}

/// **The story at one site**: `(venue, entry, side)`.
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str, side: u16) {
    let mut rig = stocked_tuned(quick);
    let (t, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
    let (o, o_row, o_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), (venue, entry), 1);
    let o_hull = ObjId(o_row.hull);
    let tug = o_tvs[0];
    let l = pilot(&mut rig, "Lander", 3);
    // Two spare rigs of Occator's pool, titled to T.
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tug)
        .expect("the tug's unit")
        .schema()
        .clone();
    let spares: Vec<ObjId> = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tug && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .filter(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .take(2)
        .collect();
    let [a, d] = spares.as_slice() else {
        panic!("two spare rigs in Occator's pool: {spares:?}");
    };
    let (a, d) = (*a, *d);
    for hull in [a, d] {
        let moved = run(
            &mut rig.tw,
            GOD,
            &format!("@asset/transfer #{}=#{}", hull.0, t.0),
        );
        assert!(!moved.contains("Permission denied"), "{moved}");
    }

    // The ground: both beside the pad, in sight of T's carrier, on L's blocks.
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let (a_cell, d_cell) = ((rx + 3, ry - 2), (rx - 3, ry - 2));
    let s = staff_member(&mut rig);
    for (hull, (x, y)) in [(a, a_cell), (d, d_cell)] {
        to_cell_on(&mut rig, entry, hull, (x, y));
        powerless(&mut rig, hull);
        staff(
            &mut rig,
            s,
            &format!("@tacsim/site {entry} leave #{} casualty {x},{y}", hull.0),
        );
        seed_block(&mut rig, entry, letter_of(side, (x, y)), l);
    }

    // 1. O recovers both with nobody's authority: two bare `Recovery` rows, evidence only.
    launched_at(&mut rig, entry, o, tug);
    recover(&mut rig, entry, (o, tug), a, a_cell);
    recover(&mut rig, entry, (o, tug), d, d_cell);
    let unlawful: Vec<u32> = record(&mut rig, entry)
        .unlawful
        .iter()
        .filter(|act| act.kind == UnlawfulKind::Recovery && act.actor == o)
        .map(|act| act.subject)
        .collect();
    assert_eq!(unlawful, vec![a.0, d.0]);
    assert!(
        tacsim::surface::security::record_of(rig.tw.world(), o).is_none(),
        "a bare row is evidence, never a proof"
    );

    // 2. O files on `a`; T's sensor record vacates it, and the vacate proves the recovery.
    let out = go(&mut rig, o, &format!("+salvage/claim #{} go", a.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    assert!(matches!(
        custody(&mut rig, entry, a).map(|row| row.claim),
        Some(ClaimSeam::Filed { .. })
    ));
    let _ = go(
        &mut rig,
        t,
        &format!("+salvage/contest #{}=recovery go", a.0),
    );
    assert!(
        seconds_until(&mut rig, 10, |rig| custody(rig, entry, a).is_none()),
        "the vacating contest settles"
    );
    let proven = tacsim::surface::security::record_of(rig.tw.world(), o).expect("O's record");
    assert_eq!(proven.rows.len(), 1, "{proven:?}");
    assert_eq!(
        (
            proven.rows[0].act.kind,
            proven.rows[0].act.subject,
            &proven.rows[0].proof,
            proven.rows[0].debt
        ),
        (UnlawfulKind::Recovery, a.0, &Proof::Vacated, None)
    );

    // 3. O spends down to 100 cr and sails home to Ceres, the tug left on the site.
    let spare = available(&rig, o) - 10_000;
    if spare > 0 {
        let from = account(&rig, o).0;
        let sink = registry_account(&rig).0;
        let _ = run(&mut rig.tw, o, &format!("@econ/pay {from}={sink} {spare}"));
    }
    let before = available(&rig, o);
    let registry_before = {
        let id = registry_account(&rig);
        rig.tw
            .world()
            .module_state::<EconomyStore>()
            .and_then(|e| e.snapshot().account(id).map(|a| a.posted_balance.minor()))
            .expect("the Registry's balance")
    };
    let ticket = file_and_launch(&mut rig, o, o_hull, "+ship/undock ceres leave=1");
    let heard = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let heard = crate::driving_rig::said(&rig.tw, &heard, o);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), o_hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "ceres_port"
        ),
        "home at Ceres"
    );

    // 4. The fine at docking: 5,000 cr for `a`, nothing for `d`, a debt O cannot cover.
    assert!(
        heard.contains("fines you") && heard.contains("an unlawful recovery"),
        "O is told: {heard}"
    );
    let owed = debts(&rig, o);
    assert_eq!(owed.len(), 1, "one fine, `d` is bare: {owed:?}");
    let fine = owed[0].clone();
    assert_eq!(fine.credit_account, registry_account(&rig));
    assert_eq!(
        fine.amount.minor(),
        i64::try_from(tacsim::surface::security::FINE_RECOVERY_MAJOR).expect("fits") * 100,
        "5,000 cr"
    );
    assert_eq!(available(&rig, o), before, "nothing was taken short");
    let fined = tacsim::surface::security::record_of(rig.tw.world(), o).expect("O's record");
    assert_eq!(fined.rows[0].debt, Some(fine.id.0), "the receipt");

    // 5. The grace (the owner's 2026-11-04 rule: any wait on a player's response is at least 28
    //    IC days): the fine is owed and listed, but O's carrier is not held until it is past its
    //    28 IC days unpaid.
    assert!(
        !tacsim::surface::security::holds(rig.tw.world(), o_hull),
        "within the grace the carrier may undock"
    );
    assert!(
        !matches!(
            status(&rig, fine.id.0),
            Some(mush_economy::ObligationStatus::Delinquent)
        ),
        "within the grace the fine is not delinquent"
    );
    let due_at =
        rig.tw.world().world_ic_now().at + tacsim::surface::retrieval::DEBT_GRACE_IC_DAYS * 86_400;
    skip_ic(&mut rig, due_at + 3_600);
    assert_eq!(
        status(&rig, fine.id.0),
        Some(mush_economy::ObligationStatus::Delinquent),
        "past the grace, unpaid, it is delinquent"
    );
    // The quarantine: O's carrier does not lift while the delinquent fine stands.
    let (refused, queued) = command(&mut rig, o, "+ship/undock urvara");
    assert!(
        refused.contains("is held at") && refused.contains("lifts when it is paid"),
        "{refused}"
    );
    assert!(queued.is_empty(), "nothing filed: {refused}");
    let listed = run(&mut rig.tw, o, "+debts");
    assert!(listed.contains(&format!("#{}", fine.id.0)), "{listed}");
    let short = run(&mut rig.tw, o, &format!("+debts/pay {}", fine.id.0));
    assert!(short.contains("cannot cover"), "{short}");

    // 6. Funded, O pays; the hold lifts and the row is settled.
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let to = account(&rig, o).0;
    let minted = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={to} 600000"),
    );
    let paid = run(&mut rig.tw, o, &format!("+debts/pay {}", fine.id.0));
    assert!(
        paid.contains(&format!("Debt #{} paid", fine.id.0)),
        "{paid} (minted: {minted}; available {})",
        available(&rig, o)
    );
    assert_eq!(
        status(&rig, fine.id.0),
        Some(mush_economy::ObligationStatus::Satisfied)
    );
    let registry_after = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(registry_account(&rig))
                .map(|a| a.posted_balance.minor())
        })
        .expect("the Registry's balance");
    assert_eq!(registry_after - registry_before, fine.amount.minor());
    let (cleared, _) = command(&mut rig, o, "+ship/undock urvara");
    assert!(
        !cleared.contains("is held at"),
        "the hold lifted: {cleared}"
    );

    // 7. T reports `d` to the registry on T's sensor record; L, who saw nothing, cannot.
    let site_word = venue;
    let refused = run(
        &mut rig.tw,
        l,
        &format!("+salvage/report {site_word}=#{} registry", d.0),
    );
    assert!(refused.contains("no record of your own"), "{refused}");
    let reported = run(
        &mut rig.tw,
        t,
        &format!("+salvage/report {site_word}=#{} registry", d.0),
    );
    assert!(
        reported.contains("takes your report of one act") && reported.contains("Second"),
        "{reported}"
    );
    let again = run(
        &mut rig.tw,
        t,
        &format!("+salvage/report {site_word}=#{} registry", d.0),
    );
    assert!(again.contains("already has that"), "{again}");
    let record = tacsim::surface::security::record_of(rig.tw.world(), o).expect("O's record");
    let row = record
        .rows
        .iter()
        .find(|row| row.act.subject == d.0)
        .expect("`d` is proven");
    assert_eq!((&row.proof, row.debt), (&Proof::Reported { by: t }, None));
}

/// **RS.7's gate on Urvara Reach (512)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs7_story_b_port_security_on_urvara_reach() {
    story("urvara", "urvara_port", 512);
}

/// **RS.7's gate on Ahuna Flank (256)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs7_story_b_port_security_on_ahuna_flank() {
    story("ahuna", "ahuna_port", 256);
}
