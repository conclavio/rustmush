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

use mush_core::ObjId;
use tacsim::arena::mind::SortieId;
use tacsim::definitions::UnitClass;
use tacsim::deliberation::order::CharacterRef;
use tacsim::surface::black_box::{BlackBoxLine, EpochHost, Hangar, SealWord};
use tacsim::surface::{ActKind, ActResult, RefusedCode, TvState};

use mush_roll::commerce::ic_claims::IcCloseKind;

use crate::excursions::{
    RAMP, act_go, command, count, due_of, finish, finishes, fire, in_bay, inspections, keeping,
    land, land_all, no_refused_writes, on_pad, on_pad_as, port_map, projection, prospector,
    put_charge, reboot_with, reboot_with_inspection, record_of, recovery_order, recovery_staff,
    said, staff_bay, staff_place, staff_place_on, stocked, store,
};

/// A hull's black box lines, decoded (the store's open tail); every entry decodes (the fix
/// round's F4: nothing is dropped silently).
fn lines(rig: &crate::excursions::Rig, hull: ObjId) -> Vec<BlackBoxLine> {
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
fn hull_name(rig: &crate::excursions::Rig, hull: ObjId) -> String {
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
    let open_line = format!(
        "{} out · surface from the bay of {} · Standard Doctrine · the ROM · governed · licence \
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
    let staff = recovery_staff(&mut rig.tw);
    let (out, _) = command(&mut rig, staff, &format!("+hull/box #{}", hull.0));
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.contains(" out · port from "), "{out}");

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

/// **M8a 4. The Registry inspects a rig on the pad and in the field** (doc 17h cut M8a, A4,
/// A6, A7, A8; the brief's test 4), composed here (`reboot_with_inspection`: the Registry
/// authority with an operator character, the Roll's setup acknowledged, the inspection over the
/// authority; the mushd lens harness cannot stand a surface rig up): a non-operator is told the
/// revocation's one sentence and nothing else, before any hull is looked at; a number no hull
/// row holds is `No such hull.`; on the pad the operator's `@registry/inspect #tv` prints the
/// record in words — the ROM, provenance unknown, no trip open — and stages one action whose
/// landing writes one `roll_registry_inspections` record (`core: None`, `Zero`, `governed:
/// None`, `Unknown`, `NoSeal`; no seal, so no licence state); in the field the same hull reads
/// its open trip — the seal's mind, `licence doctrine`, the first eight of its digest, `seal
/// matches the fitted core` — and the second record carries the digest, `Matches` and
/// `Some(Doctrine)`; both records survive a restart, and a third inspection writes a third.
#[test]
#[allow(clippy::too_many_lines)]
fn m8a_the_registry_inspects_a_rig_on_the_pad_and_in_the_field() {
    use tacsim::minds::licence_state::SealLicenceState;
    use tacsim::minds::registry_inspection::{HullClass, InspectionFacts, Provenance, SealCheck};

    let mut rig = stocked();
    let operator = rig.tw.spawn_player("Registry operator");
    let (mut rig, policy) = reboot_with_inspection(rig, operator, &[operator]);
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let n = tv.0;
    let name = hull_name(&rig, tv);
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

    // On the pad: no trip open; one record. The fifth line is the driver's receipt of the
    // submission (`Queued.`, E7c2 R11), never the command's.
    let (out, queued) = command(&mut rig, operator, &format!("@registry/inspect #{n}"));
    assert_eq!(
        out,
        format!(
            "Hull #{n} {name} · the ROM\nprovenance unknown\nno trip open\nInspecting hull \
             #{n}.\nQueued."
        )
    );
    assert_eq!(queued.len(), 1, "one submission: {out}");
    assert_eq!(queued[0].actor, operator);
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::minds::registry_inspection::INSPECTION_ACTION
    );
    land_all(&mut rig, &queued);
    let records = inspections(&rig.tw);
    assert_eq!(records.len(), 1, "{records:?}");
    let first = &records[0];
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
    assert_eq!(
        out,
        format!(
            "Hull #{n} {name} · the ROM\nprovenance unknown\nout since {} · Standard Doctrine · \
             licence doctrine · seal {} · seal matches the fitted core\nInspecting hull #{n}.\
             \nQueued.",
            tacsim::port::licence::ic_stamp(out_ic),
            &seal.seal_digest[..8]
        )
    );
    assert!(!out.contains("sortie"), "{out}");
    assert!(out.lines().count() <= 6);
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
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
        }
    );

    // A restart keeps both; a third inspection writes a third.
    let mut rig = reboot_with(rig, &[operator, pilot]);
    let mut kept = inspections(&rig.tw);
    kept.sort_by_key(|r| r.inspection.to_hex());
    let mut before = records.clone();
    before.sort_by_key(|r| r.inspection.to_hex());
    assert_eq!(kept, before, "the records survive a restart");
    let (_, queued) = command(&mut rig, operator, &format!("@registry/inspect #{n}"));
    land_all(&mut rig, &queued);
    assert_eq!(inspections(&rig.tw).len(), 3);
    let (out, queued) = command(&mut rig, pilot, &format!("@registry/inspect #{n}"));
    assert_eq!(out, "That did not go through. Nothing changed.");
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}
