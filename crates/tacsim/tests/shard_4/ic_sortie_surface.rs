//! **Doc 23 cut S.1 acceptance — the first IC sortie, on the surface excursion** (doc 23 §6
//! "What the host does at each end", §7 "Records", §9's S.1 row, §10 tier 1; D-23-1, D-23-2,
//! D-23-4, D-23-5, D-23-7, D-23-8; the S.1 brief's lead rulings L1–L8).
//!
//! A rig's seal epoch opens when it leaves the Dray-P's bay at its launch's end and closes at
//! `+tv/recover`'s end or a staff home, on the rig's `tacsim.excursion` record
//! (`RigRecord::sortie`) and in the hull's durable black box (`log:hull.<n>`, two lines an
//! excursion). The epoch is seal and record: no mind thinks, no pool is spent, no claim opens.
//!
//! Composed exactly as `excursions.rs` composes — its rig and helpers, whose `land` takes the
//! world task's composed path, so the black box participant appends in each finish's own bundle
//! and its chunk reaches the store a reboot boots from. Every world asserts no namespace write
//! was refused.
//!
//! **Doc 23 cut S.3 — abandonment and recovery** (doc 23 §2 "Recovery", "Abandonment", §4
//! D-23-4, §9's S.3 row; the S.3 brief's lead rulings A1–A6): the `s3_` tests below — an
//! abandoned rig keeps its flag and its instant, told once; a staff home of an abandoned rig is
//! a recovery, naming its recoverer; a home mid-excursion still returns; a second carrier brings
//! another's abandoned rig in; the home's refusals; and a ship placed into a hangar port is
//! recovered.

use mush_core::{GOD, ObjId};
use tacsim::arena::mind::SortieId;
use tacsim::definitions::UnitClass;
use tacsim::deliberation::order::CharacterRef;
use tacsim::surface::black_box::{BlackBoxLine, EpochHost, Hangar, SealWord};
use tacsim::surface::{ActKind, ActResult, RefusedCode, TvState};

use mush_roll::commerce::ic_claims::IcCloseKind;

use crate::excursions::{
    RAMP, act_go, command, count, due_of, finish, finishes, fire, god, in_bay, inspections,
    keeping, land, land_all, land_receipt, no_refused_writes, on_pad, on_pad_as, port_map,
    projection, prospector, put_charge, reboot_with, reboot_with_inspection, record_of,
    recovery_order, recovery_staff, said, staff_bay, staff_place, staff_place_on, station_room,
    stocked, store, teleport,
};

/// A hull's black box lines, decoded (the store's open tail); every entry decodes (the fix
/// round's F4: nothing is dropped silently).
pub(crate) fn lines(rig: &crate::excursions::Rig, hull: ObjId) -> Vec<BlackBoxLine> {
    let (lines, skipped) = store(&rig.tw).black_box_lines(hull);
    assert_eq!(skipped, 0, "every black box entry decodes");
    lines.into_iter().map(|(_, line)| line).collect()
}

/// **The Dray-P's open epoch** (doc 23 cut S.2): its typed `tacsim.ship_epoch` record's — the
/// harness flies the Dray-P to Ahuna Flank (`on_pad`), so the ship's epoch opened on the way and
/// stays open on the site's pad (D-23-3).
fn ship_epoch(rig: &mut crate::excursions::Rig, hull: ObjId) -> Option<tacsim::port::ShipEpoch> {
    let h = crate::excursions::handles(&rig.tw);
    crate::excursions::typed(
        &mut rig.tw,
        &h.families.epochs,
        &tacsim::port::EpochKey(hull),
    )
    .and_then(|record: tacsim::port::ShipEpochRecord| record.open)
}

/// **The whole black box read back from the store's documents** — the history read over the
/// chunk documents (`black_box::history`), not the RAM tail. Doc 17h cut M8a (A5): neither
/// player verb reads this; `+hull/box` and `@registry/inspect` read the store's open tail only.
/// A Registry history read over the sealed chunks is the continuation pattern's
/// (`registry_service`'s `ContinuationHandle`, the rating job's shape) and is cut M8d's or
/// M8g's.
fn history(rig: &crate::excursions::Rig, hull: ObjId) -> Vec<BlackBoxLine> {
    let docs: Vec<mush_core::LedgerDoc> = rig
        .store
        .get(tacsim::NS)
        .map(|data| {
            data.kv
                .iter()
                .filter(|(key, _)| mush_core::is_log_key(key))
                .map(|(key, value)| (key.clone(), value.to_string()))
                .chain(data.ledger.iter().cloned())
                .collect()
        })
        .unwrap_or_default();
    tacsim::surface::black_box::history(&docs, hull)
        .expect("the black box's history reads")
        .into_iter()
        .map(|(_, line)| line)
        .collect()
}

/// **1. One excursion opens and closes, with a restart between** (§10 tier 1 item 1; L3, L5,
/// L7, L8; D-23-2, D-23-5, D-23-8): launched and finished, the record holds the open epoch —
/// `SortieId::ic(tv, launch_ic)`, the Dray-P its hangar, opened at the launch's due instant, not
/// abandoned — and the black box one `mind_sealed` line: the Dray-P and (doc 23 cut S.2, V7) the
/// Dray-P's own open epoch as the hangar, the operator's character the principal, the ROM seal
/// (`no core: the ROM`, Class 0, governed, `doctrine`), the vehicle class admitted as the site's
/// one open epoch. A reboot from the store keeps the record and the line. Recovered and
/// finished, the epoch is closed and the black box holds the `sortie_closed` line,
/// `returned_to_base` into the Dray-P, not abandoned — two lines in all, which a second reboot
/// and the documents' own history read both give back. The Dray-P's own black box holds its one
/// open line throughout (cut S.2: it flew to Ahuna under an epoch the site's pad keeps open).
#[test]
#[allow(clippy::too_many_lines)]
fn s1_one_excursion_opens_and_closes_across_a_restart() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let launch_ic = record_of(&mut rig.tw, tv)
        .and_then(|record| record.excursion)
        .map(|open| open.id.launch_ic)
        .expect("the start opens the excursion");
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None,
        "the epoch opens at the launch's end, not its start"
    );
    let due = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let open = record.sortie.clone().expect("the epoch is open");
    assert_eq!(open.id, SortieId::ic(tv, launch_ic));
    assert!(!open.abandoned);
    assert_eq!(
        (open.hull, open.slot, open.operator, open.opened_ic),
        (hull, 1, pilot, due)
    );
    assert_eq!(open.site.as_str(), "ahuna_port");
    let opened = lines(&rig, tv);
    assert_eq!(opened.len(), 1, "{opened:?}");
    let BlackBoxLine::MindSealed {
        unit,
        sortie,
        at_ic,
        host,
        hangar,
        site,
        principal,
        seal,
        mind,
        class,
        governed,
        licence,
        licence_state,
        seal_digest,
        fallback,
        admitted,
        ..
    } = &opened[0]
    else {
        panic!("the first line is the seal: {opened:?}");
    };
    assert_eq!((*unit, *sortie, *at_ic), (tv, open.id, due));
    assert_eq!(*host, EpochHost::Surface);
    let ship = ship_epoch(&mut rig, hull).expect("the Dray-P's epoch is open on the pad (S.2)");
    assert_eq!(
        *hangar,
        Hangar::Bay {
            hull,
            slot: 1,
            sortie: Some(ship.id),
        },
        "the hangar is the Dray-P (D-23-2) and names its open epoch (S.2's V7)"
    );
    assert_eq!(site.as_str(), "ahuna_port");
    assert_eq!(
        principal,
        &Some(CharacterRef::Character { id: pilot }),
        "the warrant principal is the operator's character (D-23-5)"
    );
    assert_eq!(*seal, SealWord::Sealed, "the rig has a unit to seal");
    assert_eq!(fallback.as_deref(), Some("no core: the ROM"));
    assert_eq!(mind.as_deref(), Some("Standard Doctrine"));
    assert_eq!(*class, Some(tacsim::arena::CoreClass::Zero));
    assert_eq!(*governed, Some(true));
    assert_eq!(licence.as_deref(), Some("doctrine"));
    // Doc 17h cut M8a (A1, A2; the brief's test 1): the line carries the licence's state
    // beside its label, derived at the seal from the doctrine origin, and the record's seal
    // facts carry the same word.
    assert_eq!(
        *licence_state,
        Some(tacsim::minds::licence_state::SealLicenceState::Doctrine)
    );
    assert_eq!(
        open.seal.as_ref().and_then(|facts| facts.licence_state),
        Some(tacsim::minds::licence_state::SealLicenceState::Doctrine)
    );
    assert_eq!(
        seal_digest.as_deref().map(str::len),
        Some(64),
        "the whole seal's digest"
    );
    assert_eq!(
        open.seal.as_ref().map(|facts| facts.seal_digest.clone()),
        seal_digest.clone(),
        "the record keeps the seal the line prints"
    );
    assert_eq!(admitted.class, Some(UnitClass::Vehicle));
    assert_eq!(admitted.host, EpochHost::Surface);
    assert_eq!(admitted.count, 1, "the site's one open epoch (D-23-8)");

    // A restart: the record and the black box survive.
    let mut rig = reboot_with(rig, &[pilot]);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        Some(open.clone())
    );
    assert_eq!(lines(&rig, tv), opened, "the black box restores");

    // The recover closes it.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let due = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.sortie, None, "the epoch is closed");
    assert_eq!(record.excursion, None);
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "two lines an excursion: {both:?}");
    assert_eq!(both[0], opened[0]);
    assert_eq!(
        both[1],
        BlackBoxLine::SortieClosed {
            unit: tv,
            sortie: open.id,
            at_ic: due,
            close: IcCloseKind::ReturnedToBase { base: hull },
            abandoned: false,
            abandoned_ic: None,
            recovered_by: None,
            unrecorded: false,
        }
    );
    assert!(!open.unrecorded, "the open line was written");
    let mut rig = reboot_with(rig, &[pilot]);
    assert_eq!(lines(&rig, tv), both, "both lines survive a second restart");
    assert_eq!(
        history(&rig, tv),
        both,
        "an inspection reads them from the documents"
    );
    assert_eq!(store(&rig.tw).black_box_len(tv), 2);
    assert!(!store(&rig.tw).black_box_dark());
    assert_eq!(
        lines(&rig, hull).len(),
        1,
        "the Dray-P's own black box holds its open line alone (S.2: its epoch stays open on the \
         pad)"
    );
    let _ = fire(&mut rig);
    no_refused_writes(&rig.tw);
}

/// **2. A parked rig keeps its epoch through the idle return; staff `home` closes it
/// `RecoveredToBase`** (§9's S.1 row; L6): launched, finished, left two cells off the ramp with a
/// charge short of its way home, the rig stands idle a day — the idle return parks it where it
/// stands (the projection reads `Parked`) and submits nothing; the epoch is still open after the
/// park and after further fires, with one line. A staff home closes it at the receipt,
/// `recovered_to_base` into the Dray-P (the durable facts read it parked, not the projection).
#[test]
fn s1_a_parked_rig_keeps_its_epoch_until_staff_home_recovers_it() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 2_000);
    put_charge(&mut rig, tv, 1);
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let (_, queued) = crate::excursions::advance_to(&mut rig, since + 24 * 3_600);
    assert!(
        queued
            .iter()
            .all(|submit| submit.action.key().id.as_str() != tacsim::surface::SURFACE_ACT_ACTION),
        "no recover it cannot pay: {queued:?}"
    );
    land_all(&mut rig, &queued);
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Parked),
        "parked where it stands"
    );
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        Some(open.clone()),
        "a parked rig's epoch stays open"
    );
    assert_eq!(lines(&rig, tv).len(), 1);

    let staff = recovery_staff(&mut rig.tw);
    recovery_order(&mut rig, staff, "home", tv);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.sortie, None,
        "the home closes the epoch at its receipt"
    );
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "{both:?}");
    let BlackBoxLine::SortieClosed {
        sortie,
        close,
        abandoned,
        ..
    } = &both[1]
    else {
        panic!("the close line: {both:?}");
    };
    assert_eq!(*sortie, open.id);
    assert_eq!(*close, IcCloseKind::RecoveredToBase { base: hull });
    assert!(!abandoned);
    // Doc 23 cut S.3 (A3): a parked rig's recovery names the staff actor who brought it in.
    assert!(
        matches!(&both[1], BlackBoxLine::SortieClosed { recovered_by: Some(by), abandoned_ic: None, .. } if *by == staff),
        "{both:?}"
    );
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert!(
        in_bay(&rig.tw, tv, &row),
        "the pending home lands on the tick"
    );
    assert_eq!(
        lines(&rig, tv).len(),
        2,
        "the home's finish writes nothing more"
    );
    no_refused_writes(&rig.tw);
}

