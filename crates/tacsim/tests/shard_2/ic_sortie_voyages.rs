//! **Doc 23 cut S.2 acceptance — the ship's epoch on the port voyage** (doc 23 §6 "What the
//! host does at each end", §7 "Records", §9's S.2 row, §10 tier 1; D-23-1, D-23-3, D-23-5,
//! D-23-7, D-23-8; the S.2 brief's lead rulings V1–V6).
//!
//! A Dray's seal epoch opens when the launch arm's depart takes it off a hangar port and closes
//! `returned_to_base` when it docks at a hangar port again — on its sailing plan
//! (its typed `tacsim.ship_epoch` record) and in its own black box (`log:hull.<n>`). The open and the close are the
//! `tacsim.port` tick's epoch arm's: a pull reconciliation the fire after the move, submitted as
//! the warrant principal (the filing pilot) and landed in one bundle through the black box's
//! participant; a lapsed burn departs nothing and writes nothing.
//!
//! Composed exactly as `voyage.rs` composes — its rig and helpers, whose `land` takes the world
//! task's composed path since this cut. Every boot asserts no namespace write was refused.

use mush_core::{AuthoritativeSubmit, CanonicalId, ObjId};
use mush_roll::commerce::ic_claims::IcCloseKind;
use tacsim::arena::mind::SortieId;
use tacsim::definitions::UnitClass;
use tacsim::deliberation::order::CharacterRef;
use tacsim::port::ship::{Berth, berth_of, hangar_port_of};
use tacsim::port::voyage::{CLEARANCE_LAPSED, Departed};
use tacsim::port::{
    EPOCH_ACTION, EpochKey, EpochOp, HullRow, LAUNCH_GRACE_FIRES, ShipEpoch, ShipEpochRecord,
};
use tacsim::surface::black_box::{BlackBoxLine, EpochHost, Hangar, SealWord};

use crate::voyage::{
    Rig, Roster, aboard_with_load, drain, fire, handles, journey, land, land_all,
    no_refused_writes, port_map, reboot_with, said, store, ticket, to_ic, typed, undock,
    undock_and_burn, voyage,
};

/// A hull's black box lines, decoded (the store's open tail); every entry decodes.
fn lines(rig: &Rig, hull: ObjId) -> Vec<BlackBoxLine> {
    let (lines, skipped) = store(&rig.tw).black_box_lines(hull);
    assert_eq!(skipped, 0, "every black box entry decodes");
    lines.into_iter().map(|(_, line)| line).collect()
}

/// **A hull's epoch record** (`tacsim.ship_epoch`), through an owner-stamped transaction.
fn ship_record(rig: &mut Rig, hull: ObjId) -> Option<ShipEpochRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.epochs, &EpochKey(hull))
}

/// **A hull's open epoch**, from its record.
fn ship_open(rig: &mut Rig, hull: ObjId) -> Option<ShipEpoch> {
    ship_record(rig, hull).and_then(|record| record.open)
}

/// The epoch arm's submissions among `queued`.
fn epochs(queued: &[AuthoritativeSubmit]) -> Vec<&AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == EPOCH_ACTION)
        .collect()
}

/// A lane port's host room.
fn host(rig: &Rig, entry: &str) -> ObjId {
    ObjId(port_map(&rig.tw, entry).host.expect("the port's host room"))
}

