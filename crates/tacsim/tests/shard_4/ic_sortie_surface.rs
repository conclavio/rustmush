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

use mush_core::ObjId;
use tacsim::arena::mind::SortieId;
use tacsim::definitions::UnitClass;
use tacsim::deliberation::order::CharacterRef;
use tacsim::surface::black_box::{BlackBoxLine, EpochHost, SealWord};
use tacsim::surface::{ActKind, ActResult, RefusedCode, TvState};

use mush_roll::commerce::ic_claims::IcCloseKind;

use crate::excursions::{
    RAMP, act_go, due_of, finish, finishes, fire, in_bay, land, land_all, no_refused_writes,
    on_pad, projection, put_charge, reboot_with, record_of, recovery_order, recovery_staff,
    staff_place, stocked, store,
};

/// A hull's black box lines, decoded (the store's open tail); every entry decodes (the fix
/// round's F4: nothing is dropped silently).
fn lines(rig: &crate::excursions::Rig, hull: ObjId) -> Vec<BlackBoxLine> {
    let (lines, skipped) = store(&rig.tw).black_box_lines(hull);
    assert_eq!(skipped, 0, "every black box entry decodes");
    lines.into_iter().map(|(_, line)| line).collect()
}

/// **The whole black box read back from the store's documents** — the history read an
/// inspection makes over the chunk documents (`black_box::history`), not the RAM tail.
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
/// abandoned — and the black box one `mind_sealed` line: the Dray-P and no open epoch of its own
/// as the hangar, the operator's character the principal, the ROM seal (`no core: the ROM`,
/// Class 0, governed, `doctrine`), the vehicle class admitted as the site's one open epoch. A
/// reboot from the store keeps the record and the line. Recovered and finished, the epoch is
/// closed and the black box holds the `sortie_closed` line, `returned_to_base` into the Dray-P,
/// not abandoned — two lines in all, which a second reboot and the documents' own history read
/// both give back.
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
    assert_eq!(hangar.hull, hull, "the hangar is the Dray-P (D-23-2)");
    assert_eq!(hangar.slot, 1);
    assert_eq!(hangar.sortie, None, "the ship has no epoch until S.2");
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
    assert!(
        lines(&rig, hull).is_empty(),
        "the Dray-P's own black box is empty in S.1"
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
/// `abandoned`, with no `sortie_closed` line; a staff `home` then closes it `returned_to_base`
/// with `abandoned` on the line.
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
    recovery_order(&mut rig, staff, "close", tv);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.excursion, None, "the close clears the excursion");
    let kept = record.sortie.clone().expect("the epoch is kept");
    assert!(kept.abandoned, "flagged abandoned");
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
        close, abandoned, ..
    } = &both[1]
    else {
        panic!("the close line: {both:?}");
    };
    assert_eq!(*close, IcCloseKind::ReturnedToBase { base: hull });
    assert!(*abandoned, "the line carries the flag");
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
/// closes the epoch `returned_to_base`, abandoned.
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
    assert_eq!((*sortie, hangar.hull), (open.id, hull));
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
    assert_eq!(*close, IcCloseKind::ReturnedToBase { base: hull });
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