/// **3. A staff close with the rig out flags the epoch and keeps it** (§10 tier 1 item 3;
/// D-23-4; L5): after the launch's end, staff `close` clears the excursion and keeps the epoch,
/// `abandoned`, with no `sortie_closed` line; a staff `home` then closes it with `abandoned` on
/// the line. **Doc 23 cut S.3** (A1–A3; the S.3 brief's test 2, amending this one): the close's
/// IC instant is the epoch's `abandoned_ic`, and a staff home of an abandoned rig is a recovery
/// — `recovered_to_base` into its own hull (S.1 asserted `returned_to_base`), the staff actor
/// the recoverer, `abandoned_ic` on the line; the record's epoch is gone.
#[test]
fn s1_a_staff_close_with_the_rig_out_flags_and_keeps_the_epoch() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    let staff = recovery_staff(&mut rig.tw);
    let t = rig.tw.world().world_ic_now().at;
    recovery_order(&mut rig, staff, "close", tv);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.excursion, None, "the close clears the excursion");
    let kept = record.sortie.clone().expect("the epoch is kept");
    assert!(kept.abandoned, "flagged abandoned");
    assert_eq!(
        kept.abandoned_ic,
        Some(t),
        "with the close's instant (S.3, A1)"
    );
    assert_eq!(kept.id, open.id);
    assert_eq!(lines(&rig, tv).len(), 1, "abandonment closes nothing");
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        Some(kept)
    );

    recovery_order(&mut rig, staff, "home", tv);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None
    );
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "{both:?}");
    let BlackBoxLine::SortieClosed {
        close,
        abandoned,
        abandoned_ic,
        recovered_by,
        ..
    } = &both[1]
    else {
        panic!("the close line: {both:?}");
    };
    assert_eq!(
        *close,
        IcCloseKind::RecoveredToBase { base: hull },
        "a staff home of an abandoned rig is a recovery (S.3, A2)"
    );
    assert!(*abandoned, "the line carries the flag");
    assert_eq!(*abandoned_ic, Some(t), "and its instant (S.3, A1)");
    assert_eq!(*recovered_by, Some(staff), "and the recoverer (S.3, A3)");
    no_refused_writes(&rig.tw);
}

/// Fire the ticks again at the world's own current instant (the same due instant, no time
/// moved); the submissions queued.
fn refire_same_instant(rig: &mut crate::excursions::Rig) -> Vec<mush_core::AuthoritativeSubmit> {
    let t = rig.tw.world().now();
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    crate::excursions::drain(&mut rig.tw)
}

/// **4. A re-run tick mints nothing** (§10 tier 1 item 3; L4; the fix round's F5): the launch's
/// finish queued at its due instant, landed; the ticks fired again at that same instant and 6 s
/// on queue no second finish, and the finish landed again is refused "already finished" — one
/// `mind_sealed` line, one open epoch. After a reboot the same: the ticks re-fired at the
/// reboot's instant and on queue nothing for it, the finish replayed is refused, one line, one
/// epoch. The same for the recover's close.
#[test]
#[allow(clippy::too_many_lines)]
fn s1_a_re_run_finish_mints_nothing() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = crate::excursions::advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1, "one finish at the due instant");
    land_all(&mut rig, &queued);
    let same = refire_same_instant(&mut rig);
    assert!(
        finishes(&same).is_empty(),
        "no second finish at the same instant"
    );
    land_all(&mut rig, &same);
    let (_, again) = fire(&mut rig);
    assert!(finishes(&again).is_empty(), "no second finish queued");
    land_all(&mut rig, &again);
    let done = finishes(&queued)[0].clone();
    let refused = land(&mut rig, &done).expect_err("the replayed finish refuses");
    assert!(refused.contains("already finished"), "{refused}");
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("one open epoch");
    let opened = lines(&rig, tv);
    assert_eq!(opened.len(), 1, "one mind_sealed line: {opened:?}");
    // Across a reboot: nothing re-mints.
    let mut rig = reboot_with(rig, &[pilot]);
    let same = refire_same_instant(&mut rig);
    assert!(finishes(&same).is_empty(), "no finish after the reboot");
    land_all(&mut rig, &same);
    let (_, again) = fire(&mut rig);
    assert!(finishes(&again).is_empty());
    land_all(&mut rig, &again);
    assert!(
        land(&mut rig, &done).is_err(),
        "the replayed finish refuses after the reboot"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        Some(open.clone()),
        "one open epoch after the reboot"
    );
    assert_eq!(
        lines(&rig, tv),
        opened,
        "one mind_sealed line after the reboot"
    );

    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = crate::excursions::advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    let (_, again) = fire(&mut rig);
    assert!(finishes(&again).is_empty());
    land_all(&mut rig, &again);
    let done = finishes(&queued)[0].clone();
    assert!(
        land(&mut rig, &done).is_err(),
        "the replayed recover refuses"
    );
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "one sortie_closed line: {both:?}");
    assert_eq!(both[1].sortie(), open.id);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None
    );
    no_refused_writes(&rig.tw);
}

/// **6. A staff close between the tick's placement and the launch's finish opens the epoch,
/// abandoned** (the fix round's F3; D-23-1, D-23-4): the tick places the launching rig down the
/// ramp at the due instant and queues the finish; staff `close` lands first — the excursion and
/// the act clear, and the epoch the finish would have opened opens in the close's own commit,
/// abandoned on the record (never on the line), with one `mind_sealed` line as the finish would
/// have written it. The held finish is refused as stale and writes nothing; a staff `home` then
/// closes the epoch, abandoned — **doc 23 cut S.3** (A2): `recovered_to_base`, a staff home of
/// an abandoned rig being a recovery (S.1 asserted `returned_to_base`).
#[test]
fn s1_a_staff_close_before_the_launch_finish_opens_the_epoch_abandoned() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let launch_ic = record_of(&mut rig.tw, tv)
        .and_then(|record| record.excursion)
        .map(|open| open.id.launch_ic)
        .expect("the start opens the excursion");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = crate::excursions::advance_to(&mut rig, due);
    let held = finishes(&queued);
    assert_eq!(held.len(), 1, "the launch's finish is held before landing");
    let done = held[0].clone();
    assert!(
        !in_bay(&rig.tw, tv, &row),
        "the tick placed the rig down the ramp"
    );
    let staff = recovery_staff(&mut rig.tw);
    recovery_order(&mut rig, staff, "close", tv);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.act.is_none() && record.excursion.is_none());
    let open = record.sortie.clone().expect("the close opens the epoch");
    assert_eq!(open.id, SortieId::ic(tv, launch_ic));
    assert_eq!((open.hull, open.opened_ic), (hull, due));
    assert!(open.abandoned, "abandoned on the record");
    assert!(!open.unrecorded, "its line was written");
    let opened = lines(&rig, tv);
    assert_eq!(opened.len(), 1, "{opened:?}");
    let BlackBoxLine::MindSealed {
        sortie,
        hangar,
        seal,
        principal,
        ..
    } = &opened[0]
    else {
        panic!("the open line: {opened:?}");
    };
    assert_eq!(*sortie, open.id);
    assert!(matches!(hangar, Hangar::Bay { hull: of, .. } if *of == hull));
    assert_eq!(*seal, SealWord::Sealed);
    assert_eq!(principal, &Some(CharacterRef::Character { id: pilot }));
    let refused = land(&mut rig, &done).expect_err("the held finish is stale");
    assert!(!refused.is_empty());
    assert_eq!(lines(&rig, tv), opened, "the stale finish writes nothing");
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        Some(open.clone())
    );
    recovery_order(&mut rig, staff, "home", tv);
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "{both:?}");
    let BlackBoxLine::SortieClosed {
        sortie,
        close,
        abandoned,
        unrecorded,
        ..
    } = &both[1]
    else {
        panic!("the close line: {both:?}");
    };
    assert_eq!(*sortie, open.id);
    assert_eq!(
        *close,
        IcCloseKind::RecoveredToBase { base: hull },
        "a staff home of an abandoned rig is a recovery (S.3, A2)"
    );
    assert!(*abandoned && !*unrecorded);
    no_refused_writes(&rig.tw);
}

/// **7. A staff close between the tick's placement and the recover's finish closes the epoch**
/// (the fix round's F3): the tick climbs the recovering rig into its bay at the due instant and
/// queues the finish; staff `close` lands first and closes the epoch `returned_to_base`, not
/// abandoned, in its own commit. The held finish is refused as stale and writes nothing.
#[test]
fn s1_a_staff_close_before_the_recover_finish_closes_the_epoch() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = crate::excursions::advance_to(&mut rig, due);
    let held = finishes(&queued);
    assert_eq!(held.len(), 1, "the recover's finish is held before landing");
    let done = held[0].clone();
    assert!(
        in_bay(&rig.tw, tv, &row),
        "the tick climbed the rig into its bay"
    );
    let staff = recovery_staff(&mut rig.tw);
    recovery_order(&mut rig, staff, "close", tv);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None,
        "the close closes the epoch"
    );
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "{both:?}");
    let BlackBoxLine::SortieClosed {
        sortie,
        close,
        abandoned,
        ..
    } = &both[1]
    else {
        panic!("the close line: {both:?}");
    };
    assert_eq!(*sortie, open.id);
    assert_eq!(*close, IcCloseKind::ReturnedToBase { base: hull });
    assert!(!abandoned);
    assert!(land(&mut rig, &done).is_err(), "the held finish is stale");
    assert_eq!(lines(&rig, tv), both, "the stale finish writes nothing");
    no_refused_writes(&rig.tw);
}

/// **5. A launch that never left writes nothing** (L3): staff move the launching rig three cells
/// off before its due instant — the finish records `Refused { NeverLeft }`, no epoch opens and
/// the hull's black box stays empty.
#[test]
fn s1_a_launch_that_never_left_writes_nothing() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 3_000);
    let _ = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|last| (last.kind, last.result)),
        Some((
            ActKind::Launch,
            ActResult::Refused {
                code: RefusedCode::NeverLeft
            }
        ))
    );
    assert_eq!(record.sortie, None, "no epoch opens");
    assert!(lines(&rig, tv).is_empty(), "no line");
    assert_eq!(store(&rig.tw).black_box_len(tv), 0);
    no_refused_writes(&rig.tw);
}

/// **4. A settle on a site's pad keeps the ship's epoch open, and a rig launched there names it**
/// (doc 23 cut S.2; V7, D-23-2, D-23-3): the Dray-P flew from Ceres to Ahuna Flank (`on_pad`), so
/// its epoch opened — `host: port`, Ceres left — and the pad, a site, is no hangar: further fires
/// close nothing. A rig launched from the bay writes its `mind_sealed` line naming the Dray-P and
/// the Dray-P's open epoch as its hangar; after the rig's recover the ship's epoch is still open,
/// with its one line.
#[test]
fn s2_a_rig_launched_on_a_pad_names_its_carriers_open_epoch() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let ship = ship_epoch(&mut rig, hull).expect("the Dray-P's epoch opened on the way");
    assert!(
        tacsim::port::ship::hangar_port_of(rig.tw.world(), hull).is_none(),
        "a site's pad is no hangar (V3)"
    );
    assert_eq!(ship.from.as_str(), "ceres_port");
    let shipped = lines(&rig, hull);
    assert_eq!(shipped.len(), 1, "{shipped:?}");
    // Doc 17h cut M8a (the brief's test 1): the ship's open line carries the licence word
    // and its state too (the ROM seal: `doctrine`).
    assert!(matches!(
        &shipped[0],
        BlackBoxLine::MindSealed { host: EpochHost::Port, sortie, licence, licence_state, .. }
            if *sortie == ship.id
                && licence.as_deref() == Some("doctrine")
                && *licence_state == Some(tacsim::minds::licence_state::SealLicenceState::Doctrine)
    ));
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(
            queued
                .iter()
                .all(|submit| submit.action.key().id.as_str() != tacsim::port::EPOCH_ACTION),
            "the pad closes nothing (D-23-3)"
        );
        land_all(&mut rig, &queued);
    }

    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let opened = lines(&rig, tv);
    assert_eq!(opened.len(), 1, "{opened:?}");
    let BlackBoxLine::MindSealed { hangar, .. } = &opened[0] else {
        panic!("the rig's open line: {opened:?}");
    };
    assert_eq!(
        *hangar,
        Hangar::Bay {
            hull,
            slot: 1,
            sortie: Some(ship.id),
        },
        "the rig names its carrier's open epoch (V7)"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None,
        "the rig's epoch closed"
    );
    assert_eq!(lines(&rig, tv).len(), 2);
    assert_eq!(
        ship_epoch(&mut rig, hull),
        Some(ship),
        "the ship's epoch is still open after the rig's recover"
    );
    assert_eq!(lines(&rig, hull), shipped, "the ship's box is unmoved");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Doc 23 cut S.3 — abandonment and recovery
// ---------------------------------------------------------------------------

/// The world's IC instant.
fn ic_now(rig: &crate::excursions::Rig) -> i64 {
    rig.tw.world().world_ic_now().at
}

/// A rig's open epoch's abandonment: `(abandoned, abandoned_ic)`.
fn abandonment(rig: &mut crate::excursions::Rig, tv: ObjId) -> Option<(bool, Option<i64>)> {
    record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .map(|open| (open.abandoned, open.abandoned_ic))
}