/// **A chartered pilot aboard the docked Salt Moth, undocked for Stickney and launched**: the
/// burn landed and the launch fire run; the hull, the departure instant and the arrival instant.
fn launched_for_stickney() -> (Rig, ObjId, HullRow, i64, i64) {
    let (mut rig, pilot, row) = aboard_with_load(3_600, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let t = ticket(&mut rig.tw, hull, pending.seq).expect("the ticket");
    let (_, queued) = fire(&mut rig);
    assert!(
        epochs(&queued).is_empty(),
        "the launch fire departs; the epoch is the next fire's"
    );
    land_all(&mut rig, &queued);
    let departed_at = journey(&rig.tw, hull).expect("a journey").departed().at;
    assert_eq!(departed_at, t.plan.depart_ic);
    (
        rig,
        pilot,
        row,
        departed_at,
        t.plan.depart_ic + t.plan.total_s,
    )
}

/// Fire (6 s on) and land everything, until the epoch arm submitted and its submission landed;
/// that submission. At most `fires` fires.
fn fire_until_epoch(rig: &mut Rig, fires: usize) -> AuthoritativeSubmit {
    for _ in 0..fires {
        let (_, queued) = fire(rig);
        let found = epochs(&queued).first().map(|submit| (*submit).clone());
        land_all(rig, &queued);
        if let Some(submit) = found {
            return submit;
        }
    }
    panic!("the epoch arm submitted nothing in {fires} fires");
}

/// **1. Ceres to Stickney opens and closes** (§9's S.2 row; V1, V2, V4, V5, V6; D-23-1, D-23-5,
/// D-23-8): the launch records the departure (the pilot, the instant, Ceres left) and opens
/// nothing; the next fire's epoch arm submits ONE `tacsim_voyage_epoch` as the pilot, which opens
/// `SortieId::ic(hull, departed().at)` on the sailing plan and writes one `mind_sealed` line —
/// `host: port`, Ceres' entry and host room the hangar, the pilot the principal, the ROM seal
/// (`no core: the ROM`), one hauler admitted. Docked at Stickney (a hangar port), the next
/// submission closes it `returned_to_base` into Stickney's host room at the arrival instant. Two
/// lines in all.
#[test]
#[allow(clippy::too_many_lines)]
fn s2_ceres_to_stickney_opens_and_closes() {
    let (mut rig, pilot, row, departed_at, arrival) = launched_for_stickney();
    let hull = ObjId(row.hull);
    let ceres = host(&rig, "ceres_port");
    let stickney = host(&rig, "stickney_port");
    let v = voyage(&rig.tw, hull).expect("the sailing plan");
    assert_eq!(
        v.departed,
        Some(Departed {
            seq: 1,
            pilot,
            depart_ic: departed_at,
            to_venue: CanonicalId::new("stickney").expect("a venue"),
            from: CanonicalId::new("ceres_port").expect("an entry"),
        }),
        "the launch records the departure (V5)"
    );
    assert_eq!(
        ship_record(&mut rig, hull),
        None,
        "the open is the epoch arm's, not the launch's"
    );
    assert!(lines(&rig, hull).is_empty());
    assert!(matches!(berth_of(rig.tw.world(), hull), Berth::UnderWay));
    assert!(
        hangar_port_of(rig.tw.world(), hull).is_none(),
        "under way is no hangar"
    );

    let (_, queued) = fire(&mut rig);
    let submitted = epochs(&queued);
    assert_eq!(submitted.len(), 1, "one epoch submission: {queued:?}");
    assert_eq!(submitted[0].actor, pilot, "as the filing pilot (D-23-5)");
    land_all(&mut rig, &queued);
    let open = ship_open(&mut rig, hull).expect("the epoch is open");
    assert_eq!(open.id, SortieId::ic(hull, departed_at));
    assert_eq!(
        open.opened_ic, departed_at,
        "the journey's instant, not the fire's"
    );
    assert_eq!(open.from.as_str(), "ceres_port");
    assert_eq!(open.base, ceres);
    assert_eq!(open.principal, pilot);
    assert!(!open.unrecorded, "the line was written");
    let opened = lines(&rig, hull);
    assert_eq!(opened.len(), 1, "{opened:?}");
    let BlackBoxLine::MindSealed {
        unit,
        sortie,
        at_ic,
        host: by,
        hangar,
        site,
        principal,
        seal,
        mind,
        governed,
        licence,
        seal_digest,
        fallback,
        admitted,
        ..
    } = &opened[0]
    else {
        panic!("the first line is the seal: {opened:?}");
    };
    assert_eq!((*unit, *sortie, *at_ic), (hull, open.id, departed_at));
    assert_eq!(*by, EpochHost::Port);
    assert_eq!(
        *hangar,
        Hangar::Port {
            entry: open.from.clone(),
            base: ceres
        },
        "the hangar port left (V4)"
    );
    assert_eq!(site.as_str(), "ceres_port");
    assert_eq!(principal, &Some(CharacterRef::Character { id: pilot }));
    assert_eq!(*seal, SealWord::Sealed, "the Dray has a unit to seal");
    assert_eq!(fallback.as_deref(), Some("no core: the ROM"));
    assert_eq!(mind.as_deref(), Some("Standard Doctrine"));
    assert_eq!(*governed, Some(true));
    assert_eq!(licence.as_deref(), Some("doctrine"));
    assert_eq!(
        open.seal.as_ref().map(|facts| facts.seal_digest.clone()),
        seal_digest.clone(),
        "the record keeps the seal the line prints"
    );
    assert_eq!(admitted.class, Some(UnitClass::Hauler));
    assert_eq!(admitted.host, EpochHost::Port);
    assert_eq!(admitted.count, 1, "one ship an epoch (D-23-8)");

    // Under way: nothing more is due.
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(epochs(&queued).is_empty(), "an open epoch under way idles");
        land_all(&mut rig, &queued);
    }

    // Docked at Stickney, a hangar port: the close.
    let _ = to_ic(&mut rig, arrival);
    if ship_open(&mut rig, hull).is_some() {
        let _ = fire_until_epoch(&mut rig, 3);
    }
    assert_eq!(
        hangar_port_of(rig.tw.world(), hull).map(|def| def.venue.as_str().to_owned()),
        Some("stickney".to_owned()),
        "docked at Stickney, a hangar port (V3)"
    );
    assert_eq!(ship_open(&mut rig, hull), None, "the epoch is closed");
    let record = ship_record(&mut rig, hull).expect("the record");
    assert_eq!(
        record
            .last
            .as_ref()
            .map(|last| (last.op, last.id, last.at_ic, last.written)),
        Some((EpochOp::Close, open.id, arrival, true)),
        "the record's last end is the close"
    );
    assert_eq!(record.attempts, 2, "two ends committed");
    let both = lines(&rig, hull);
    assert_eq!(both.len(), 2, "two lines a passage: {both:?}");
    assert_eq!(both[0], opened[0]);
    assert_eq!(
        both[1],
        BlackBoxLine::SortieClosed {
            unit: hull,
            sortie: open.id,
            at_ic: arrival,
            close: IcCloseKind::ReturnedToBase { base: stickney },
            abandoned: false,
            unrecorded: false,
        }
    );
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(
            epochs(&queued).is_empty(),
            "a closed epoch at a hangar idles"
        );
        land_all(&mut rig, &queued);
    }
    assert_eq!(lines(&rig, hull).len(), 2);
    no_refused_writes(&rig.tw);
}

