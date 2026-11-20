//! **Doc 50 unit LW.2 — the record of proven offenses, jurisdiction by control, and who answers**
//! (Q-50-37, Q-50-38, Q-50-68; doc 50a A-10b, A-10c), over the composed lane (`driving`'s stocked
//! rig), through registered commands and the port tick.
//!
//! A House, Meridiani, has a Dray docked at Ceres Freeport and a pilot who flies her. The Freeport
//! has proven an unlawful recovery against Meridiani itself (an act made `as=<org>`).
//!
//! 1. **Clean at the Republic's port.** She sails to Stickney Yards, which the Republic holds de
//!    facto: the Freeport's offense does not bind there, and nothing is fined.
//! 2. **An organization's hull answers through its organization** (A-10b). She sails home to
//!    Ceres, which the Freeport holds: the offense is fined to the port's authority, the Freeport
//!    Registry, on Meridiani's account. Before the unit her docking fined nobody.
//! 3. **The quarantine holds the hull that docked, at the proving polity's ports only** (A-10c).
//!    She sails back to Stickney inside the grace and the grace runs out there: she lifts from
//!    the Republic's port. Before the unit a past-due fine held her at every port and site.
//! 4. **Wanted at Ceres.** Home at Ceres the fine holds her: `+ship/undock` refuses with the
//!    quarantine's line.
//! 5. **Jurisdiction follows control.** Staff hand Ceres station to the Republic de facto
//!    (`@faction/control`): the Freeport's offense no longer binds there, and she may lift.
//!
//! **What is a fixture.** The House, its account and its hull as doc 47 cut CO.9's story seeds
//! them; the stores in her hold; the proven offense against Meridiani, seeded as a stored record
//! the boot reads back, because no salvage verb yet takes `as=<org>`.

use mush_core::{GOD, ObjId};
use mush_org::PrincipalId;
use tacsim::offense::{Evidence, Offender, OffenseKey, OffenseKind, OffenseRecord, Proof};
use tacsim::port::commission::{Step, YardCommission};

use crate::driving_rig::{
    Rig, cid, command, file_and_launch, fire, land_quietly, pilot, reboot_seeding, run, seed_pool,
    station_room, stocked, teleport, to_ic,
};

const MARK: &str = "MER-LW2";

fn org_named(rig: &Rig, name: &str) -> mush_org::OrgId {
    rig.tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the org store")
        .state()
        .orgs()
        .find(|org| org.name.as_str() == name)
        .map(|org| org.id)
        .unwrap_or_else(|| panic!("no organization {name}"))
}

fn seeded(rig: &Rig) -> Option<YardCommission> {
    tacsim::port::commission::commissions(rig.tw.world())
        .into_iter()
        .find(|record| record.mark == MARK)
}

/// The port `hull` is docked at, by entry.
fn docked_at(rig: &Rig, hull: ObjId) -> Option<String> {
    match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
        tacsim::port::ship::Berth::Docked { map, .. } => Some(map.entry.to_string()),
        _ => None,
    }
}

/// `flyer` takes `hull` to `dest`, filled for the leg, and is docked at `entry` after.
fn sail(
    rig: &mut Rig,
    (flyer, hull, hold): (ObjId, ObjId, mush_economy::VaultId),
    dest: &str,
    entry: &str,
) {
    seed_pool(
        rig,
        hold,
        "propellant",
        60,
        mush_inventory::LotId(next_lot()),
    );
    seed_pool(
        rig,
        hold,
        "provisions",
        40,
        mush_inventory::LotId(next_lot()),
    );
    let ticket = file_and_launch(rig, flyer, hull, &format!("+ship/undock {dest}"));
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s + 3_600);
    assert_eq!(
        docked_at(rig, hull).as_deref(),
        Some(entry),
        "docked at {entry}"
    );
}

/// A fresh fixture lot id.
fn next_lot() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x50_2000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Meridiani's port-security debts.
fn debts(rig: &Rig, org: mush_org::OrgId) -> Vec<mush_economy::Obligation> {
    mush_economy::debts_of(
        rig.tw.world(),
        &PrincipalId::Organization(org),
        &[tacsim::surface::retrieval::DEBT_NAMESPACE],
    )
}