/// The abandonment notice (A1): the close receipt's line to the operator.
fn abandoned_notice(tv: ObjId) -> String {
    format!(
        "Staff closed rig #{}'s excursion and cancelled its work. It stands where it is, \
         abandoned, until staff bring it home; its hopper is unchanged.",
        tv.0
    )
}

/// The plain close's notice: a close that abandons nothing new.
fn plain_close_notice(tv: ObjId) -> String {
    format!(
        "Staff closed rig #{}'s excursion and cancelled its work. Its location and hopper are \
         unchanged.",
        tv.0
    )
}

/// Fire `n` times, landing everything; how many times `who` heard `line`.
fn fire_and_count(rig: &mut crate::excursions::Rig, n: usize, who: ObjId, line: &str) -> usize {
    let mut heard = 0;
    for _ in 0..n {
        let (events, queued) = fire(rig);
        heard += count(&said(&rig.tw, &events, who), line);
        land_all(rig, &queued);
    }
    heard
}

/// **S.3 test 1. An abandoned rig keeps its flag and its instant across a restart** (A1;
/// D-23-4): launched, finished and closed by staff at IC `t`, the record's epoch is
/// `abandoned` with `abandoned_ic == Some(t)`, and after a reboot the same. The lessee's launch
/// from the ground is refused as the rig is out of the bay; a second staff `close` keeps `t`;
/// and once staff put the rig back in its bay by hand, a launch is refused by S.1's start
/// refusal — no act takes an abandoned rig out. The operator heard the abandonment notice
/// exactly once, counted across the reboot (the second close says the plain close).
#[test]
fn s3_an_abandoned_rig_keeps_its_flag_and_its_instant_across_a_restart() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let staff = recovery_staff(&mut rig.tw);
    let t = ic_now(&rig);
    recovery_order(&mut rig, staff, "close", tv);
    assert_eq!(abandonment(&mut rig, tv), Some((true, Some(t))));
    let notice = abandoned_notice(tv);
    let mut heard = fire_and_count(&mut rig, 3, pilot, &notice);
    assert_eq!(heard, 1, "the owner is told once");

    let mut rig = reboot_with(rig, &[pilot, staff]);
    assert_eq!(
        abandonment(&mut rig, tv),
        Some((true, Some(t))),
        "the flag and the instant survive a restart"
    );
    heard += fire_and_count(&mut rig, 3, pilot, &notice);
    assert_eq!(heard, 1, "and not again after the reboot");

    // No act takes it out: from the ground the launch is refused, the rig being out.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch go");
    assert_eq!(out.trim(), tacsim::port::tv::LAUNCH_OUT.replace("{n}", "1"));
    assert!(queued.is_empty());

    // A second close keeps the first instant, and says the plain close.
    assert!(ic_now(&rig) > t, "time moved");
    recovery_order(&mut rig, staff, "close", tv);
    assert_eq!(
        abandonment(&mut rig, tv),
        Some((true, Some(t))),
        "a second close keeps the first instant"
    );
    let plain = plain_close_notice(tv);
    let mut told_plain = 0;
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        let said_now = said(&rig.tw, &events, pilot);
        heard += count(&said_now, &notice);
        told_plain += count(&said_now, &plain);
        land_all(&mut rig, &queued);
    }
    assert_eq!(heard, 1, "the abandonment is told once in all");
    assert_eq!(told_plain, 1, "the second close says the plain close, once");

    // Put back in its bay by a staff write, the rig still cannot leave: S.1's start refusal.
    staff_bay(&mut rig, tv, &row);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch go");
    assert_eq!(queued.len(), 1, "the start is submitted: {out}");
    let refused = land(&mut rig, &queued[0]).expect_err("the start is refused");
    assert!(
        refused.contains("that rig's last trip is still open; staff must bring it home first"),
        "{refused}"
    );
    assert_eq!(abandonment(&mut rig, tv), Some((true, Some(t))));
    assert_eq!(lines(&rig, tv).len(), 1, "abandonment closes nothing");
    no_refused_writes(&rig.tw);
}

/// **S.3 test 3. A home mid-excursion with the charge to come home still returns** (A2's
/// "otherwise", A3): launched and finished, never closed, a staff `home` closes the epoch
/// `returned_to_base` into the Dray-P — not abandoned, no recoverer, no instant.
#[test]
fn s3_a_home_mid_excursion_still_returns() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    let staff = recovery_staff(&mut rig.tw);
    let t = ic_now(&rig);
    recovery_order(&mut rig, staff, "home", tv);
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None
    );
    let both = lines(&rig, tv);
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(
        both[1],
        BlackBoxLine::SortieClosed {
            unit: tv,
            sortie: open.id,
            at_ic: t,
            close: IcCloseKind::ReturnedToBase { base: hull },
            abandoned: false,
            abandoned_ic: None,
            recovered_by: None,
            unrecorded: false,
        }
    );
    no_refused_writes(&rig.tw);
}

/// `@tacsim/tv home #<tv> #<into>` as staff, and exactly one recovery queued; landed.
fn home_into(rig: &mut crate::excursions::Rig, staff: ObjId, tv: ObjId, into: ObjId) {
    let line = format!("@tacsim/tv home #{} #{}", tv.0, into.0);
    let (out, queued) = command(rig, staff, &line);
    assert_eq!(queued.len(), 1, "{line}: {out}");
    assert_eq!(queued[0].actor, staff);
    assert_eq!(
        queued[0].action.key().id.as_str(),
        "tacsim_surface_recovery"
    );
    land_all(rig, &queued);
}

/// A launched and finished rig on Ahuna Flank, abandoned by a staff `close`, its carrier then
/// taken off the pad by a staff write (`mush_sim::unplace` through `keeping`, so `berth_of`
/// reads `Elsewhere`), the Tally Moth flown in and settled beside it before that: the pilot, the
/// carrier, the rig, the staff actor, the close's instant, and the Tally Moth's pilot and row.
fn abandoned_and_carrier_gone(
    rig: &mut crate::excursions::Rig,
) -> (
    ObjId,
    ObjId,
    ObjId,
    ObjId,
    i64,
    (ObjId, tacsim::port::HullRow),
) {
    let (pilot, row, tvs) = on_pad(rig, 1);
    let tv = tvs[0];
    let salt = ObjId(row.hull);
    let _ = act_go(rig, pilot, "+tv/1 launch go");
    let _ = finish(rig, tv, pilot);
    let staff = recovery_staff(&mut rig.tw);
    let t = ic_now(rig);
    recovery_order(rig, staff, "close", tv);
    // The second carrier flies in first: a programme hull taken off every map holds the Co-op's
    // enrolment ("the programme is still being set up"), so the Tally Moth enrols and flies
    // while the Salt Moth still stands on the pad beside it, then the Salt Moth is taken away.
    let (other, tally_row, none) = on_pad_as(rig, "Other", 2, "Tally Moth", 0);
    assert!(none.is_empty());
    keeping(rig, |ctx| {
        mush_sim::unplace(
            ctx,
            salt,
            "doc 23 cut S.3: the abandoned rig's carrier is gone",
        )
    })
    .expect("staff take the Salt Moth off the pad");
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), salt),
        tacsim::port::ship::Berth::Elsewhere
    ));
    (pilot, salt, tv, staff, t, (other, tally_row))
}

/// **S.3 test 4. A second carrier recovers an abandoned rig** (A4; §2 "Recovery"; D-23-2): the
/// Salt Moth's rig is launched, finished and abandoned on Ahuna Flank; the Tally Moth, another
/// pilot's Dray-P with no rigs, flies Ceres to Ahuna by the production path and settles; then the
/// Salt Moth is taken off the pad by a staff write. Staff `home #<rig> #<Tally Moth>`: the receipt
/// keeps the assigned hull and names the recoverer (`into`) with the recoverer's bay, and (the
/// fix round's R1) leaves the epoch open and abandoned, with no line. Across a reboot between the
/// order and the completion, the tick places the rig `Interior` in the Tally Moth's bay and the
/// finish lands, closing the epoch `recovered_to_base` into the Tally Moth, the staff actor the
/// recoverer, abandoned with its instant; the operator hears the home naming the Tally Moth, and
/// the projection reads `Out` (R3: a foreign bay, as the tick reads it). The lease is untouched;
/// the lessee's `+tv/1 launch` is refused with `LAUNCH_NOT_SITE` — because the lessee's own hull
/// is off the pad, not because the rig sits in another hull's bay (today's behaviour; doc 23 §8
/// defers what either may do) — and the Tally Moth's `+ship/lift` is filed, not held: the
/// departure guard walks the hull's leased rigs, and the rig rides along.
#[test]
fn s3_a_second_carrier_recovers_an_abandoned_rig() {
    let mut rig = stocked();
    let (pilot, salt, tv, staff, t, (other, tally_row)) = abandoned_and_carrier_gone(&mut rig);
    let tally = ObjId(tally_row.hull);
    let lease = store(&rig.tw).tv_lease_on(tv).cloned().expect("the lease");
    assert_eq!(lease.hull, salt);

    home_into(&mut rig, staff, tv, tally);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record
            .sortie
            .as_ref()
            .map(|open| (open.abandoned, open.abandoned_ic)),
        Some((true, Some(t))),
        "the epoch stays open and abandoned until the completion (R1)"
    );
    let receipt = record.recovery.expect("the receipt");
    assert_eq!(
        (receipt.into, receipt.hull, receipt.pending),
        (Some(tally), salt, true),
        "the receipt names the recoverer and keeps the assigned hull"
    );
    assert_eq!(
        receipt.bay_room,
        Some(ObjId(
            tally_row.rooms[&mush_core::CanonicalId::new("bay").expect("an id")]
        )),
        "the recoverer's bay"
    );
    assert_eq!(
        lines(&rig, tv).len(),
        1,
        "no close line at the receipt (R1)"
    );

    let mut rig = reboot_with(rig, &[pilot, other, staff]);
    let notice = format!(
        "Staff brought rig #{} into hull #{}'s bay. Its hopper is unchanged; unload it before \
         departure.",
        tv.0, tally.0
    );
    let heard = fire_and_count(&mut rig, 4, pilot, &notice);
    assert!(
        in_bay(&rig.tw, tv, &tally_row),
        "the rig lands Interior in the recoverer's bay"
    );
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.recovery)
            .is_some_and(|receipt| !receipt.pending),
        "the finish landed"
    );
    assert_eq!(
        heard, 1,
        "the operator hears the home, naming the recoverer"
    );
    assert_one_recovery_into(&rig, tv, tally, (t, staff));
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None,
        "the completion took the epoch"
    );
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Out),
        "a foreign bay reads out (R3)"
    );
    assert_eq!(
        store(&rig.tw).tv_lease_on(tv),
        Some(&lease),
        "the lease is untouched"
    );
    after_the_recovery(&mut rig, (pilot, other), tv);
    no_refused_writes(&rig.tw);
}

/// **The rig's black box holds its open line and exactly one close**: `recovered_to_base` into
/// `base`, abandoned at `t`, recovered by `staff` (the fix round's R1: written at the completion,
/// into the hull the rig landed in).
fn assert_one_recovery_into(
    rig: &crate::excursions::Rig,
    tv: ObjId,
    base: ObjId,
    (t, staff): (i64, ObjId),
) {
    let all = lines(rig, tv);
    let closes: Vec<&BlackBoxLine> = all
        .iter()
        .filter(|line| matches!(line, BlackBoxLine::SortieClosed { .. }))
        .collect();
    assert_eq!((all.len(), closes.len()), (2, 1), "{all:?}");
    let BlackBoxLine::SortieClosed {
        close,
        abandoned,
        abandoned_ic,
        recovered_by,
        ..
    } = closes[0]
    else {
        panic!("the close line: {all:?}");
    };
    assert_eq!(*close, IcCloseKind::RecoveredToBase { base });
    assert_eq!(
        (*abandoned, *abandoned_ic, *recovered_by),
        (true, Some(t), Some(staff))
    );
}