/// **2. A passage across a server restart keeps the epoch** (§10 tier 1; V1): opened under way,
/// the world reboots from its store; the sailing plan's open epoch and the black box's line come
/// back, the arm opens nothing again, and the arrival after the reboot closes the epoch with the
/// second line.
#[test]
fn s2_a_passage_across_a_restart_keeps_the_epoch() {
    let (mut rig, _pilot, row, departed_at, arrival) = launched_for_stickney();
    let hull = ObjId(row.hull);
    let _ = fire_until_epoch(&mut rig, 3);
    let open = ship_open(&mut rig, hull).expect("the epoch is open");
    assert_eq!(open.id, SortieId::ic(hull, departed_at));
    let opened = lines(&rig, hull);
    assert_eq!(opened.len(), 1);

    let mut rig = reboot_with(rig, Roster::default());
    assert_eq!(
        ship_open(&mut rig, hull),
        Some(open.clone()),
        "the sailing plan keeps the epoch"
    );
    assert_eq!(lines(&rig, hull), opened, "the black box restores");
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(
            epochs(&queued).is_empty(),
            "the arm re-finds the epoch open and opens nothing"
        );
        land_all(&mut rig, &queued);
    }
    let _ = to_ic(&mut rig, arrival);
    if ship_open(&mut rig, hull).is_some() {
        let _ = fire_until_epoch(&mut rig, 3);
    }
    assert_eq!(ship_open(&mut rig, hull), None);
    let both = lines(&rig, hull);
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(both[0], opened[0]);
    assert!(
        matches!(&both[1], BlackBoxLine::SortieClosed { sortie, close: IcCloseKind::ReturnedToBase { .. }, .. } if *sortie == open.id)
    );
    no_refused_writes(&rig.tw);
}