/// Every due fire up to `target` IC, landing what each queues.
fn skip_ic(rig: &mut Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..3 {
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn lw2_an_offense_binds_only_where_its_polity_holds_and_the_house_answers_for_its_hull() {
    let mut rig = stocked();

    // ---- The House, its account, its hull and its pilot (CO.9's seeds) ----
    for line in [
        "@org/create Meridiani".to_owned(),
        format!("@account/open org:Meridiani={}", tacsim::CREDIT),
        format!("@tacsim/port seed ceres_port dray {MARK}=org:Meridiani"),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
    }
    let meridiani = org_named(&rig, "Meridiani");
    for _ in 0..30 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if seeded(&rig).is_some_and(|record| record.step == Step::Done) {
            break;
        }
    }
    let record = seeded(&rig).expect("the seed's commission");
    assert_eq!(record.step, Step::Done, "laid up: {record:?}");
    let dray = record.object().expect("the Dray");
    let hold = mush_economy::VaultId(record.hold.expect("her hold"));
    let flyer = pilot(&mut rig, "Housepilot", 2);
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=Meridiani unit.deploy", flyer.0),
    );
    assert!(granted.contains("Granted "), "{granted}");
    assert_eq!(
        tacsim::offense::answerer_for_hull(rig.tw.world(), dray),
        Some(Offender::Organization(meridiani)),
        "an organization's hull answers through its organization"
    );

    // ---- The Freeport's proven offense against Meridiani (a stored record, read at boot) ----
    let freeport = tacsim::polity::polity(rig.tw.world(), "freeport").expect("the Freeport");
    let republic = tacsim::polity::polity(rig.tw.world(), "republic").expect("the Republic");
    assert_eq!(
        tacsim::surface::security::proving_polity(rig.tw.world()),
        Some(freeport),
        "the Freeport Registry's polity proves a salvage offense"
    );
    let key = OffenseKey {
        offender: Offender::Organization(meridiani),
        polity: freeport,
    };
    let mut offense = OffenseRecord::new(key);
    offense.rows.push(tacsim::offense::ProvenOffense {
        kind: OffenseKind::SalvageRecovery,
        evidence: Evidence::Site {
            site: cid("urvara_port"),
            act: tacsim::surface::claim::UnlawfulAct {
                kind: tacsim::surface::claim::UnlawfulKind::Recovery,
                actor: flyer,
                subject: 4_242,
                landholder: None,
                at_ic: rig.tw.world().world_ic_now().at,
            },
        },
        proof: Proof::Vacated,
        proven_ic: rig.tw.world().world_ic_now().at,
        debt: None,
        payout: None,
    });
    let mut rig = reboot_seeding(
        rig,
        vec![(
            key.doc_key(),
            serde_json::to_value(&offense).expect("the record encodes"),
        )],
        &[flyer],
    );
    assert_eq!(
        tacsim::offense::record_of(rig.tw.world(), &key).map(|r| r.rows.len()),
        Some(1),
        "the boot reads the record back"
    );
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, flyer, gallery);
    let boarded = run(&mut rig.tw, flyer, "+ship/board");
    assert!(
        boarded.contains("You cycle the lock into the "),
        "{boarded}"
    );
    let ship = (flyer, dray, hold);

    // ---- (1) Clean at the Republic's port ----
    sail(&mut rig, ship, "stickney", "stickney_port");
    assert_eq!(
        tacsim::offense::jurisdiction_at(rig.tw.world(), &cid("stickney_port")),
        Some(republic)
    );
    assert!(
        debts(&rig, meridiani).is_empty(),
        "nothing is fined at Stickney"
    );

    // ---- (2) The House answers at Ceres: fined to the Freeport Registry ----
    sail(&mut rig, ship, "ceres", "ceres_port");
    let owed = debts(&rig, meridiani);
    assert_eq!(owed.len(), 1, "one fine, on Meridiani: {owed:?}");
    let fine = owed[0].clone();
    assert_eq!(
        fine.amount.minor(),
        i64::try_from(tacsim::surface::security::FINE_RECOVERY_MAJOR).expect("fits") * 100
    );
    let registry = crate::driving_rig::port_map(&rig.tw, "ceres_port");
    let authority = registry.orgs[&cid("freeport_registry")]
        .account
        .expect("the Registry's account");
    assert_eq!(
        fine.credit_account.0, authority,
        "paid to the port's governing authority"
    );
    assert_eq!(
        tacsim::offense::record_of(rig.tw.world(), &key).and_then(|r| r.rows[0].debt),
        Some(fine.id.0),
        "the receipt is on the House's record"
    );

    // ---- (3) The grace runs out at the Republic's port: she lifts there ----
    sail(&mut rig, ship, "stickney", "stickney_port");
    let due_at =
        rig.tw.world().world_ic_now().at + tacsim::surface::retrieval::DEBT_GRACE_IC_DAYS * 86_400;
    skip_ic(&mut rig, due_at + 3_600);
    assert!(
        matches!(
            rig.tw
                .world()
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|e| e.snapshot().obligation(fine.id).map(|o| o.status.clone())),
            Some(mush_economy::ObligationStatus::Delinquent)
        ),
        "past its grace, unpaid"
    );
    assert!(
        !tacsim::surface::security::holds_at(rig.tw.world(), dray, &cid("stickney_port")),
        "the Freeport's fine does not hold her at the Republic's port"
    );
    sail(&mut rig, ship, "ceres", "ceres_port");

    // ---- (4) Wanted at Ceres: held ----
    let (refused, queued) = command(&mut rig, flyer, "+ship/undock stickney");
    assert!(
        refused.contains("is held at") && refused.contains("lifts when it is paid"),
        "{refused}"
    );
    assert!(queued.is_empty(), "nothing filed: {refused}");

    // ---- (5) Ceres changes hands: the Freeport's offense no longer binds there ----
    let flipped = run(
        &mut rig.tw,
        GOD,
        "@faction/control Ceres station=Republic/defacto",
    );
    assert!(flipped.contains("holds Ceres station"), "{flipped}");
    assert_eq!(
        tacsim::offense::jurisdiction_at(rig.tw.world(), &cid("ceres_port")),
        Some(republic)
    );
    let (cleared, _) = command(&mut rig, flyer, "+ship/undock stickney");
    assert!(
        !cleared.contains("is held at"),
        "she lifts from a port the Freeport no longer holds: {cleared}"
    );
}