/// **The fix round's R2 (a). A second-carrier home waits while its hull is gone, then completes**
/// (R1 (d)): ordered into the Tally Moth, which staff then take off the pad before any surface
/// fire (`mush_sim::unplace` through `keeping`) — the next fires tell the wait once, naming the
/// retry into the Tally Moth, and the rig's epoch is still open and abandoned, with no close line.
/// Staff place the Tally Moth back on Ahuna's domain (`staff_place_on`); the next fires place the
/// rig in its bay and the finish closes the epoch into the Tally Moth — the hull it landed in —
/// with exactly one `sortie_closed` line.
#[test]
fn s3_a_second_carrier_home_waits_while_its_hull_is_gone() {
    let mut rig = stocked();
    let (_, _, tv, staff, t, (_, tally_row)) = abandoned_and_carrier_gone(&mut rig);
    let tally = ObjId(tally_row.hull);
    home_into(&mut rig, staff, tv, tally);
    keeping(&mut rig, |ctx| {
        mush_sim::unplace(ctx, tally, "doc 23 cut S.3: the recovering hull leaves")
    })
    .expect("staff take the Tally Moth off the pad");
    let wait = format!(
        "Rig #{0} recovery is waiting: the recovering hull is no longer settled on the rig's \
         site. Staff can retry @tacsim/tv home #{0} #{1} or cancel with @tacsim/tv close #{0}.",
        tv.0, tally.0
    );
    assert_eq!(fire_and_count(&mut rig, 3, staff, &wait), 1, "told once");
    assert_eq!(
        abandonment(&mut rig, tv),
        Some((true, Some(t))),
        "the epoch is still open and abandoned"
    );
    assert_eq!(lines(&rig, tv).len(), 1, "no close line while it waits");
    assert!(!in_bay(&rig.tw, tv, &tally_row));

    let ahuna = mush_sim::DomainId(
        port_map(&rig.tw, "ahuna_port")
            .domain
            .expect("Ahuna's domain"),
    );
    staff_place_on(&mut rig, tally, ahuna, 0, 0);
    let _ = fire_and_count(&mut rig, 4, staff, &wait);
    assert!(
        in_bay(&rig.tw, tv, &tally_row),
        "the rig lands in the Tally Moth"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.sortie),
        None
    );
    assert_one_recovery_into(&rig, tv, tally, (t, staff));
    no_refused_writes(&rig.tw);
}

/// A staff home into `into` refused with `answer`: nothing queued, nothing written.
fn refused_home_into(
    rig: &mut crate::excursions::Rig,
    (staff, tv, into): (ObjId, ObjId, ObjId),
    answer: &str,
) {
    let before = record_of(&mut rig.tw, tv);
    let _ = rig.tw.take_module_ops();
    let (out, queued) = command(
        rig,
        staff,
        &format!("@tacsim/tv home #{} #{}", tv.0, into.0),
    );
    assert_eq!(out.trim(), answer);
    assert!(queued.is_empty(), "a refused home queues nothing");
    assert!(
        rig.tw.take_module_ops().is_empty(),
        "a refused home writes nothing"
    );
    assert_eq!(record_of(&mut rig.tw, tv), before);
}

/// **The fix round's R2 (b). A home into a full bay is refused** (R1 (c)): the Tally Moth settles
/// on Ahuna beside the Salt Moth with both its bay's rigs aboard, so a home of the Salt Moth's
/// abandoned rig into it answers "That hull's bay has no room for the rig." and writes nothing.
#[test]
fn s3_a_home_into_a_full_bay_is_refused() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let staff = recovery_staff(&mut rig.tw);
    recovery_order(&mut rig, staff, "close", tv);
    let (_, tally_row, theirs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 2);
    assert!(theirs.iter().all(|own| in_bay(&rig.tw, *own, &tally_row)));
    refused_home_into(
        &mut rig,
        (staff, tv, ObjId(tally_row.hull)),
        "That hull's bay has no room for the rig.",
    );
    no_refused_writes(&rig.tw);
}

/// **The fix round's R2 (b). A home into another hull needs the lease standing**: the rig's lease
/// is ended by a staff write to its stored document across a reboot (no verb ends a lease with
/// the rig out), so the rig's identity reads its last receipt and no lease names the hull; a home
/// into the Tally Moth answers "That rig's lease must stand to recover it into another hull." and
/// writes nothing.
#[test]
fn s3_a_home_into_another_hull_needs_its_lease() {
    let mut rig = stocked();
    let (pilot, _, tv, staff, _, (other, tally_row)) = abandoned_and_carrier_gone(&mut rig);
    let lease = store(&rig.tw).tv_lease_on(tv).cloned().expect("the lease");
    let mut rig = reboot_with(rig, &[pilot, other, staff]);
    let ended = ic_now(&rig);
    {
        let key = tacsim::port::tv::tv_lease_key(lease.lease);
        let data = rig.store.get_mut(tacsim::NS).expect("tacsim's documents");
        let (_, value) = data
            .kv
            .iter_mut()
            .find(|(at, _)| *at == key)
            .expect("the lease document");
        value["status"] = serde_json::json!("returned");
        value["ended_at_ic"] = serde_json::json!(ended);
    }
    let mut rig = reboot_with(rig, &[pilot, other, staff]);
    assert_eq!(store(&rig.tw).tv_lease_on(tv), None, "the lease ended");
    refused_home_into(
        &mut rig,
        (staff, tv, ObjId(tally_row.hull)),
        "That rig's lease must stand to recover it into another hull.",
    );
    no_refused_writes(&rig.tw);
}

/// **The fix round's R4. A replaced abandoning close still tells the owner once**: staff close
/// the rig out (abandoning its epoch) and close it again before any surface fire — the one
/// receipt the record keeps is the second, which carries the abandonment forward, so the next
/// fires tell the abandonment once and the plain close never.
#[test]
fn s3_a_replaced_abandoning_close_still_tells_the_owner() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let staff = recovery_staff(&mut rig.tw);
    recovery_order(&mut rig, staff, "close", tv);
    recovery_order(&mut rig, staff, "close", tv);
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.recovery)
            .is_some_and(|receipt| receipt.abandoned),
        "the second receipt carries the notice"
    );
    let mut abandoned = 0;
    let mut plain = 0;
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        let heard = said(&rig.tw, &events, pilot);
        abandoned += count(&heard, &abandoned_notice(tv));
        plain += count(&heard, &plain_close_notice(tv));
        land_all(&mut rig, &queued);
    }
    assert_eq!((abandoned, plain), (1, 0));
    no_refused_writes(&rig.tw);
}

/// Today's behaviour once another hull holds the rig (A4; doc 23 §8 defers it): the lessee's
/// launch is refused, and the recoverer's lift is filed (its guard walks its own leased rigs).
fn after_the_recovery(rig: &mut crate::excursions::Rig, (pilot, other): (ObjId, ObjId), tv: ObjId) {
    let before = record_of(&mut rig.tw, tv);
    let (out, queued) = command(rig, pilot, "+tv/1 launch go");
    assert!(
        queued.is_empty(),
        "the lessee's launch submits nothing: {out}"
    );
    assert_eq!(
        out.trim(),
        tacsim::port::tv::LAUNCH_NOT_SITE.replace("{name}", "Salt Moth"),
        "the lessee's hull is off the pad"
    );
    assert_eq!(record_of(&mut rig.tw, tv), before);
    let (out, queued) = command(rig, other, "+ship/lift ceres");
    assert!(
        out.contains("Clearance filed for "),
        "the lift is not held: {out}"
    );
    assert_eq!(queued.len(), 1, "one burn: {out}");
}

/// **S.3 test 5. A home into another hull is refused unless the rig is abandoned and the hull is
/// settled on its site** (A4): `home #<rig> #<hull>` on a rig whose epoch is open but not
/// abandoned answers "That rig is not abandoned; home it into its own hull."; once abandoned,
/// naming a Dray-P docked at Ceres, or an object that is no carrier, answers "That hull is not
/// settled on the rig's site with a readable bay." Each refusal queues nothing and writes
/// nothing.
#[test]
fn s3_a_home_into_another_hull_is_refused_off_the_rule() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (_, ceres_row) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let at_ceres = ObjId(ceres_row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let staff = recovery_staff(&mut rig.tw);
    let refuse = |rig: &mut crate::excursions::Rig, into: ObjId, answer: &str| {
        let before = record_of(&mut rig.tw, tv);
        let _ = rig.tw.take_module_ops();
        let (out, queued) = command(
            rig,
            staff,
            &format!("@tacsim/tv home #{} #{}", tv.0, into.0),
        );
        assert_eq!(out.trim(), answer);
        assert!(queued.is_empty(), "a refused home queues nothing");
        assert!(
            rig.tw.take_module_ops().is_empty(),
            "a refused home writes nothing"
        );
        assert_eq!(record_of(&mut rig.tw, tv), before);
    };
    refuse(
        &mut rig,
        at_ceres,
        "That rig is not abandoned; home it into its own hull.",
    );
    recovery_order(&mut rig, staff, "close", tv);
    for into in [at_ceres, pilot] {
        refuse(
            &mut rig,
            into,
            "That hull is not settled on the rig's site with a readable bay.",
        );
    }
    assert!(!in_bay(&rig.tw, tv, &row));
    assert_eq!(lines(&rig, tv).len(), 1);
    no_refused_writes(&rig.tw);
}