/// **3. A lapsed launch writes nothing** (§9's S.2 row; V5): the burn is filed and never landed,
/// the clearance lapses after [`LAUNCH_GRACE_FIRES`] — no journey, no departure recorded, no
/// epoch, no line, and the arm submits nothing on any fire.
#[test]
fn s2_a_lapsed_launch_writes_nothing() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let (out, dropped) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(dropped.len(), 1, "the burn is dropped, never landed");
    let lapsed = CLEARANCE_LAPSED.replace("{port}", "Stickney Yards");
    let mut told = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let (events, queued) = fire(&mut rig);
        assert!(
            epochs(&queued).is_empty(),
            "nothing departed, nothing is due"
        );
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
        for submit in &queued {
            let _ = land(&mut rig, submit);
        }
    }
    assert_eq!(told, 1, "the clearance lapsed");
    let v = voyage(&rig.tw, hull).expect("the sailing plan");
    assert!(v.pending.is_none());
    assert_eq!(v.departed, None, "nothing departed");
    assert_eq!(ship_record(&mut rig, hull), None, "no epoch record");
    assert!(journey(&rig.tw, hull).is_none());
    assert!(lines(&rig, hull).is_empty(), "no line");
    assert_eq!(store(&rig.tw).black_box_len(hull), 0);
    assert!(
        hangar_port_of(rig.tw.world(), hull).is_some(),
        "still in Ceres' berth"
    );
    no_refused_writes(&rig.tw);
}

/// Fire the ticks again at the world's own current instant (no time moved); the submissions
/// queued, landed where they land.
fn refire_same_instant(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let t = rig.tw.world().now();
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    for submit in &queued {
        let _ = land(rig, submit);
    }
    queued
}

/// **5. A re-run fire mints nothing** (§10 tier 1; V1: the document is the cursor): after the
/// open landed, the ticks fired again at the same instant and twice more queue no second epoch
/// submission, and the open replayed is refused — one `mind_sealed` line, one open epoch. The
/// same after the arrival for the close.
#[test]
fn s2_a_re_run_fire_mints_nothing() {
    let (mut rig, _pilot, row, _, arrival) = launched_for_stickney();
    let hull = ObjId(row.hull);
    let open_submit = fire_until_epoch(&mut rig, 3);
    let open = ship_open(&mut rig, hull).expect("the epoch is open");
    assert!(epochs(&refire_same_instant(&mut rig)).is_empty());
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        assert!(epochs(&queued).is_empty(), "no second open");
        land_all(&mut rig, &queued);
    }
    let refused = land(&mut rig, &open_submit).expect_err("the replayed open is refused");
    assert!(!refused.is_empty());
    assert_eq!(
        ship_open(&mut rig, hull),
        Some(open.clone()),
        "one open epoch"
    );
    assert_eq!(lines(&rig, hull).len(), 1, "one mind_sealed line");

    let _ = to_ic(&mut rig, arrival);
    if ship_open(&mut rig, hull).is_some() {
        let close_submit = fire_until_epoch(&mut rig, 3);
        assert!(
            land(&mut rig, &close_submit).is_err(),
            "the replayed close is refused"
        );
    }
    assert_eq!(ship_open(&mut rig, hull), None);
    assert!(epochs(&refire_same_instant(&mut rig)).is_empty());
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        assert!(epochs(&queued).is_empty(), "no second close");
        land_all(&mut rig, &queued);
    }
    // The fix round's R3: the same id never opens twice — the open replayed after its close is
    // refused, and the record keeps the close.
    assert!(
        land(&mut rig, &open_submit).is_err(),
        "the closed epoch's open replayed is refused"
    );
    assert_eq!(ship_open(&mut rig, hull), None);
    let both = lines(&rig, hull);
    assert_eq!(both.len(), 2, "one open line, one close line: {both:?}");
    assert!(matches!(both[1], BlackBoxLine::SortieClosed { .. }));
    no_refused_writes(&rig.tw);
}
