//! **Doc 40 — the ramp exit's boarding window** (the owner's ruling, 2026-11-04, "back in the
//! field"), over the composed world (`driving`'s keyed rig on Ahuna Flank), the arena's egress
//! pumped by hand as `field_raiders.rs` pumps it.
//!
//! A raider's withdrawal counts only once the hull is aboard, and the pad is never an exit:
//!
//! - **The carrier lost first.** The raider's fixture carrier is a wreck in the field when its
//!   leader's WITHDRAW goes out. Before the fix the kernel let each hull go at the ramp and the
//!   host's later step put it "aboard" the wreck: out of the field, off the ground, its exit on
//!   the record. Now the kernel keeps it (`WithdrawRefusal::CarrierLost`) and the host strands it
//!   in the field, its WITHDRAW lifted so its doctrine fights on.
//! - **The carrier lost later.** A hull that went aboard (in the batch that let it go) and the
//!   hull that never launched are lost with the carrier when it falls after the boarding: each a
//!   wreck on its own record once the carrier checks in, still aboard, never deleted.
//!
//! **What is a fixture.** The first story wrecks the carrier by staff hand before the field (the
//! arena's wreck mark on its record, `carrier_damage::wreck_plan`; an admitted wreck stays one).
//! The second stands a destroyed drive in by staff hand (GC.3's fixture), so the carrier stays on
//! the ground with its hulls aboard instead of lifting, and its hull falls to B's fire in the
//! field, as GC.3's gate takes a Dray-P's.

use crate::field_host_rig::{seat, set_section};
use crate::field_raiders_rig::*;

/// **`unit` a wreck on its own record**, by staff hand (the arena's wreck mark).
fn wreck(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let plan = tacsim::carrier_damage::wreck_plan(
            tacsim::carrier_damage::spec_of(ctx.w, &cells),
            &cells,
        )
        .expect("a wreck plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Whether `unit` is a wreck on its own record.
fn wrecked(rig: &Rig, unit: ObjId) -> bool {
    mush_tactics::module::unit_of(rig.tw.world(), unit).is_some_and(tacsim::arena::damage::is_wreck)
}

/// Whether `unit` is a row of Ahuna Flank's live record.
fn in_field(rig: &Rig, unit: ObjId) -> bool {
    tacsim::field::live_on(rig.tw.world(), &cid(AHUNA))
        .is_some_and(|live| live.units.iter().any(|row| row.unit == unit))
}

/// **`by` marks the flight at `bearing` Hostile**: the promotion lands, everyone in `offline`
/// goes offline, and the field's task opens.
async fn open_field(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    (by, bearing): (ObjId, &str),
    offline: &[ObjId],
) {
    let (out, queued) = command(rig, by, &format!("+field/mark {bearing}=hostile"));
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
    for who in offline {
        rig.tw.world_mut().set_connected(*who, false);
    }
    assert!(
        until(rig, rx, lines, 15, |rig, _| task(rig).is_some()).await,
        "the field's task opens"
    );
}

/// **The boarding window's repro** (the owner's ruling, 2026-11-04): the raider's carrier is a
/// wreck in the field when its leader's WITHDRAW goes out. Every hull stays in the field on its
/// row, on the ground, its exit never named and its WITHDRAW spent; nobody boards a wreck.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn boarding_a_raider_whose_carrier_is_lost_stays_in_the_field() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 0);
    let escort = armed(&mut rig, "Ward Dog", (183, 126), 270_000);
    let npc = staff_hand(&mut rig, |ctx| {
        ctx.w
            .create(ObjKind::Npc, "Hired escort", GOD, None)
            .expect("the NPC")
    });
    seat_npc(&mut rig, escort, npc);
    warm(&mut rig);
    let (raiders, carrier) = seed_and_launch(&mut rig);
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    wreck(&mut rig, carrier);
    assert!(
        wrecked(&rig, carrier),
        "the fixture: the carrier is a wreck"
    );
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        second(&mut rig);
    }
    open_field(
        &mut rig,
        &mut rx,
        &mut lines,
        (leader, "E"),
        &[a, npc, leader],
    )
    .await;
    assert!(in_field(&rig, carrier), "the wreck is admitted");
    for hull in &raiders {
        assert!(in_field(&rig, *hull), "#{} is in the field", hull.0);
    }

    let ordered = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/withdraw {SKULKS}"),
    );
    assert!(ordered.contains("make for the"), "{ordered}");
    // Each hull is asked out once a world second; the kernel keeps it and the host spends its
    // WITHDRAW. Before the fix each was let go instead and boarded the wreck.
    let settled = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        raiders.iter().all(|hull| {
            !tacsim::field::raiders::withdrawing(rig.tw.world(), *hull)
                || mush_sim::pose_of(rig.tw.world(), *hull).is_none()
        })
    })
    .await;
    let where_now = |rig: &Rig| -> Vec<_> {
        raiders
            .iter()
            .map(|hull| {
                (
                    hull.0,
                    in_field(rig, *hull),
                    mush_sim::pose_of(rig.tw.world(), *hull)
                        .map(|(_, pose)| (pose.x_cm / 1_000, pose.y_cm / 1_000)),
                    tacsim::field::raiders::withdrawing(rig.tw.world(), *hull),
                )
            })
            .collect()
    };
    assert!(settled, "the WITHDRAW settles: {:?}", where_now(&rig));
    // A few more seconds: nobody is let go late.
    let _ = until(&mut rig, &mut rx, &mut lines, 4, |_, _| false).await;
    let record = site_record(&mut rig).expect("the record");
    for hull in &raiders {
        assert!(
            in_field(&rig, *hull) && mush_sim::pose_of(rig.tw.world(), *hull).is_some(),
            "#{} stays in the field, on the ground, stranded: {:?}",
            hull.0,
            where_now(&rig)
        );
        assert!(
            !record.exited(*hull),
            "#{}'s exit is never named: the pad is no exit",
            hull.0
        );
        assert!(
            !tacsim::field::raiders::withdrawing(rig.tw.world(), *hull),
            "#{}'s WITHDRAW is lifted, so it fights on",
            hull.0
        );
    }
}