/// **S.3 test 6. A ship placed into a hangar port is recovered** (A5; §2 "Recovery"): the Salt
/// Moth flew Ceres to Ahuna Flank under an open epoch and is settled there; staff place it into
/// Stickney's berth — `mush_sim::place` `Direct` on Stickney's domain through `keeping`
/// (`staff_place_on`), the write the arrival itself makes, so `berth_of` reads `Docked` at
/// Stickney, a hangar port. Its own journey (to Ahuna) never arrived at Stickney, so the next
/// port fire closes the epoch `recovered_to_base` into Stickney's host room at the IC now, naming
/// no recoverer; the journey book is untouched. (A passage that docks by its own drive still
/// closes `returned_to_base`: S.2's `s2_ceres_to_stickney_opens_and_closes`, unchanged.)
#[test]
fn s3_a_ship_placed_into_a_hangar_port_is_recovered() {
    let mut rig = stocked();
    let (_, row, _) = on_pad(&mut rig, 0);
    let hull = ObjId(row.hull);
    let ship = ship_epoch(&mut rig, hull).expect("the Salt Moth's epoch is open on the pad");
    let journey = |rig: &crate::excursions::Rig| {
        rig.tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull).cloned())
    };
    let flown = journey(&rig).expect("the journey to Ahuna");
    let stickney = port_map(&rig.tw, "stickney_port");
    let domain = mush_sim::DomainId(stickney.domain.expect("Stickney's domain"));
    let host = ObjId(stickney.host.expect("Stickney's host room"));
    staff_place_on(&mut rig, hull, domain, 0, 0);
    assert!(
        tacsim::port::ship::hangar_port_of(rig.tw.world(), hull).is_some(),
        "docked at Stickney, a hangar port"
    );
    let pages = tacsim::port::epoch_pages(rig.tw.world());
    let mut closed_at = None;
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        let epoch = queued
            .iter()
            .any(|submit| submit.action.key().id.as_str() == tacsim::port::EPOCH_ACTION);
        land_all(&mut rig, &queued);
        if epoch {
            closed_at = Some(ic_now(&rig));
            break;
        }
    }
    let at = closed_at.expect("the next port fire closes the epoch");
    assert_eq!(ship_epoch(&mut rig, hull), None, "the epoch is closed");
    let both = lines(&rig, hull);
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(
        both[1],
        BlackBoxLine::SortieClosed {
            unit: hull,
            sortie: ship.id,
            at_ic: at,
            close: IcCloseKind::RecoveredToBase { base: host },
            abandoned: false,
            abandoned_ic: None,
            recovered_by: None,
            unrecorded: false,
        }
    );
    assert_eq!(journey(&rig), Some(flown), "the journey book is untouched");
    assert_eq!(
        tacsim::port::epoch_pages(rig.tw.world()),
        pages + 1,
        "the recovered close is paged once (the fix round's R7)"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Doc 17h cut M8a — the box and the inspection
// ---------------------------------------------------------------------------

/// A hull's name, as the lines print it.
pub(crate) fn hull_name(rig: &crate::excursions::Rig, hull: ObjId) -> String {
    rig.tw
        .world()
        .get(hull)
        .map(|o| o.name.clone())
        .expect("the hull")
}

/// **M8a 2 and 3. The owner reads the box; nobody else does** (doc 17h cut M8a, A3, A7; the
/// brief's tests 2 and 3; `sim_lenses/observer_rule.rs`'s shape, here because the mushd lens
/// harness has no Dray-P on a site's pad with a leased rig — that composition is this file's):
/// after launch and finish the operator's `+hull/box #tv` prints the one open line in A3's
/// shape, the IC date by the same formatter; after `+tv/recover`'s close, two lines; `+hull/box
/// #tv 1` the last only; the same after a restart. The positive control, then the absences: a
/// second operator on the site (with their own leased rig: a pilot too), a bystander and a
/// pilot with no lease each get the one sentence and nothing else, as does a number nobody
/// holds and a count outside 1..=32; staff read it whole. The word "sortie" reaches no line.
#[test]
fn m8a_the_owner_reads_the_box_and_nobody_else() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let no_box = tacsim::surface::hull::NO_BOX;
    // No trip yet: no box to read, even for its owner.
    let (out, queued) = command(&mut rig, pilot, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, no_box);
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let out_ic = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    let seal = open.seal.clone().expect("the rig has a seal");
    // The fix round's F1: a trip's open is two rows, the second indented two spaces.
    let open_line = format!(
        "{} out · surface from the bay of {}\n  Standard Doctrine · the ROM · governed · licence \
         doctrine · seal {}",
        tacsim::port::licence::ic_stamp(out_ic),
        hull_name(&rig, hull),
        &seal.seal_digest[..8]
    );
    let (out, _) = command(&mut rig, pilot, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, open_line);
    assert!(!out.contains("sortie"), "{out}");
    // The Dray-P's own box (S.2: one open port line) is not the pilot's to read: the
    // programme hull's title is the Trust's and the pilot its charter tenant, neither a
    // title-holding character nor a lessee (the readers of doc 17h §2.5); staff read it.
    let (out, _) = command(&mut rig, pilot, &format!("+hull/box #{}", hull.0));
    assert_eq!(out, no_box);
    // The ship's open line: its port epoch opened under the ROM's seal (S.2: the line names
    // `doctrine`, `s2_a_ship_leaves_and_reaches_a_port`), so it is two rows, the second the
    // seal's (the ship/port-epoch branch the inspection reads too).
    let ship_seal = ship_epoch(&mut rig, hull)
        .and_then(|open| open.seal)
        .expect("the Dray-P's port epoch is sealed");
    let staff = recovery_staff(&mut rig.tw);
    let (out, _) = command(&mut rig, staff, &format!("+hull/box #{}", hull.0));
    let rows: Vec<&str> = out.lines().collect();
    assert_eq!(rows.len(), 2, "{out}");
    assert!(rows[0].contains(" out · port from "), "{out}");
    assert!(
        rows[1].starts_with("  Standard Doctrine · ")
            && rows[1].ends_with(&format!(
                " · licence doctrine · seal {}",
                &ship_seal.seal_digest[..8]
            )),
        "{out}"
    );

    // The absences (A7): one sentence each, nothing else.
    let (other, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let bystander = rig.tw.spawn_player("Bystander");
    let stranger = rig.tw.spawn_player("Pilot with no lease");
    for who in [other, bystander, stranger] {
        let (out, queued) = command(&mut rig, who, &format!("+hull/box #{}", tv.0));
        assert_eq!(out, no_box, "#{}", who.0);
        assert!(queued.is_empty());
    }
    for line in [
        "+hull/box #999999",
        &format!("+hull/box #{} 0", tv.0),
        &format!("+hull/box #{} 33", tv.0),
        &format!("+hull/box #{} 1 more", tv.0),
    ] {
        let (out, _) = command(&mut rig, pilot, line);
        assert_eq!(out, no_box, "{line}");
    }
    let (out, _) = command(&mut rig, pilot, "+hull/box");
    assert_eq!(out, "Usage: +hull/box #<hull> [n]");
    let (out, _) = command(&mut rig, pilot, &format!("+hull #{}", tv.0));
    assert_eq!(out, "Usage: +hull/box #<hull> [n]");

    // The recover closes the trip: two lines, then the last alone.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let in_ic = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    let close_line = format!("{} in · returned", tacsim::port::licence::ic_stamp(in_ic));
    let both = format!("{open_line}\n{close_line}");
    let (out, _) = command(&mut rig, pilot, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, both);
    let (out, _) = command(&mut rig, pilot, &format!("+hull/box #{} 1", tv.0));
    assert_eq!(out, close_line);
    // Staff read it whole.
    let (out, _) = command(&mut rig, staff, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, both);

    // A restart: the same.
    let mut rig = reboot_with(rig, &[pilot, other]);
    let (out, _) = command(&mut rig, pilot, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, both, "the box reads the same after a restart");
    let (out, _) = command(&mut rig, other, &format!("+hull/box #{}", tv.0));
    assert_eq!(out, no_box);
    no_refused_writes(&rig.tw);
}

/// **M8a fix round 1, F7 (D-UM-42): a lessee reads only the lines since their lease began.**
/// The first lessee's trip (open and close, three rows) is theirs to read while the rig is
/// theirs; once they hand it back they read nothing. A second lessee of the same rig reads
/// nothing of the first's trip — the hull has no box they can read until their own trip opens,
/// and then only its rows. Staff read every line; and once staff put the rig's title on a
/// character (F10, the title-holder arm), that holder reads every line too, both lessees'.
#[test]
#[allow(clippy::too_many_lines)]
fn m8a_a_lessee_reads_only_the_lines_since_their_lease() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let no_box = tacsim::surface::hull::NO_BOX;
    let read = |rig: &mut crate::excursions::Rig, who: ObjId| {
        command(rig, who, &format!("+hull/box #{}", tv.0)).0
    };
    // The first lessee's trip, out and in.
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    let first = read(&mut rig, pilot);
    assert_eq!(first.lines().count(), 3, "{first}");
    let first_open = first.lines().next().expect("the open row").to_owned();

    // Hand it back at Occator's counter (staff carry the hull home, as the S.3 recovery test
    // does, so the counter's docked-hull rule holds).
    let ceres = mush_sim::DomainId(
        port_map(&rig.tw, "ceres_port")
            .domain
            .expect("Ceres domain"),
    );
    staff_place_on(&mut rig, hull, ceres, 0, 0);
    keeping(&mut rig, |ctx| {
        mush_sim::unplace(ctx, pilot, "the lessee walks to the lease counter")
    })
    .expect("the pilot leaves the hull");
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 lease return go");
    land_all(&mut rig, &queued);
    assert!(store(&rig.tw).tv_lease_on(tv).is_none(), "returned: {out}");
    assert_eq!(read(&mut rig, pilot), no_box, "a lessee no more");

    // The second lessee leases the same rig: nothing of the first's trip.
    let (second, _, tvs) = on_pad_as(&mut rig, "Second", 2, "Tally Moth", 1);
    assert_eq!(tvs, vec![tv], "the pool leases the returned rig again");
    assert_eq!(
        read(&mut rig, second),
        no_box,
        "no line since the lease began"
    );
    let _ = act_go(&mut rig, second, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, second);
    let theirs = read(&mut rig, second);
    assert_eq!(theirs.lines().count(), 2, "their own open alone: {theirs}");
    assert!(!theirs.contains(&first_open), "{theirs}");
    assert!(!theirs.contains("sealed in the box"), "{theirs}");

    // Staff read every line: the first trip's three rows and the second's open.
    let staff = recovery_staff(&mut rig.tw);
    let all = read(&mut rig, staff);
    assert_eq!(all.lines().count(), 5, "{all}");
    assert!(all.starts_with(&first_open), "{all}");

    // The title-holder arm: staff release the rig's claims (a lease's lien rides on it, and a
    // title with a claim on it does not move) and put its title on the first pilot, who then
    // reads it all — both lessees' lines.
    let claims = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(tv).map(|a| a.encumbrances.len()))
        .expect("the rig is titled");
    for n in (1..=claims).rev() {
        god(&mut rig, &format!("@asset/release #{}={n}", tv.0));
    }
    god(&mut rig, &format!("@asset/transfer #{}=#{}", tv.0, pilot.0));
    assert_eq!(
        read(&mut rig, pilot),
        all,
        "the title holder reads every line"
    );
    no_refused_writes(&rig.tw);
}

/// **M8a 4. The Registry inspects a rig on the pad and in the field** (doc 17h cut M8a, A4,
/// A6, A7, A8; the brief's test 4), composed here (`reboot_with_inspection`: the Registry
/// authority with an operator character, the Roll's setup acknowledged, the inspection over the
/// authority; the mushd lens harness cannot stand a surface rig up): a non-operator is told the
/// revocation's one sentence and nothing else, before any hull is looked at; a number no hull
/// row holds is `No such hull.`; on the pad the operator's `@registry/inspect #tv` prints only
/// `Inspecting hull #n.` and stages one action whose landing writes one
/// `roll_registry_inspections` record (`core: None`, `Zero`, `governed: None`, `Unknown`,
/// `NoSeal`; no seal, so no licence state) and whose receipt — the line the driver prints in
/// place of `Done.`, rendered from that record (the fix round's F3) — reads the ROM,
/// provenance unknown, no trip open; in the field the receipt reads the open trip's seal
/// matching, and the second record carries the digest, `Matches` and `Some(Doctrine)`; both
/// records survive a restart, and a third inspection writes a third. The fix round's F2: the
/// typed action refuses an object with a unit that is no programme hull, with the provider's
/// one sentence and no record, as the command says `No such hull.` of it.
#[test]
#[allow(clippy::too_many_lines)]
fn m8a_the_registry_inspects_a_rig_on_the_pad_and_in_the_field() {
    use tacsim::minds::licence_state::SealLicenceState;
    use tacsim::minds::registry_inspection::{HullClass, InspectionFacts, Provenance, SealCheck};

    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let ship = ObjId(row.hull);
    let tv = tvs[0];
    let n = tv.0;
    assert_eq!(policy.operators(), [operator]);

    // Not an operator: the driver's sentence, before any hull is looked at (A7).
    let bystander = rig.tw.spawn_player("Bystander");
    for (who, line) in [
        (pilot, format!("@registry/inspect #{n}")),
        (pilot, "@registry/inspect #999999".to_owned()),
        (bystander, format!("@registry/inspect #{n}")),
    ] {
        let (out, queued) = command(&mut rig, who, &line);
        assert_eq!(
            out, "That did not go through. Nothing changed.",
            "{line} by #{}",
            who.0
        );
        assert!(queued.is_empty());
    }
    // A hull outside the rows: unknown. A malformed number: the help.
    let (out, queued) = command(&mut rig, operator, "@registry/inspect #999999");
    assert_eq!(out, "No such hull.");
    assert!(queued.is_empty());
    let (out, _) = command(&mut rig, operator, "@registry/inspect salt");
    assert!(out.starts_with("@registry/rate"), "{out}");
    assert!(out.contains("@registry/inspect <hull>"), "{out}");

    // On the pad: no trip open; one record. The command says only that it is inspecting
    // (the second line is the engine's receipt of the submission, `Queued.`); the facts are
    // the provider's receipt, rendered from the record it wrote (F3).
    let (out, queued) = command(&mut rig, operator, &format!("@registry/inspect #{n}"));
    assert_eq!(out, format!("Inspecting hull #{n}.\nQueued."));
    assert_eq!(queued.len(), 1, "one submission: {out}");
    assert_eq!(queued[0].actor, operator);
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::minds::registry_inspection::INSPECTION_ACTION
    );
    let receipt = land_receipt(&mut rig, &queued[0]).expect("the inspection lands");
    assert_eq!(
        receipt.as_deref(),
        Some(format!("Hull #{n}: the ROM · provenance unknown · no trip open").as_str())
    );
    let records = inspections(&rig.tw);
    assert_eq!(records.len(), 1, "{records:?}");
    let first = &records[0];
    // Doc 17h cut M8d K1: the record carries the hull's title holder at the inspection.
    let first_holder = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(tv).map(|a| a.title_owner.clone()));
    assert!(first_holder.is_some(), "the rig is titled");
    assert_eq!(first.operator, operator);
    assert_eq!(first.authority, policy);
    assert_eq!(
        first.facts(),
        InspectionFacts {
            hull: tv,
            core: None,
            class: HullClass::Zero,
            governed: None,
            provenance: Provenance::Unknown,
            seal_digest: None,
            seal_check: SealCheck::NoSeal,
            licence_state: None,
            ungoverned_note: None,
            holder: first_holder.clone(),
        }
    );

    // In the field: the open trip's seal, matching the hardware; a second record.
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let out_ic = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    let seal = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .and_then(|open| open.seal)
        .expect("the trip is sealed");
    let (out, queued) = command(&mut rig, operator, &format!("@registry/inspect #{n}"));
    assert_eq!(out, format!("Inspecting hull #{n}.\nQueued."));
    assert_eq!(queued.len(), 1);
    let receipt = land_receipt(&mut rig, &queued[0])
        .expect("the inspection lands")
        .expect("a receipt");
    assert_eq!(
        receipt,
        format!(
            "Hull #{n}: the ROM · provenance unknown · seal {} matches",
            &seal.seal_digest[..8]
        )
    );
    assert!(
        !receipt.contains("sortie") && receipt.chars().count() <= 120,
        "{receipt}"
    );
    // The trip's own words stay in the box (the receipt is one line): its out-since stamp.
    let (boxed, _) = command(&mut rig, pilot, &format!("+hull/box #{n}"));
    assert!(
        boxed.starts_with(&format!(
            "{} out · ",
            tacsim::port::licence::ic_stamp(out_ic)
        )),
        "{boxed}"
    );
    let records = inspections(&rig.tw);
    assert_eq!(records.len(), 2, "{records:?}");
    let second = records
        .iter()
        .find(|record| record.inspection != first.inspection)
        .expect("the second record");
    assert_eq!(
        second.facts(),
        InspectionFacts {
            hull: tv,
            core: None,
            class: HullClass::Zero,
            governed: None,
            provenance: Provenance::Unknown,
            seal_digest: Some(seal.seal_digest.clone()),
            seal_check: SealCheck::Matches,
            licence_state: Some(SealLicenceState::Doctrine),
            ungoverned_note: None,
            holder: first_holder.clone(),
        }
    );

    // F10: the ship's branch of the open trip — the Dray-P's own open port epoch. The
    // receipt and the record carry that epoch's seal, checked against the ship's hardware.
    let ship_open = ship_epoch(&mut rig, ship).expect("the Dray-P's epoch is open on the pad");
    let (out, queued) = command(
        &mut rig,
        operator,
        &format!("@registry/inspect #{}", ship.0),
    );
    assert_eq!(
        out,
        format!(
            "Inspecting hull #{}.
Queued.",
            ship.0
        )
    );
    let receipt = land_receipt(&mut rig, &queued[0])
        .expect("the ship's inspection lands")
        .expect("a receipt");
    assert!(
        receipt.starts_with(&format!("Hull #{}: ", ship.0)),
        "{receipt}"
    );
    let shipped = inspections(&rig.tw)
        .into_iter()
        .find(|record| record.hull == ship)
        .expect("the ship's record");
    // The Dray-P's port epoch is sealed (the box test above pins it): the record carries that
    // seal's digest and its doctrine state, checked against the ship's hardware unchanged
    // since the seal, and the receipt ends with the match.
    let seal = ship_open
        .seal
        .as_ref()
        .expect("the Dray-P's port epoch is sealed");
    assert_eq!(
        shipped.seal_digest.as_deref(),
        Some(seal.seal_digest.as_str())
    );
    assert_eq!(shipped.seal_check, SealCheck::Matches);
    assert_eq!(shipped.licence_state, Some(SealLicenceState::Doctrine));
    assert!(
        receipt.ends_with(&format!(" · seal {} matches", &seal.seal_digest[..8])),
        "{receipt}"
    );
    let records: Vec<_> = inspections(&rig.tw)
        .into_iter()
        .filter(|record| record.hull == tv)
        .collect();

    // A restart keeps both; a third inspection writes a third.
    let mut rig = reboot_with(rig, &[operator, pilot]);
    let mut kept: Vec<_> = inspections(&rig.tw)
        .into_iter()
        .filter(|record| record.hull == tv)
        .collect();
    kept.sort_by_key(|r| r.inspection.to_hex());
    let mut before = records.clone();
    before.sort_by_key(|r| r.inspection.to_hex());
    assert_eq!(kept, before, "the records survive a restart");
    let (_, queued) = command(&mut rig, operator, &format!("@registry/inspect #{n}"));
    land_all(&mut rig, &queued);
    assert_eq!(
        inspections(&rig.tw)
            .iter()
            .filter(|record| record.hull == tv)
            .count(),
        3
    );
    // F9: the rig's three records hold three slots of its ring, `seq` 1..=3.
    let mut seqs: Vec<u64> = inspections(&rig.tw)
        .iter()
        .filter(|record| record.hull == tv)
        .map(|record| record.seq)
        .collect();
    seqs.sort_unstable();
    assert_eq!(seqs, vec![1, 2, 3]);
    let (out, queued) = command(&mut rig, pilot, &format!("@registry/inspect #{n}"));
    assert_eq!(out, "That did not go through. Nothing changed.");
    assert!(queued.is_empty());

    // F2: an object with a tactical unit that is no programme hull (a copy of the rig's unit
    // on a fresh Thing, in the unit store's RAM). The command says `No such hull.` and stages
    // nothing; the typed action, the same request by another door, is refused by the
    // provider's own read — its one sentence, no record, no receipt (F3: a refused prepare
    // prints only the refusal).
    let stray = rig
        .tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Stray chassis",
            mush_core::GOD,
            Some(mush_core::LIMBO),
        )
        .expect("a thing");
    let unit = mush_tactics::module::unit_of(rig.tw.world(), tv)
        .cloned()
        .expect("the rig's unit");
    rig.tw
        .world_mut()
        .module_state_mut::<mush_tactics::module::UnitStore>()
        .expect("the tactics store")
        .units
        .insert(stray, unit);
    let (out, queued) = command(
        &mut rig,
        operator,
        &format!("@registry/inspect #{}", stray.0),
    );
    assert_eq!(out, "No such hull.");
    assert!(queued.is_empty());
    let action = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .expect("the inspection is composed")
        .writer()
        .action()
        .clone();
    let refused = land_receipt(
        &mut rig,
        &mush_core::AuthoritativeSubmit {
            actor: operator,
            action,
            subject: mush_core::PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: serde_json::json!({"hull": stray.0}),
            key: mush_core::IdempotencyKeyText::parse_canonical("m8a.inspect.stray")
                .expect("a key"),
        },
    );
    let refusal = refused.expect_err("the provider refuses a unit that is no hull");
    assert!(
        refusal.contains("The Registry inspection is unavailable or stale."),
        "{refusal}"
    );
    assert_eq!(inspections(&rig.tw).len(), 4, "nothing was recorded");
    no_refused_writes(&rig.tw);
}

