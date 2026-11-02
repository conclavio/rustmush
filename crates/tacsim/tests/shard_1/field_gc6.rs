//! **Doc 36 cut GC.6 — the claim (story b)** (Q-36-24 to Q-36-28, Q-36-30; doc 27 Q-S1–Q-S4,
//! Q-S13–Q-S15, Q-F3, Q-R1, P-3), over the composed world (`driving`'s keyed rig) on Urvara
//! Reach, with three parties:
//!
//! - **the title holder** (T), whose two disabled rigs lie on the ground and whose carrier
//!   stands on the pad;
//! - **the opportunist** (O), whose rig tows both into O's own carrier and files a claim on each;
//! - **the landholder** (L), who holds the Registry's claim on the block where the first lies.
//!
//! The first recovery is off L's ground with neither T's nor L's authority: T's own carrier saw
//! the attach, so T contests with that sensor record, hands it to L as a report, and L contests
//! too; the claim is vacated at once and the title never moves. The second lies on open ground
//! out of anyone's sight: nobody contests, a W-clock outage pauses the compressed window, and when
//! it runs out the registry awards the title to O, which GC.8b's rights table then reads.
//!
//! **What is a fixture.** The rigs' power is destroyed and they are put on the site's index by
//! staff hand (GC.4's and GC.5's precedent), their title is moved to T by `@asset/transfer`, and
//! L's block claim is seeded as a typed record (only industry's composed claim act writes one in
//! play). The world clock's outage is the two `W-clock` events the economy subscribes to. The
//! remains GC.6 scraps are dropped by staff.

use mush_core::{GOD, ObjId, WorldEvent};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::surface::claim::UnlawfulKind;
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::site_record::{GroundKind, SiteRecord};

use crate::driving::{
    cid, command, launched_at, on_pad_as_at, pilot, port_map, reboot, run, seed_typed, staff_hand,
    stocked_tuned, to_cell_on,
};
use crate::field_gc4::{powerless, seconds_until};
use crate::field_host::{second, site_record_at, tune};
use crate::site_record::{staff, staff_member};

/// Urvara Reach's port entry.
const URVARA: &str = "urvara_port";
/// The compressed contest window, real seconds.
const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute contest window, on the field host's lab tempo.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_s = WINDOW;
}

fn record(rig: &mut crate::driving::Rig) -> SiteRecord {
    site_record_at(rig, URVARA).expect("Urvara Reach's record")
}

fn custody(rig: &mut crate::driving::Rig, hull: ObjId) -> Option<SalvageCustody> {
    record(rig).custody.into_iter().find(|row| row.hull == hull)
}

fn title(rig: &crate::driving::Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

fn contract(rig: &crate::driving::Rig, id: u64) -> mush_economy::EconomicContract {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(id)).cloned())
        .expect("the claim's contract")
}

/// `line` as `who`, every submission it queued landed; the reply.
fn go(rig: &mut crate::driving::Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one submission: {out}");
    crate::driving::land(rig, &queued[0]).unwrap_or_else(|e| panic!("{line}: {e}"));
    out
}

/// Move real time `secs` on, then a second of ticks.
fn jump(rig: &mut crate::driving::Rig, secs: i64) {
    let t = rig.tw.world().now().saturating_add(secs);
    rig.tw.set_now(t);
    let _ = second(rig);
}

/// The tug's tow on Urvara Reach.
fn towing(rig: &mut crate::driving::Rig, tug: ObjId) -> Option<ObjId> {
    record(rig).tows.towing(tug).map(|op| op.casualty())
}

/// O's rig takes `casualty` in tow from beside it.
fn attach(rig: &mut crate::driving::Rig, who: ObjId, tug: ObjId, casualty: ObjId) {
    let out = go(rig, who, &format!("+tv/1 tow #{} go", casualty.0));
    assert!(
        seconds_until(rig, 120, |rig| towing(rig, tug) == Some(casualty)),
        "the tow attaches: {out}"
    );
}