/// **A carrier lost after the boarding takes its hulls with it** (the owner's ruling,
/// 2026-11-04): one hull withdraws and goes aboard in the batch that lets it go, the other never
/// launched; then B's fire takes the grounded carrier's hull in the field. At demotion the
/// carrier is a wreck on the site's index and both hulls are wrecks on their own records, still
/// aboard (off the ground, the raider's mark naming the carrier), and neither launches again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn boarding_a_raider_aboard_a_carrier_wrecked_later_is_a_wreck_in_the_bay() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 0);
    let (b, _) = crate::driving_rig::prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (166, 130), 270_000);
    seat(&mut rig, hull_b, b);
    warm(&mut rig);
    let (raiders, carrier) = seed_and_launch(&mut rig);
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    // The second hull back aboard before the field; the first under its leader's WITHDRAW, so
    // the host orders it home once the field runs.
    staff_hand(&mut rig, |ctx| {
        mush_sim::unplace(ctx, raiders[1], "test: aboard").expect("aboard");
        let mut mark = ctx
            .w
            .attr(raiders[0], tacsim::field::raiders::RAIDER_ATTR)
            .and_then(|attr| attr.as_json().cloned())
            .expect("the raider's mark");
        mark["withdraw"] = serde_json::Value::Bool(true);
        ctx.w
            .set_attr(
                raiders[0],
                tacsim::field::raiders::RAIDER_ATTR,
                mush_core::Attr::json(mark, GOD),
            )
            .expect("marked");
    });
    // GC.3's fixtures: a destroyed drive keeps the carrier on the ground with its hulls aboard;
    // an unarmoured hull at one point of structure falls to a few rounds.
    set_section(&mut rig, carrier, "drive", Some("destroyed"), Some(0));
    stripped(&mut rig, carrier);
    set_section(&mut rig, carrier, "hull", None, Some(1));
    assert!(
        tacsim::carrier_damage::damage_of(rig.tw.world(), carrier)
            .is_some_and(|damage| damage.has(tacsim::carrier_damage::CarrierLoss::RefusesLift)),
        "the fixture: the carrier refuses lift"
    );
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        second(&mut rig);
    }
    // B marks the raider west of it: the raider holds its standing Hostile on B's party.
    open_field(&mut rig, &mut rx, &mut lines, (b, "W"), &[a, leader]).await;

    // The first hull goes home and aboard at once: off the ground the moment it leaves the field.
    let aboard = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        !in_field(rig, raiders[0])
    })
    .await;
    assert!(aboard, "the withdrawing hull leaves the field at its ramp");
    assert!(
        mush_sim::pose_of(rig.tw.world(), raiders[0]).is_none(),
        "it is aboard in the batch that let it go, not on the site's ground"
    );
    assert!(!tacsim::field::raiders::withdrawing(
        rig.tw.world(),
        raiders[0]
    ));

    // B locks the carrier and fires its main gun until the carrier's hull falls.
    let at_carrier = |rig: &Rig| {
        frame(rig, b).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(156) <= 1 && y.abs_diff(126) <= 1)
                })
                .cloned()
        })
    };
    let destroyed = |lines: &[LedgerEvent]| {
        lines.iter().any(
            |line| matches!(line.kind, EventKind::UnitDestroyed { unit, .. } if unit == carrier),
        )
    };
    let fell = until(&mut rig, &mut rx, &mut lines, 120, |rig, lines| {
        if destroyed(lines) {
            return true;
        }
        if let Some(contact) = at_carrier(rig)
            && frame(rig, b).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, b, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, b, "FIRE");
        false
    })
    .await;
    assert!(
        fell,
        "B's fire takes the carrier's hull; B sees {:?}",
        frame(&rig, b).map(|frame| frame
            .contacts
            .iter()
            .map(|c| (c.designator.to_string(), c.cell))
            .collect::<Vec<_>>())
    );
    let demoted = until(&mut rig, &mut rx, &mut lines, 90, |rig, _| {
        task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
    })
    .await;
    assert!(demoted, "the field demotes after its quiet interval");

    assert!(wrecked(&rig, carrier), "the carrier is a wreck");
    let record = site_record(&mut rig).expect("the record");
    assert!(
        record
            .units
            .iter()
            .any(|left| left.unit == carrier && left.condition == LeftCondition::Wreck),
        "the carrier is a wreck on the index: {:?}",
        record.units
    );
    for hull in &raiders {
        assert!(
            wrecked(&rig, *hull),
            "#{} is lost with its carrier: a wreck",
            hull.0
        );
        assert!(
            mush_sim::pose_of(rig.tw.world(), *hull).is_none()
                && tacsim::field::raiders::carrier_of(rig.tw.world(), *hull) == Some(carrier),
            "#{} stays aboard the wreck",
            hull.0
        );
        assert!(
            rig.tw.world().get(*hull).is_some(),
            "#{} is never deleted",
            hull.0
        );
    }
    let again = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
    assert!(
        again.contains("nobody aboard"),
        "a wreck aboard never launches: {again}"
    );
}