/// **M8d D6 wiring** (doc 17h cut M8d D6; D-UM-42): the unwarranted chronicle reads every
/// landed inspection through the composed observer and tick. A rig with no core reads no
/// finding: the hull's cursor moves to the inspection's sequence, no `minds.unwarranted` line
/// reaches the chronicle, and the cursor survives a restart (the boot pass re-reads the ring
/// and finds nothing new; K7: it adds no row for an uninspected hull). The ungoverned half runs
/// in `m8d_the_registry_fines_the_holder_the_inspection_recorded` over a seeded finding; the
/// fact's resolve is pinned by mush-roll's facts tests (scoped, K6).
#[test]
fn m8d_the_unwarranted_chronicle_reads_each_inspection_once() {
    use tacsim::minds::registry_inspection::unwarranted;

    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, _policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (pilot, _row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (_, queued) = command(&mut rig, operator, &format!("@registry/inspect #{}", tv.0));
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    // The world task's commit pipeline announces every installed family
    // (`install_composed_action_announcing`); this harness installs without announcing, so the
    // landed slot is announced here exactly as the pipeline would.
    {
        let family = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
            .expect("the inspection is composed")
            .writer()
            .family()
            .expect("the record family");
        let engine = rig.tw.engine_mut();
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        engine.emit_world_event(
            &mush_core::WorldEvent::ActionStateChanged {
                owner: family.owner().clone(),
                family: family.descriptor().id().clone(),
                keys: vec![mush_roll::commerce::registry_inspection::inspection_key(tv, 1).into()],
            },
            &mut out,
            &mut fx,
        );
    }
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let chronicle = unwarranted::current(rig.tw.world())
        .cloned()
        .expect("the chronicle is written once an inspection is read");
    let row = chronicle.hulls.get(&tv.0).expect("the rig's row");
    assert_eq!(row.cursor, 1, "the first inspection was read");
    assert!(row.finds.is_empty(), "a coreless rig is no finding");
    let canon = rig
        .tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|state| state.recent())
        .unwrap_or_default();
    assert!(
        !canon.iter().any(|event| event.kind == unwarranted::KIND),
        "nothing unwarranted was chronicled"
    );
    let rig = reboot_with(rig, &[operator, pilot]);
    let mut rig = rig;
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        unwarranted::current(rig.tw.world()),
        Some(&chronicle),
        "the cursor survives a restart and the boot pass finds nothing new"
    );
    no_refused_writes(&rig.tw);
}

/// **A real ungoverned rig is a finding** (doc 17h cut M8d D6; found by M8g G4's leak battery):
/// a leased rig bound to M8b's ungoverned Class I root, inspected on the pad, records
/// `governed: false` with no core asset (the fitted root's component names none), and that is
/// the unwarranted finding every consequence reads — the chronicle commits its one canon line,
/// naming nobody. The seeded findings of the tests above carry a core asset; this one is the
/// world's own record.
#[test]
fn m8d_a_real_ungoverned_rig_is_a_finding() {
    use tacsim::definitions::core_replacement::RIG_I_UNGOVERNED;
    use tacsim::minds::registry_inspection::unwarranted;

    let mut rig = crate::excursions::stocked_fitted();
    let operator = rig.tw.spawn_player("Registry operator");
    let (pilot, _row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (mut rig, _policy) = reboot_with_inspection(rig, operator, &[operator, pilot]);
    fit(&mut rig, tv, RIG_I_UNGOVERNED);
    let seq = inspect(&mut rig, operator, tv);
    let records = inspections(&rig.tw);
    let record = records
        .iter()
        .find(|record| record.hull == tv && record.seq == seq)
        .expect("the record");
    assert_eq!(record.governed, Some(false));
    assert_eq!(
        record.core, None,
        "the fitted root's component names no asset"
    );
    assert!(
        unwarranted::unwarranted(record),
        "the world's own record is a finding"
    );
    let family = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .expect("the inspection is composed")
        .writer()
        .family()
        .expect("the record family");
    announce(
        &mut rig,
        &family,
        vec![mush_roll::commerce::registry_inspection::inspection_key(
            tv, seq,
        )],
    );
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let lines: Vec<_> = rig
        .tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|state| state.recent())
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == unwarranted::KIND)
        .collect();
    assert_eq!(lines.len(), 1, "one unwarranted line");
    assert_eq!(lines[0].summary, unwarranted::LINE);
    assert!(lines[0].entities.is_empty(), "naming nobody");
    no_refused_writes(&rig.tw);
}

/// Announce a landed typed write as the world task's commit pipeline does
/// (`install_composed_action_announcing`); this harness installs without announcing.
pub(crate) fn announce<K, V>(
    rig: &mut crate::excursions::Rig,
    family: &mush_core::StateFamilyHandle<K, V>,
    keys: Vec<String>,
) where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let engine = rig.tw.engine_mut();
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    engine.emit_world_event(
        &mush_core::WorldEvent::ActionStateChanged {
            owner: family.owner().clone(),
            family: family.descriptor().id().clone(),
            keys: keys.into_iter().map(Into::into).collect(),
        },
        &mut out,
        &mut fx,
    );
}

/// The economy's live title holder of `hull`.
fn title_of(rig: &crate::excursions::Rig, hull: ObjId) -> Option<mush_org::PrincipalId> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .asset(hull)
                .map(|a| a.title_owner.clone())
        })
}

/// Staff release `hull`'s claims and title it to `to` (`#<n>`, or `org:<name> scope=<n>`).
fn retitle(rig: &mut crate::excursions::Rig, hull: ObjId, to: &str) {
    let claims = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(hull).map(|a| a.encumbrances.len()))
        .expect("the rig is titled");
    for k in (1..=claims).rev() {
        god(rig, &format!("@asset/release #{}={k}", hull.0));
    }
    god(rig, &format!("@asset/transfer #{}={to}", hull.0));
}

/// One real inspection of `hull` by `operator`, landed and announced; its sequence.
fn inspect(rig: &mut crate::excursions::Rig, operator: ObjId, hull: ObjId) -> u64 {
    let (_, queued) = command(rig, operator, &format!("@registry/inspect #{}", hull.0));
    assert_eq!(queued.len(), 1, "the inspection stages");
    land_all(rig, &queued);
    inspections(&rig.tw)
        .iter()
        .filter(|record| record.hull == hull)
        .map(|record| record.seq)
        .max()
        .expect("the inspection landed")
}

/// Seed the record an inspection of an ungoverned Class I rig writes, over the real record at
/// `seq` (`Engine::action_transaction`, the bootstrapping seam; no rig composes M8b's back room
/// with the inspection), keeping its recorded holder, and announce it.
fn seed_ungoverned(rig: &mut crate::excursions::Rig, tv: ObjId, seq: u64) {
    let host = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .expect("the inspection is composed");
    let family = host.writer().family().expect("the record family");
    let slot = mush_roll::commerce::registry_inspection::inspection_key(tv, seq);
    let core = rig
        .tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Ungoverned core",
            mush_core::GOD,
            Some(mush_core::LIMBO),
        )
        .expect("a thing");
    {
        let engine = rig.tw.engine_mut();
        let mut record = engine
            .action_state()
            .get(&family, &slot)
            .expect("the family reads")
            .cloned()
            .expect("the real inspection");
        assert_eq!(record.seq, seq);
        record.core = Some(core);
        record.class = tacsim::minds::registry_inspection::HullClass::I;
        record.governed = Some(false);
        let mut txn = engine.action_transaction(&family).expect("a transaction");
        txn.replace(&family, &slot, record)
            .expect("the seeded finding");
        let (_, installation) = txn.prepare().expect("it prepares").into_parts();
        engine
            .install_prepared_action(installation)
            .expect("it installs");
    }
    announce(rig, &family, vec![slot]);
}

/// The `Sanction` obligations the economy holds.
fn sanctions(rig: &crate::excursions::Rig) -> Vec<mush_economy::Obligation> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy")
        .snapshot()
        .obligations()
        .filter(|o| matches!(o.cause, mush_economy::ObligationCause::Sanction { .. }))
        .cloned()
        .collect()
}