/// O's rig, beside its own ramp, loads its tow into O's carrier.
fn load(rig: &mut crate::driving::Rig, who: ObjId, tug: ObjId, casualty: ObjId) {
    let out = go(rig, who, "+tv/1 load go");
    assert!(
        seconds_until(rig, 120, |rig| custody(rig, casualty).is_some()
            && record(rig).tows.act_of(tug).is_none()),
        "the load lands in custody: {out}"
    );
}

/// **GC.6's gate: story (b)**, the contested salvage, with an uncontested claim beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc6_story_b_the_contested_salvage_on_urvara_reach() {
    let mut rig = stocked_tuned(quick);
    // T: a carrier on the pad. O: a carrier on the same pad with one rig.
    let (t, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ("urvara", URVARA), 1);
    let (o, o_row, o_tvs) =
        on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ("urvara", URVARA), 1);
    let o_carrier = ObjId(o_row.hull);
    let tug = o_tvs[0];
    // L: the landholder.
    let l = pilot(&mut rig, "Lander", 3);
    // T's two hulls: spare rigs of Occator's pool (a leased rig's title carries the lease's
    // claim and cannot move), titled to T by staff (GC.8b's precedent).
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
        .filter(|id| crate::driving::store(&rig.tw).tv_lease_on(*id).is_none())
        .take(2)
        .collect();
    let [near, far] = spares.as_slice() else {
        panic!("two spare rigs in Occator's pool: {spares:?}");
    };
    let (near, far) = (*near, *far);
    for hull in [near, far] {
        let moved = run(
            &mut rig.tw,
            GOD,
            &format!("@asset/transfer #{}=#{}", hull.0, t.0),
        );
        assert_eq!(title(&rig, hull), Some(PrincipalId::Object(t)), "{moved}");
    }
    // T's hulls out on the ground, disabled and left: `near` beside the pad (in sight of T's
    // carrier) on L's block, `far` out of anyone's sight on open ground.
    let near_cell = (258_i64, 254_i64);
    let far_cell = (258_i64, 300_i64);
    to_cell_on(&mut rig, URVARA, near, near_cell);
    to_cell_on(&mut rig, URVARA, far, far_cell);
    let s = staff_member(&mut rig);
    for (hull, (x, y)) in [(near, near_cell), (far, far_cell)] {
        powerless(&mut rig, hull);
        staff(
            &mut rig,
            s,
            &format!("@tacsim/site {URVARA} leave #{} casualty {x},{y}", hull.0),
        );
    }
    // L's block: the Registry's claim on the block `near` lies in, seeded.
    let scheme = tacsim::surface::address::Scheme::of_site(512);
    let letter = tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: near_cell.0 * 1_000 + 500,
            y_cm: near_cell.1 * 1_000 + 500,
        },
        scheme,
    )
    .expect("a block");
    let far_letter = tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: far_cell.0 * 1_000 + 500,
            y_cm: far_cell.1 * 1_000 + 500,
        },
        scheme,
    )
    .expect("a block");
    assert_ne!(letter, far_letter, "the far rig lies on another block");
    let place = port_map(&rig.tw, URVARA)
        .site
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(URVARA), letter, place);
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
        &mut rig,
        &handles.families.claims,
        &tacsim::surface::claims::BlockKey {
            site: cid(URVARA),
            letter,
        },
        tenure,
    );

    // 1. O's rig tows `near` (pinning L's tenure, T's title and T's carrier as a witness); a
    //    restart with the attachment intact keeps the pin; O loads it into O's own carrier.
    launched_at(&mut rig, URVARA, o, tug);
    to_cell_on(&mut rig, URVARA, tug, (near_cell.0 + 1, near_cell.1));
    let _ = second(&mut rig);
    attach(&mut rig, o, tug, near);
    let pin = record(&mut rig)
        .tows
        .pins
        .into_iter()
        .find(|pin| pin.casualty == near)
        .expect("the attach pinned the tenure");
    assert_eq!(pin.tug, tug);
    assert_eq!(pin.landholder(), Some(l), "{pin:?}");
    assert_eq!(pin.answering, Some(t));
    assert!(
        pin.witnesses.iter().any(|w| w.holder == t),
        "T's carrier saw it: {pin:?}"
    );
    assert!(pin.witnesses.iter().all(|w| w.holder != l), "L saw nothing");
    let mut rig = reboot(rig, &[t, o, l]);
    assert_eq!(
        record(&mut rig)
            .tows
            .pins
            .iter()
            .find(|kept| kept.casualty == near),
        Some(&pin),
        "the pin survives a restart with the attachment intact"
    );
    let ramp = {
        let (x, y, _) = crate::driving::ramp_at(&rig.tw, URVARA);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    to_cell_on(&mut rig, URVARA, tug, ramp);
    let _ = second(&mut rig);
    load(&mut rig, o, tug, near);
    let held = custody(&mut rig, near).expect("in O's custody");
    assert_eq!((held.holder, held.carrier), (o, o_carrier));
    assert_eq!(held.claim, ClaimSeam::Owed);
    assert_eq!(held.pin.as_ref(), Some(&pin), "the pin rides into custody");
    assert!(held.operation.is_some());
    assert!(
        record(&mut rig)
            .unlawful
            .iter()
            .any(|act| act.kind == UnlawfulKind::Recovery
                && act.actor == o
                && act.subject == near.0
                && act.landholder == Some(l)),
        "the unlawful recovery is evidence on the site's log"
    );

    // 2. The same for `far`, on open ground and out of sight.
    to_cell_on(&mut rig, URVARA, tug, (far_cell.0 + 1, far_cell.1));
    let _ = second(&mut rig);
    attach(&mut rig, o, tug, far);
    to_cell_on(&mut rig, URVARA, tug, ramp);
    let _ = second(&mut rig);
    load(&mut rig, o, tug, far);
    let far_pin = custody(&mut rig, far)
        .and_then(|row| row.pin)
        .expect("far's pin");
    assert_eq!(far_pin.landholder(), None, "open ground");
    assert!(far_pin.witnesses.is_empty(), "nobody saw it: {far_pin:?}");

    // 3. Filing. Only the custodian files; the preview names who is told; the claim is an
    //    economy contract with a RealUtc window, and the seam moves to Filed.
    let refused = run(&mut rig.tw, t, &format!("+salvage/claim #{}", near.0));
    assert!(refused.contains("Only the"), "{refused}");
    let preview = run(&mut rig.tw, o, &format!("+salvage/claim #{}", near.0));
    assert!(
        preview.contains("Freeport Registry")
            && preview.contains("Pilot")
            && preview.contains("Lander")
            && preview.contains("10 real minutes"),
        "{preview}"
    );
    let filed_at = rig.tw.world().now();
    let out = go(&mut rig, o, &format!("+salvage/claim #{} go", near.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    let near_claim = custody(&mut rig, near).expect("still in custody");
    let ClaimSeam::Filed { contract: c_near } = near_claim.claim else {
        panic!("filed: {near_claim:?}");
    };
    assert_eq!(near_claim.told, vec![t, l], "T and L are told");
    let file = near_claim.file.as_ref().expect("the library's claim");
    assert!(!file.entitlement_at_attach.is_lawful(), "{file:?}");
    let filed = contract(&rig, c_near);
    assert_eq!(filed.state, ContractState::Active);
    assert_eq!(
        mush_economy::salvage_claim_hull(&filed),
        Some(near),
        "a salvage claim on the hull"
    );
    assert_eq!(
        filed.effective_real_window_end(),
        Some(filed_at + WINDOW),
        "the compressed RealUtc window"
    );
    let again = run(&mut rig.tw, o, &format!("+salvage/claim #{} go", near.0));
    assert!(again.contains("filed already"), "{again}");
    let out = go(&mut rig, o, &format!("+salvage/claim #{} go", far.0));
    assert!(out.contains("You file a salvage claim"), "{out}");
    let far_claim = custody(&mut rig, far).expect("far in custody");
    let ClaimSeam::Filed { contract: c_far } = far_claim.claim else {
        panic!("filed: {far_claim:?}");
    };
    assert_eq!(far_claim.told, vec![t], "only T: open ground");
    assert!(
        far_claim
            .file
            .as_ref()
            .is_some_and(|f| f.entitlement_at_attach.is_lawful())
    );

    // 4. Evidence. T may not read the black box from outside O's carrier; O, aboard, may.
    let refused = run(&mut rig.tw, t, &format!("+salvage/inspect #{}", near.0));
    assert!(refused.contains("read aboard the Red Moth"), "{refused}");
    let read = go(&mut rig, o, &format!("+salvage/inspect #{}", near.0));
    assert!(read.contains("black box"), "{read}");

    // 5. The contest: T on its own sensor record. T then hands that record to L as a report,
    //    and L, who saw nothing, now holds evidence too.
    let preview = run(&mut rig.tw, t, &format!("+salvage/contest #{}", near.0));
    assert!(
        preview.contains("unlawful recovery") && preview.contains("sensor record"),
        "{preview}"
    );
    let refused = run(&mut rig.tw, l, &format!("+salvage/contest #{}", near.0));
    assert!(refused.contains("neither evidence"), "{refused}");
    let _ = go(&mut rig, t, &format!("+salvage/contest #{} go", near.0));
    let _ = go(
        &mut rig,
        t,
        &format!("+salvage/report #{}=Lander go", near.0),
    );
    let preview = run(&mut rig.tw, l, &format!("+salvage/contest #{}", near.0));
    assert!(preview.contains("unlawful recovery"), "{preview}");
    // T's preview on `far` offers bare title only, which never defeats a lawful claim; T does
    // not file it, so `far` stays uncontested.
    let bare = run(&mut rig.tw, t, &format!("+salvage/contest #{}", far.0));
    assert!(bare.contains("Bare title never defeats"), "{bare}");

    // 6. A vacating contest settles at once: the contract closes, custody goes, the title
    //    never moved, and the vacated claim is evidence on the site's log.
    assert!(
        seconds_until(&mut rig, 10, |rig| custody(rig, near).is_none()),
        "the claim on `near` is vacated"
    );
    assert_eq!(contract(&rig, c_near).state, ContractState::Cancelled);
    assert_eq!(title(&rig, near), Some(PrincipalId::Object(t)));
    assert!(
        record(&mut rig)
            .unlawful
            .iter()
            .any(|act| act.kind == UnlawfulKind::Vacated && act.subject == near.0)
    );

    // 7. The W-clock outage pauses `far`'s window: frozen past its first deadline, nothing is
    //    awarded; resumed, the window is extended by the time lost, and runs out after it.
    let frozen_at = rig.tw.world().now();
    let ic = rig.tw.world().world_ic_now().at;
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().emit_world_event(
        &WorldEvent::WorldClockFrozen {
            real_unix: frozen_at,
            ic_unix: ic,
        },
        &mut out,
        &mut fx,
    );
    assert!(
        contract(&rig, c_far)
            .paused_real_remaining_secs
            .is_some_and(|rem| rem > 0),
        "the window is paused"
    );
    let outage = WINDOW + 300;
    jump(&mut rig, outage);
    for _ in 0..5 {
        let _ = second(&mut rig);
    }
    assert!(
        custody(&mut rig, far).is_some_and(|row| row.file.is_some()),
        "nothing settles while the clock is stopped"
    );
    let refused = run(&mut rig.tw, t, &format!("+salvage/contest #{}", far.0));
    assert!(refused.contains("paused"), "{refused}");
    let resumed_at = rig.tw.world().now();
    let unavailable = resumed_at - frozen_at;
    rig.tw.engine_mut().emit_world_event(
        &WorldEvent::WorldClockResumed {
            frozen_real_unix: frozen_at,
            resumed_real_unix: resumed_at,
            unavailable_secs: unavailable,
            ic_unix: ic,
        },
        &mut out,
        &mut fx,
    );
    let end = contract(&rig, c_far)
        .effective_real_window_end()
        .expect("a window");
    assert!(end > resumed_at, "the window reopens after the outage");
    assert_eq!(
        contract(&rig, c_far).real_deadline_extension_secs,
        unavailable,
        "extended by the time lost"
    );
    // Before the award O holds `far` as its salvor (custody), never its title.
    assert_eq!(custody(&mut rig, far).map(|row| row.holder), Some(o));
    assert_eq!(title(&rig, far), Some(PrincipalId::Object(t)));
    let short = end - rig.tw.world().now() - 30;
    jump(&mut rig, short);
    assert!(
        custody(&mut rig, far).is_some(),
        "not yet: the window still runs"
    );
    jump(&mut rig, 60);
    assert!(
        seconds_until(&mut rig, 30, |rig| custody(rig, far).is_none()),
        "the uncontested claim on `far` settles"
    );

    // 8. The award: the title is O's, through the economy's journal, and GC.8b's rights table
    //    reads O as the title holder (its title arm reads the economy alone).
    assert_eq!(title(&rig, far), Some(PrincipalId::Object(o)));
    assert!(
        seconds_until(&mut rig, 120, |rig| contract(rig, c_far).state
            == ContractState::Expired),
        "the sweep lapses the claim's contract"
    );
    let after = staff_hand(&mut rig, |ctx| {
        ctx.actor = o;
        tacsim::port::bay_work::right_of(ctx, far)
    });
    assert_eq!(after, Some(tacsim::port::bay_work::Right::Title));

    // 9. Remains scrapped where they lie (GC.3's follow-up): open to the title holder, which
    //    the winning claimant now is. O may not break up remains of T's hull; O breaks up
    //    remains of the hull the registry just awarded O, and the alloys go into O's hold.
    let (a_cell, b_cell) = ((262_i64, 252_i64), (262_i64, 254_i64));
    for (hull, (x, y)) in [(near, a_cell), (far, b_cell)] {
        staff(
            &mut rig,
            s,
            &format!(
                "@tacsim/site {URVARA} drop remains {x},{y} 6400 #{}",
                hull.0
            ),
        );
    }
    let remains_of = |rig: &mut crate::driving::Rig, hull: ObjId| {
        record(rig)
            .objects
            .iter()
            .find(|obj| matches!(obj.kind, GroundKind::Remains { of: Some(of), .. } if of == hull))
            .map(|obj| obj.id)
            .expect("the remains lie on the ground")
    };
    let (of_near, of_far) = (remains_of(&mut rig, near), remains_of(&mut rig, far));
    to_cell_on(&mut rig, URVARA, tug, (263, 253));
    let _ = second(&mut rig);
    let refused = run(&mut rig.tw, o, &format!("+tv/1 scrap {of_near}"));
    assert!(refused.contains("Only the title holder"), "{refused}");
    let preview = run(&mut rig.tw, o, &format!("+tv/1 scrap {of_far}"));
    assert!(
        preview.contains("6 t of structural_alloys") && preview.contains("Red Moth"),
        "{preview}"
    );
    let out = go(&mut rig, o, &format!("+tv/1 scrap {of_far} go"));
    assert!(out.contains("breaks up"), "{out}");
    assert!(
        record(&mut rig).objects.iter().all(|obj| obj.id != of_far),
        "the remains are off the site's record"
    );
    let lot = tacsim::surface::claim::remains_lot(&cid(URVARA), of_far).expect("the lot");
    let inventory = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .and_then(|h| h.inventory)
        .expect("the inventory");
    let found = crate::driving::typed(&mut rig.tw, &inventory.lots, &lot).expect("the alloys");
    assert_eq!(found.commodity().as_str(), "structural_alloys");
    assert_eq!(found.quantity(), 6);
    assert_eq!(
        found.endpoint(),
        mush_inventory::EndpointRef(crate::driving::hold_vault_of(&rig.tw, &o_row))
    );
}