/// **M8d D3 with fix round K1, K8 and K10: the Registry fines the holder the inspection
/// recorded** (the lead's rulings). The pilot holds the rig when it is inspected; the record
/// carries the pilot as holder:
/// - the fine needs its own grant; a coreless inspection is `NOTHING_TO_FINE`, an absent one
///   `NO_SUCH_INSPECTION`, an amount over the cap `FINE_RANGE`;
/// - the ungoverned finding is fined: `FINING`, the receipt `FINED` (K8's wording), and a second
///   fine of it `ALREADY_FINED`;
/// - **K1**: the title then moves to another player, and the levy still debits the pilot's own
///   account; and a pilot with no account would carry the debt (the opened account);
/// - **restart**: the fine is not announced before a reboot; the boot pass re-notes its slot and
///   the levy makes exactly one `Sanction`, due once, and a further fire none;
/// - the chronicle reads the finding: one `minds.unwarranted` line naming nobody.
#[test]
#[allow(clippy::too_many_lines)]
fn m8d_the_registry_fines_the_holder_the_inspection_recorded() {
    use tacsim::minds::registry_inspection::unwarranted;

    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (pilot, _row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let n = tv.0;
    retitle(&mut rig, tv, &format!("#{}", pilot.0));
    let first = inspect(&mut rig, operator, tv);
    assert_eq!(
        inspections(&rig.tw)
            .iter()
            .find(|r| r.hull == tv && r.seq == first)
            .and_then(|r| r.holder.clone()),
        Some(mush_org::PrincipalId::Object(pilot)),
        "K1: the record carries the holder at the inspection"
    );
    let (out, queued) = command(&mut rig, operator, &format!("@registry/fine #{n} 10"));
    assert_eq!(out, "That did not go through. Nothing changed.");
    assert!(queued.is_empty());
    god(
        &mut rig,
        &format!(
            "@org/grant #{}=Registry roll.registry.fine scope=#{}",
            operator.0,
            policy.scope().0
        ),
    );
    for (line, said) in [
        (
            format!("@registry/fine #{n} 10"),
            "That inspection found nothing to fine.",
        ),
        (format!("@registry/fine #{n}/9 10"), "No such inspection."),
        (
            format!("@registry/fine #{n} 1000.01"),
            "A fine is from 0.01cr to 1000cr.",
        ),
    ] {
        let (out, queued) = command(&mut rig, operator, &line);
        assert_eq!(out, said, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    seed_ungoverned(&mut rig, tv, first);
    let (out, queued) = command(&mut rig, operator, &format!("@registry/fine #{n} 12.50"));
    assert_eq!(
        out,
        format!("Fining the holder of hull #{n} 12.50cr.\nQueued.")
    );
    let receipt = land_receipt(&mut rig, &queued[0]).expect("the fine lands");
    assert_eq!(
        receipt.as_deref(),
        Some(
            format!(
                "Fined the holder of hull #{n} 12.50cr. The fine is levied when the Registry \
                 next settles."
            )
            .as_str()
        )
    );
    let (out, queued) = command(&mut rig, operator, &format!("@registry/fine #{n} 5"));
    assert_eq!(out, "That inspection has already been fined.");
    assert!(queued.is_empty());

    // The chronicle reads the finding (the inspection slot was announced with the seed).
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let lines: Vec<_> = rig
        .tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|state| state.recent())
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == unwarranted::KIND)
        .collect();
    assert_eq!(lines.len(), 1, "one unwarranted line");
    assert_eq!(lines[0].summary, unwarranted::LINE);
    assert!(lines[0].entities.is_empty());
    assert!(sanctions(&rig).is_empty(), "the fine is not yet levied");

    // K1: the title moves after the inspection; the fine still follows the pilot.
    let other = rig.tw.spawn_player("Other");
    retitle(&mut rig, tv, &format!("#{}", other.0));
    assert_eq!(
        title_of(&rig, tv),
        Some(mush_org::PrincipalId::Object(other))
    );

    // A restart before the levy: the boot pass re-notes the fine's slot.
    let mut rig = reboot_with(rig, &[operator, pilot]);
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let levied = sanctions(&rig);
    assert_eq!(levied.len(), 1, "one fine, levied once, after the restart");
    let o = &levied[0];
    assert_eq!(
        o.cause,
        mush_economy::ObligationCause::Sanction {
            inspection: tacsim::minds::registry_inspection::fine::sanction_key(tv, first)
        }
    );
    assert_eq!(o.debtor, mush_org::PrincipalId::Object(pilot), "K1");
    assert_eq!(o.amount.minor(), 1_250);
    assert_eq!(o.schedule.cadence, None, "due once");
    let debit = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(o.debit_account)
                .map(|a| a.owner.clone())
        });
    assert_eq!(debit, Some(mush_org::PrincipalId::Object(pilot)));
    no_refused_writes(&rig.tw);
}

/// **M8d D3, the organisation arm with K1 and K8**: the rig titled to an organisation when it is
/// inspected is fined from the organisation's treasury, the organisation the debtor; a levy
/// refused while the Registry is suspended is retried and lands once it is active again.
#[test]
fn m8d_an_organisations_fine_debits_its_treasury() {
    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (_pilot, _row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let n = tv.0;
    god(
        &mut rig,
        &format!(
            "@org/grant #{}=Registry roll.registry.fine scope=#{}",
            operator.0,
            policy.scope().0
        ),
    );
    god(&mut rig, "@org/create Salvage");
    god(&mut rig, "@account/open org:Salvage=tacsim.credit");
    let (org, root) = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| {
            s.state()
                .orgs()
                .find(|o| o.name.as_str() == "Salvage")
                .map(|o| (o.id, o.root_scope))
        })
        .expect("the organisation");
    retitle(&mut rig, tv, &format!("org:Salvage scope={}", root.0));
    let seq = inspect(&mut rig, operator, tv);
    seed_ungoverned(&mut rig, tv, seq);
    let (out, queued) = command(&mut rig, operator, &format!("@registry/fine #{n} 3"));
    assert_eq!(out, format!("Fining the holder of hull #{n} 3cr.\nQueued."));
    land_all(&mut rig, &queued);
    let fines = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .and_then(|host| host.fines().map(|f| f.family().clone()))
        .expect("the fine");
    announce(
        &mut rig,
        &fines,
        vec![mush_roll::commerce::registry_inspection::inspection_key(
            tv, seq,
        )],
    );
    // K8: the Registry suspended, the levier's grant does not hold — a retryable refusal: the
    // slot is re-noted, nothing is levied, and it is levied once the Registry is active again.
    god(&mut rig, "@org/status Registry=suspended");
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert!(
        sanctions(&rig).is_empty(),
        "not levied while the grant does not hold"
    );
    god(&mut rig, "@org/status Registry=active");
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let levied = sanctions(&rig);
    assert_eq!(levied.len(), 1);
    assert_eq!(levied[0].debtor, mush_org::PrincipalId::Organization(org));
    let treasury = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(levied[0].debit_account).cloned())
        .expect("the debited account");
    assert_eq!(treasury.owner, mush_org::PrincipalId::Organization(org));
    assert_eq!(treasury.scope, Some(root));
    assert_eq!(levied[0].amount.minor(), 300);
    no_refused_writes(&rig.tw);
}

/// **M8d D4 with fix round K2 and K4: `@registry/revoke principal=`** over seeded ungoverned
/// findings:
/// - the verb needs `roll.registry.revoke`; an unknown name is `NO_SUCH_PRINCIPAL`, a coreless
///   inspection `NOTHING_TO_REVOKE`;
/// - **K4**: only the holder the inspection recorded may be named — anyone else is the uniform
///   refusal;
/// - the pilot, the recorded holder, is revoked: `REVOKING_PRINCIPAL`, the receipt, a record
///   keyed `{:020}` barring them 90 IC days, and **K2**: the pilot hears "The Registry has barred
///   your mind licences until <date>." once, and the operator and a bystander hear nothing;
/// - the organisation the rig is titled to at a later inspection is revoked as `Salvage`: its
///   granted member hears the notice, a bystander does not;
/// - a name that is both a character and an organisation is refused;
/// - **restart**: both records survive a reboot.
#[test]
#[allow(clippy::too_many_lines)]
fn m8d_the_registry_revokes_the_inspected_holder_for_a_term() {
    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (pilot, _row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let n = tv.0;
    let bystander = rig.tw.spawn_player("Bystander");
    retitle(&mut rig, tv, &format!("#{}", pilot.0));
    let first = inspect(&mut rig, operator, tv);
    let line = format!("@registry/revoke principal=#{} #{n}", pilot.0);
    let (out, queued) = command(&mut rig, operator, &line);
    assert_eq!(out, "That did not go through. Nothing changed.");
    assert!(queued.is_empty());
    god(
        &mut rig,
        &format!(
            "@org/grant #{}=Registry roll.registry.revoke scope=#{}",
            operator.0,
            policy.scope().0
        ),
    );
    for (line, said) in [
        (
            format!("@registry/revoke principal=Nobody #{n}"),
            "No such character or organisation.",
        ),
        (line.clone(), "That inspection found nothing to revoke for."),
    ] {
        let (out, queued) = command(&mut rig, operator, &line);
        assert_eq!(out, said, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    seed_ungoverned(&mut rig, tv, first);
    // K4: a principal who did not hold the inspected hull.
    let (out, queued) = command(
        &mut rig,
        operator,
        &format!("@registry/revoke principal=#{} #{n}", bystander.0),
    );
    assert_eq!(out, "That did not go through. Nothing changed.");
    assert!(queued.is_empty());
    let pilot_name = rig
        .tw
        .world()
        .get(pilot)
        .map(|o| o.name.to_string())
        .expect("the pilot");
    let (out, queued) = command(&mut rig, operator, &line);
    assert_eq!(
        out,
        format!("Revoking {pilot_name} for 90 IC days.\nQueued.")
    );
    let receipt = land_receipt(&mut rig, &queued[0]).expect("the revocation lands");
    let family = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .expect("the inspection")
        .writer()
        .revoked_principals()
        .expect("the family");
    let pilot_key = format!("{:020}", pilot.0);
    let record = rig
        .tw
        .engine()
        .action_state()
        .get(&family, &pilot_key)
        .expect("the family reads")
        .cloned()
        .expect("the character's record");
    assert_eq!(record.principal, mush_org::PrincipalId::Object(pilot));
    assert_eq!(record.until_ic - record.from_ic, 90 * 86_400);
    assert_eq!(record.seq, first);
    assert!(record.bars(record.from_ic) && !record.bars(record.until_ic));
    let until = tacsim::port::licence::ic_stamp(record.until_ic);
    assert_eq!(
        receipt.as_deref(),
        Some(format!("{pilot_name} holds no new licence until {until}.").as_str())
    );
    // K2: the pilot is told once; nobody else.
    announce(&mut rig, &family, vec![pilot_key.clone()]);
    let notice = format!("The Registry has barred your mind licences until {until}.");
    let mut heard = (0, 0, 0);
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        heard.0 += count(&said(&rig.tw, &events, pilot), &notice);
        heard.1 += count(&said(&rig.tw, &events, operator), &notice);
        heard.2 += count(&said(&rig.tw, &events, bystander), &notice);
        land_all(&mut rig, &queued);
    }
    assert_eq!(heard, (1, 0, 0), "the principal alone, once");

    // An organisation, the holder at a later inspection.
    god(&mut rig, "@org/create Salvage");
    let (org, root) = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| {
            s.state()
                .orgs()
                .find(|o| o.name.as_str() == "Salvage")
                .map(|o| (o.id, o.root_scope))
        })
        .expect("the organisation");
    let member = rig.tw.spawn_player("Member");
    god(
        &mut rig,
        &format!("@org/grant #{}=Salvage roll.holder.org", member.0),
    );
    retitle(&mut rig, tv, &format!("org:Salvage scope={}", root.0));
    let second = inspect(&mut rig, operator, tv);
    seed_ungoverned(&mut rig, tv, second);
    let (out, queued) = command(
        &mut rig,
        operator,
        &format!("@registry/revoke principal=Salvage #{n}"),
    );
    assert_eq!(out, "Revoking Salvage for 90 IC days.\nQueued.");
    land_all(&mut rig, &queued);
    let org_key = format!("org/{:020}", org.0);
    let record = rig
        .tw
        .engine()
        .action_state()
        .get(&family, &org_key)
        .expect("the family reads")
        .cloned()
        .expect("the organisation's record");
    assert_eq!(record.principal, mush_org::PrincipalId::Organization(org));
    announce(&mut rig, &family, vec![org_key.clone()]);
    let notice = format!(
        "The Registry has barred your mind licences until {}.",
        tacsim::port::licence::ic_stamp(record.until_ic)
    );
    let mut heard = (0, 0);
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        heard.0 += count(&said(&rig.tw, &events, member), &notice);
        heard.1 += count(&said(&rig.tw, &events, bystander), &notice);
        land_all(&mut rig, &queued);
    }
    assert_eq!(heard, (1, 0), "the organisation's granted member, once");

    // A name that is both.
    god(&mut rig, &format!("@org/create {pilot_name}"));
    let (out, queued) = command(
        &mut rig,
        operator,
        &format!("@registry/revoke principal={pilot_name} #{n}"),
    );
    assert_eq!(
        out,
        "That name is a character and an organisation; name the character as #<n>."
    );
    assert!(queued.is_empty());

    // Both records survive a restart.
    let rig = reboot_with(rig, &[operator, pilot, member]);
    let family = tacsim::minds::registry_inspection::InspectionHost::of(rig.tw.world())
        .expect("the inspection")
        .writer()
        .revoked_principals()
        .expect("the family, registered again");
    for key in [&pilot_key, &org_key] {
        assert!(
            rig.tw
                .engine()
                .action_state()
                .get(&family, key)
                .expect("the family reads")
                .is_some(),
            "{key} survives the restart"
        );
    }
    no_refused_writes(&rig.tw);
}

/// **Staff bind `tv` to the fitted root `root`** (doc 17h cut M8e E6a, the staff rebind): the
/// leased rig's coreless unit stripped and made again from the cored root, in the bay.
pub(crate) fn fit(rig: &mut crate::excursions::Rig, tv: ObjId, root: &str) {
    let removed = command(rig, GOD, &format!("@unit/remove #{}", tv.0)).0;
    assert!(
        removed.contains("is no longer a tactical unit"),
        "{removed}"
    );
    let created = command(rig, GOD, &format!("@unit/create #{}={root}", tv.0)).0;
    assert!(
        !created.contains("No ") && !created.contains("not "),
        "{root}: {created}"
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), tv).is_some_and(|unit| unit
            .schema()
            .key()
            .id
            .as_str()
            == root),
        "{root}: the rig is bound to the root"
    );
}

/// **Doc 17h cut M8e E6a: a cored rig's epoch on the hardware-configured roster** (doc 23 §6;
/// D-UM-41): a leased rig bound to the governed rig root, and to its ungoverned twin, opens its
/// epoch at the launch's end on the composed path. With no mind held the seal is the ROM's
/// (`doctrine`, no fallback now that a core is fitted), and the record's seal facts name the
/// core component, its class and the governed bit, which the open line prints; an ungoverned
/// ROM seal admits nothing (Class 0 lifts no floor). The recover closes the epoch as for a
/// stock rig, and a reboot keeps both lines.
#[test]
fn m8e_a_cored_rig_seals_its_core_and_governed_bit() {
    use tacsim::definitions::core_replacement::{RIG_I, RIG_I_UNGOVERNED};
    for (root, governed) in [(RIG_I, true), (RIG_I_UNGOVERNED, false)] {
        let mut rig = crate::excursions::stocked_fitted();
        let (pilot, row, tvs) = on_pad(&mut rig, 1);
        let tv = tvs[0];
        let hull = ObjId(row.hull);
        fit(&mut rig, tv, root);
        let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
        let _ = finish(&mut rig, tv, pilot);
        let open = record_of(&mut rig.tw, tv)
            .and_then(|record| record.sortie)
            .expect("the epoch is open");
        let facts = open.seal.clone().expect("the rig is sealed");
        assert_eq!(facts.core.as_deref(), Some("core"), "{root}");
        assert_eq!(
            facts.core_class,
            Some(tacsim::arena::CoreClass::I),
            "{root}"
        );
        assert_eq!(facts.governed, governed, "{root}");
        assert_eq!(facts.licence.as_str(), "doctrine");
        assert!(
            facts.admissions.is_empty(),
            "{root}: {:?}",
            facts.admissions
        );
        let opened = lines(&rig, tv);
        assert_eq!(opened.len(), 1, "{root}: {opened:?}");
        let BlackBoxLine::MindSealed {
            governed: line_governed,
            seal_digest,
            ..
        } = &opened[0]
        else {
            panic!("{root}: the first line is the seal: {opened:?}");
        };
        assert_eq!(*line_governed, Some(governed), "{root}");
        assert_eq!(seal_digest.as_ref(), Some(&facts.seal_digest), "{root}");
        let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
        let _ = finish(&mut rig, tv, pilot);
        assert!(in_bay(&rig.tw, tv, &row));
        let both = lines(&rig, tv);
        assert_eq!(both.len(), 2, "{root}: {both:?}");
        assert!(matches!(
            both[1],
            BlackBoxLine::SortieClosed {
                close: IcCloseKind::ReturnedToBase { base },
                ..
            } if base == hull
        ));
        let rig = reboot_with(rig, &[pilot]);
        assert_eq!(
            lines(&rig, tv),
            both,
            "{root}: both lines survive a restart"
        );
        no_refused_writes(&rig.tw);
    }
}

/// The operator saves a personal draft `name` from Standard Doctrine (the workshop's verbs),
/// pumping the author service — its due fires, its observed submission landed — until the
/// Roll holds the saved revision (`hardware/flow/launch/story.rs`' `certify` loop).
pub(crate) fn save_draft(rig: &mut crate::excursions::Rig, who: ObjId, name: &str) {
    for line in [
        format!("@mind/new {name} FROM Standard"),
        format!("@mind/save {name}=first"),
    ] {
        let (out, _) = command(rig, who, &line);
        assert!(!out.contains("Usage"), "{line}: {out}");
    }
    let saved = |rig: &crate::excursions::Rig| {
        rig.tw
            .world()
            .module_state::<mush_roll::RollStore>()
            .and_then(|store| store.mind(who, name).ok().and_then(|mind| mind.latest()))
            .is_some()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !saved(rig) {
        assert!(
            std::time::Instant::now() < deadline,
            "the author service did not save {name}"
        );
        if let Some(when) = rig.tw.engine_mut().sched.next_due() {
            rig.tw.fire_due(when);
        }
        if let Some(submitted) = rig.tw.engine_mut().take_observed_authoritative_submit() {
            let (request, _) = submitted.into_parts();
            land(rig, &request).expect("the author service's save lands");
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// `line`'s one start, prepared on the composed path; its refusal message, or `None` when it
/// landed.
pub(crate) fn start_refusal(
    rig: &mut crate::excursions::Rig,
    who: ObjId,
    line: &str,
) -> Option<String> {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    land(rig, &queued[0]).err()
}

/// **Doc 17h cut M8e E5: the ungoverned rig seals the operator's unfiled draft aboard** (doc 23
/// S.4a; D-UM-41, D-UM-42), on the composed path: a rig bound to the ungoverned root, launched
/// `mind=draft:<name>`, opens its epoch sealed with that draft — its name, the `draft` state, the
/// ungoverned core and the admissions `unfiled` and `unlicensed`, on the record's facts and on
/// the open line; no licence, so no holder. The excursion runs as a stock rig's: the recover
/// closes it, and a reboot keeps both lines.
#[test]
fn m8e_an_ungoverned_rig_seals_the_operators_draft_aboard() {
    use tacsim::arena::mind_seal::Admission;
    use tacsim::definitions::core_replacement::RIG_I_UNGOVERNED;
    use tacsim::minds::licence_state::SealLicenceState;
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    fit(&mut rig, tv, RIG_I_UNGOVERNED);
    save_draft(&mut rig, pilot, "Scratch");
    let _ = act_go(&mut rig, pilot, "+tv/1 launch mind=draft:Scratch go");
    let sealed = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .and_then(|act| act.seal)
        .expect("the dispatch sealed the draft at the start");
    assert_eq!(sealed.licence, None, "a draft has no licence to mark");
    let _ = finish(&mut rig, tv, pilot);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the epoch is open");
    let facts = open.seal.clone().expect("sealed");
    assert_eq!(
        facts, sealed.facts,
        "the open epoch keeps the dispatch's seal"
    );
    assert_eq!(facts.mind.as_str(), "scratch", "the Roll's canonical name");
    assert_eq!(facts.licence_state, Some(SealLicenceState::Draft));
    assert!(!facts.governed);
    assert_eq!(
        facts.admissions,
        vec![Admission::Unfiled, Admission::Unlicensed]
    );
    assert_eq!(facts.holder, None);
    let opened = lines(&rig, tv);
    let BlackBoxLine::MindSealed {
        mind,
        admissions,
        governed,
        licence_state,
        ..
    } = &opened[0]
    else {
        panic!("the seal line: {opened:?}");
    };
    assert_eq!(mind.as_deref(), Some("scratch"));
    assert_eq!(admissions, &vec![Admission::Unfiled, Admission::Unlicensed]);
    assert_eq!(*governed, Some(false));
    assert_eq!(*licence_state, Some(SealLicenceState::Draft));
    let mut rig = reboot_with(rig, &[pilot]);
    assert_eq!(lines(&rig, tv), opened, "the seal line survives a restart");
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    assert_eq!(lines(&rig, tv).len(), 2);
    no_refused_writes(&rig.tw);
}

/// **Doc 17h cut M8e E4: the dispatch refuses by name** (D-UM-41), each refusal the start's
/// own and the rig left in the bay with nothing sealed: a draft on a governed core
/// (`MIND_GOVERNED_DRAFT`); a draft the operator does not have (`MIND_NO_DRAFT`); any mind on a
/// rig with no core (`MIND_NO_CORE`); a filed licence where no holdings are composed
/// (`MIND_UNAVAILABLE`). At the command, `org=` naming an organisation the operator does not act
/// for is the `+minds` family's sentence and `org=` without `mind=` the usage, nothing queued.
#[test]
fn m8e_the_dispatch_refuses_by_name() {
    use tacsim::definitions::core_replacement::RIG_I;
    use tacsim::surface::dispatch::{
        MIND_GOVERNED_DRAFT, MIND_NO_CORE, MIND_NO_DRAFT, MIND_NOT_FOR_ORG, MIND_UNAVAILABLE,
    };
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    save_draft(&mut rig, pilot, "Scratch");
    // No core yet: the leased rig is the coreless stock TV.
    let refused = start_refusal(&mut rig, pilot, "+tv/1 launch mind=draft:Scratch go");
    assert!(
        refused.as_deref().is_some_and(|r| r.contains(MIND_NO_CORE)),
        "{refused:?}"
    );
    fit(&mut rig, tv, RIG_I);
    for (line, want) in [
        (
            "+tv/1 launch mind=draft:Scratch go",
            MIND_GOVERNED_DRAFT.to_owned(),
        ),
        ("+tv/1 launch mind=filed:7 go", MIND_UNAVAILABLE.to_owned()),
    ] {
        let refused = start_refusal(&mut rig, pilot, line);
        assert!(
            refused.as_deref().is_some_and(|r| r.contains(&want)),
            "{line}: {refused:?}"
        );
    }
    assert!(in_bay(&rig.tw, tv, &row));
    assert!(record_of(&mut rig.tw, tv).is_none_or(|record| record.act.is_none()));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch mind=filed:7 org=Nobody go");
    assert_eq!(out.trim(), MIND_NOT_FOR_ORG);
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch org=Nobody go");
    assert_eq!(out.trim(), tacsim::port::tv::TV_USAGE);
    assert!(queued.is_empty());
    // An ungoverned core and a draft the operator never saved.
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    fit(
        &mut rig,
        tvs[0],
        tacsim::definitions::core_replacement::RIG_I_UNGOVERNED,
    );
    let refused = start_refusal(&mut rig, pilot, "+tv/1 launch mind=draft:Nothing go");
    assert!(
        refused
            .as_deref()
            .is_some_and(|r| r.contains(&MIND_NO_DRAFT.replace("{word}", "Nothing"))),
        "{refused:?}"
    );
    no_refused_writes(&rig.tw);
}

/// **Doc 17h cut M8e fix round 1, J1: the filed path's refusals in a commerce world** — the
/// fitted roster under the Active mind commerce (`reboot_with_inspection`), so the start's
/// prepare reads the Roll's real holdings: `mind=filed:7` with no such licence held is the
/// provider's own `MIND_NO_LICENCE`, nothing is committed (the rig stays in the bay, no act), and
/// nothing is poisoned (the same rig then launches plainly); `org=<org>` for an organisation the
/// pilot acts for on its root scope (`roll.holder.org`) reaches the real `own_paid` re-ask for
/// that organisation and is refused the same; one it does not act for is the command's sentence.
#[test]
fn m8e_the_filed_path_refuses_in_a_commerce_world() {
    use tacsim::definitions::core_replacement::RIG_I;
    use tacsim::surface::dispatch::{MIND_NO_LICENCE, MIND_NOT_FOR_ORG};
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (mut rig, _) = reboot_with_inspection(rig, pilot, &[pilot]);
    fit(&mut rig, tv, RIG_I);
    let missing = MIND_NO_LICENCE.replace("{licence}", "7");
    let refused = start_refusal(&mut rig, pilot, "+tv/1 launch mind=filed:7 go");
    assert!(
        refused.as_deref().is_some_and(|r| r.contains(&missing)),
        "the provider's own sentence: {refused:?}"
    );
    assert!(in_bay(&rig.tw, tv, &row));
    assert!(record_of(&mut rig.tw, tv).is_none_or(|record| record.act.is_none()));
    // The org arm: an organisation the pilot acts for, the paid view re-asked for it.
    for line in [
        "@org/create Guild".to_owned(),
        format!("@org/grant #{}=Guild roll.holder.org", pilot.0),
    ] {
        god(&mut rig, &line);
    }
    let refused = start_refusal(&mut rig, pilot, "+tv/1 launch mind=filed:7 org=Guild go");
    assert!(
        refused.as_deref().is_some_and(|r| r.contains(&missing)),
        "the org re-ask: {refused:?}"
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch mind=filed:7 org=Registry go");
    assert_eq!(out.trim(), MIND_NOT_FOR_ORG);
    assert!(queued.is_empty());
    // Nothing poisoned: a plain launch of the same rig lands and finishes.
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.sortie)
            .is_some()
    );
    no_refused_writes(&rig.tw);
}
