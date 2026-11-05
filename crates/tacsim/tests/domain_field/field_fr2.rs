//! **Doc 40 cut FR.2 — durable standing** (Q-38-24, Q-40-15..18), over the composed world
//! (`driving`'s keyed rig), the arena's egress pumped by hand as `field_gc10.rs` pumps it.
//!
//! - **GC.10's void gate, at both sizes, with a restart mid-field** (Q-40-15): A fires on B, a
//!   fellow member of the Skulks' organization; once a snapshot holds the round the world
//!   restarts, the field resumes with the tally on its live record, and at demotion A's standing
//!   falls by the bounded amount, the change naming A as its voider and carrying its window.
//! - **The synchronous close** (Q-40-16, Q-40-17): a `+field/void` with no command of A's
//!   between the demotion and it is accepted; one typed after another command is refused at
//!   once, and the change stands.
//! - **The director's auto-void** (Q-40-18): a raid the director landed on a Syndicate member's
//!   ground fires on it, and at demotion the raid's leader has voided its own friendly fire, with
//!   no hand.
//!
//! **What is a fixture.** As GC.10's: the armed hulls are made and seated by staff hand, the
//! memberships written by `@faction/add`, the raider seeded by `@field/raiders` (or, for the
//! director, `raiders::seed` and a lift, as GC.11's gate). Everything after is the production
//! field.

use mush_core::{GOD, ObjId, ObjKind};
use mush_inventory::LotId;
use tacsim::TacsimConfig;
use tacsim::arena::PartyId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::field::standing::{NOTHING_TO_VOID, STANDING_FIELD_CAP, cause_prefix};
use tacsim::field::{FieldParty, PartyKey};

use crate::driving_rig::{
    AHUNA, Rig, act_go, cid, command, crash, finish, land, launched, on_pad, on_pad_as_at, run,
    seed_pool, staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_host_rig::{
    armed_on, egress, frame, seat, second, site_record_at, task, tune, until, weapon_of,
};

/// The raider the content ships on Ahuna Flank, and its organization (GC.11).
const SKULKS: &str = "tacsim.raider.ahuna_skulks";
/// The Skulks' organization.
const SYNDICATE: &str = "Skulk Syndicate";

/// **One size the gate runs at** (doc 43 rule 5): the site, its `+ship/undock` word, and A's and
/// B's hulls four cells apart on one row, A facing B.
struct At {
    entry: &'static str,
    venue: &'static str,
    a: (i64, i64),
    b: (i64, i64),
}

/// Ahuna Flank at 256 a side: GC.10's own cells.
const AHUNA_FLANK: At = At {
    entry: AHUNA,
    venue: "ahuna",
    a: (110, 124),
    b: (114, 124),
};

/// Urvara Reach at 512 a side: row 268 across the 256 line (`field_story`'s fight).
const URVARA_REACH: At = At {
    entry: "urvara_port",
    venue: "urvara",
    a: (250, 268),
    b: (254, 268),
};

/// What A types once the field has demoted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    /// `+field/void` first: accepted.
    Void,
    /// Another command, then `+field/void`: refused at once.
    OtherFirst,
}

/// The parties on the site's ground now, and the Friends among them.
fn friends(rig: &mut Rig, entry: &str) -> (Vec<FieldParty>, Vec<(PartyId, PartyId)>) {
    staff_hand(rig, |ctx| {
        tacsim::field::friends_on_ground(&*ctx, &cid(entry))
    })
}

/// The letter of the party `key` names.
fn letter(parties: &[FieldParty], key: impl Fn(&PartyKey) -> bool) -> PartyId {
    parties
        .iter()
        .find(|party| key(&party.key))
        .map(|party| party.party)
        .unwrap_or_else(|| panic!("no such party in {parties:?}"))
}

/// A faction's id by name.
fn faction(rig: &Rig, name: &str) -> mush_faction::FactionId {
    rig.tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| state.faction_by_name(name))
        .map(|faction| faction.id)
        .unwrap_or_else(|| panic!("the faction {name}"))
}

/// `who`'s public score with `faction`.
fn score(rig: &Rig, faction: mush_faction::FactionId, who: ObjId) -> i16 {
    mush_faction::public_standing(rig.tw.world(), faction, who).map_or(0, |view| view.score)
}

/// `who` joins `faction` openly.
fn join(rig: &mut Rig, faction: &str, who: ObjId) {
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@faction/add {faction}=#{}", who.0),
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_faction::FactionState>()
            .is_some_and(|state| state.is_member(who, self::faction(rig, faction))),
        "#{} joins {faction}: {out}",
        who.0
    );
}

/// The rounds A's party has landed on B's, as the live record holds them (the last snapshot's).
fn tallied(rig: &Rig, entry: &str, pa: PartyId, pb: PartyId) -> u32 {
    tacsim::field::live_on(rig.tw.world(), &cid(entry))
        .map(|live| {
            live.tally
                .iter()
                .filter(|row| row.shooter == pa && row.victim == pb)
                .map(|row| row.rounds)
                .sum()
        })
        .unwrap_or(0)
}

/// **GC.10's void gate at `at`, with a restart mid-field**, then `then`.
#[allow(clippy::too_many_lines)] // one field, in order
async fn void_gate(at: &At, then: Then) {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 5;
        cfg.field.idle_demote_s = 8;
        // The black box's 16 KiB chunks close when full: summaries every pulse fill one in a few
        // arena seconds, so the round's chunk reaches the host (and the tally a snapshot) before
        // the restart, which is the case the durable tally is for.
        cfg.arena.ledger.summary_ms = 100;
    });
    let (a, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (at.venue, at.entry), 1);
    let (b, _, _) = on_pad_as_at(
        &mut rig,
        ("Other", 2, "Tally Moth"),
        (at.venue, at.entry),
        1,
    );
    let hull_a = armed_on(&mut rig, at.entry, "Red Hound", at.a, 90_000);
    let hull_b = armed_on(&mut rig, at.entry, "Grey Hound", at.b, 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    join(&mut rig, SYNDICATE, a);
    join(&mut rig, SYNDICATE, b);
    let syndicate = faction(&rig, SYNDICATE);
    let (parties, pairs) = friends(&mut rig, at.entry);
    let pa = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_a });
    let pb = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_b });
    assert!(
        pairs.contains(&(pa.min(pb), pa.max(pb))),
        "A and B are friends by the Syndicate: {pairs:?}"
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // A's fire order at B's hull to the east opens the field; the round reaches B, and a
    // snapshot lays it on the live record.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let gun = weapon_of(&rig, hull_a);
    let rounds = |rig: &Rig| {
        frame(rig, a).and_then(|frame| {
            frame
                .weapons
                .iter()
                .find(|row| row.id.as_str() == gun)
                .map(|row| row.rounds)
        })
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| rounds(rig)
            .is_some())
        .await,
        "A's cockpit frame arrives"
    );
    let entry = at.entry;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 40, |rig, _| tallied(
            rig, entry, pa, pb
        ) > 0)
        .await,
        "the round reaches B and a snapshot lays the tally on the live record"
    );
    let before = tallied(&rig, entry, pa, pb);

    // The restart, mid-field: the field resumes from its snapshot, the tally with it.
    let mut rig = crash(rig, &mut rx, &[a, b]).await;
    let mut rx = egress(&mut rig);
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some()
            && tacsim::field::live_on(rig.tw.world(), &cid(entry))
                .is_some_and(|live| live.segment > 1))
        .await,
        "the field resumes after the restart"
    );
    assert!(
        tallied(&rig, entry, pa, pb) >= before,
        "the tally survives the restart: {} before, {} after",
        before,
        tallied(&rig, entry, pa, pb)
    );

    // Both leaders stand down with a Friendly mark, and the field demotes on its idle bound.
    let (out, _) = command(
        &mut rig,
        a,
        &format!("+field/mark {}=friendly", pb.letter()),
    );
    assert!(!out.contains("Usage"), "{out}");
    let (out, _) = command(
        &mut rig,
        b,
        &format!("+field/mark {}=friendly", pa.letter()),
    );
    assert!(!out.contains("Usage"), "{out}");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, _| task(rig)
            .is_none()
            && site_record_at(rig, entry)
                .is_some_and(|record| record.open_field().is_none()))
        .await,
        "the field demotes"
    );

    // The change: A's, bounded, naming A as its voider, its window written with it.
    let prefix = cause_prefix(&cid(entry), 1);
    let causes = mush_faction::standing_causes(rig.tw.world(), &prefix);
    assert_eq!(causes.len(), 1, "one change, A's: {causes:?}");
    let receipt = mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
    assert_eq!(receipt.subject, a);
    assert_eq!(receipt.voidable_by, Some(a), "friendly fire: A may void it");
    assert!(receipt.window.is_some(), "the window rides the grant");
    assert_eq!(receipt.voided_by, None);
    assert_eq!(receipt.effects.len(), 1, "{:?}", receipt.effects);
    assert_eq!(receipt.effects[0].faction, syndicate);
    let delta = receipt.effects[0].delta;
    assert!(
        (STANDING_FIELD_CAP..=-1).contains(&delta),
        "bounded: {delta}"
    );
    assert_eq!(score(&rig, syndicate, a), delta);
    assert_eq!(score(&rig, syndicate, b), 0, "B fired nothing");
    // A wall second past the demotion, so what A types next is after the offer opened.
    let _ = second(&mut rig);

    // B may not void A's change.
    let refused = run(&mut rig.tw, b, "+field/void");
    assert!(refused.contains(NOTHING_TO_VOID), "{refused}");
    assert_eq!(score(&rig, syndicate, a), delta);
    match then {
        Then::Void => {
            let voided = run(&mut rig.tw, a, "+field/void");
            assert!(voided.contains("Voided as friendly fire"), "{voided}");
            assert_eq!(score(&rig, syndicate, a), 0, "the void cancels the change");
            let receipt =
                mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
            assert_eq!(receipt.voided_by, Some(a), "recorded beside the change");
            let again = run(&mut rig.tw, a, "+field/void");
            assert!(again.contains(NOTHING_TO_VOID), "{again}");
            let history = run(&mut rig.tw, a, &format!("+standing {SYNDICATE}"));
            assert!(history.contains("(voided)"), "{history}");
        }
        Then::OtherFirst => {
            let other = run(&mut rig.tw, a, "+field");
            assert!(!other.contains("Voided"), "{other}");
            let late = run(&mut rig.tw, a, "+field/void");
            assert!(
                late.contains(NOTHING_TO_VOID),
                "the other command closed the offer: {late}"
            );
            assert_eq!(score(&rig, syndicate, a), delta, "the change stands");
            let receipt =
                mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
            assert_eq!(receipt.voided_by, None);
        }
    }
}

/// **FR.2 at 256**: the void with no command between is accepted, after a restart mid-field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr2_ahuna_the_tally_survives_a_restart_and_the_void_is_accepted() {
    void_gate(&AHUNA_FLANK, Then::Void).await;
}

/// **FR.2 at 256**: a void after another command is refused at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr2_ahuna_a_void_after_another_command_is_refused() {
    void_gate(&AHUNA_FLANK, Then::OtherFirst).await;
}

/// **FR.2 at 512**: the void with no command between is accepted, after a restart mid-field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr2_urvara_the_tally_survives_a_restart_and_the_void_is_accepted() {
    void_gate(&URVARA_REACH, Then::Void).await;
}

/// **FR.2 at 512**: a void after another command is refused at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr2_urvara_a_void_after_another_command_is_refused() {
    void_gate(&URVARA_REACH, Then::OtherFirst).await;
}

/// **Ration `unit`'s magazines** to at most `rounds` each (GC.11's guard, a staff hand).
fn rationed(rig: &mut Rig, unit: ObjId, rounds: i64) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter(|(addr, _)| {
                matches!(addr, mush_tactics::CellAddr::Component { cell, .. }
                    if cell.as_str() == tacsim::definitions::unit::AMMO_CELL)
            })
            .map(|(addr, cell)| mush_tactics::UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: cell.current.min(rounds).max(cell.min),
            })
            .collect();
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:fr2-rationed")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// **FR.2's director gate** (Q-40-18): GC.11's tempting site, its prospector and its hired guard
/// both Syndicate members; the director lands the Skulks, their leader declares, and the raid's
/// fire on its own organization's members is voided at demotion by its leader, with no hand.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one raid, in order
async fn fr2_a_director_raids_friendly_fire_is_voided_with_no_hand() {
    use crate::field_raiders_rig as raiders_rig;
    let mut rig = stocked_tuned(|cfg: &mut TacsimConfig| {
        cfg.arena.tempo.quiet_ms = 100;
        cfg.arena.tempo.alert_ms = 100;
        cfg.arena.tempo.engaged_ms = 100;
        cfg.arena.snapshot_ms = 500;
        cfg.field.disengage_s = 10;
        cfg.field.idle_demote_s = 10;
        cfg.field.raid_director = true;
        cfg.arena.ledger.summary_ms = 100;
    });
    // GC.11's raider, raided before and lifted, so the director sends it as it is.
    let def = tacsim::field::raiders::raiders()
        .expect("the fixture raiders")
        .into_iter()
        .find(|def| def.id.as_str() == SKULKS)
        .expect("the content's raider");
    let seeded = staff_hand(&mut rig, |ctx| tacsim::field::raiders::seed(ctx, &def));
    assert!(seeded.is_ok(), "{seeded:?}");
    let hulls = raiders_rig::raider_hulls(&rig);
    // As GC.11's gate: one hull came home worn thin, so the guard's six rounds take it and the
    // raid, having lost half, turns for home.
    raiders_rig::fragile(&mut rig, hulls[1]);
    let barge = hulls
        .first()
        .and_then(|hull| tacsim::field::raiders::carrier_of(rig.tw.world(), *hull))
        .expect("the raider is seeded");
    staff_hand(&mut rig, |ctx| {
        mush_sim::unplace(ctx, barge, "test: lifted").expect("lifted");
    });
    // GC.11's tempting site: two rigs out east of the pad, ore in one, a hired guard further east.
    let (a, _, tvs) = on_pad(&mut rig, 2);
    launched(&mut rig, a, tvs[0]);
    to_cell(&mut rig, tvs[0], (190, 126));
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, tvs[1], a);
    to_cell(&mut rig, tvs[1], (190, 132));
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), tvs[0]).expect("a hopper");
    seed_pool(&mut rig, hopper, "metal_ore", 3, LotId(9_921));
    let guard = raiders_rig::armed(&mut rig, "Ward Dog", (183, 126), 270_000);
    let npc = staff_hand(&mut rig, |ctx| {
        ctx.w
            .create(ObjKind::Npc, "Hired guard", GOD, None)
            .expect("the NPC")
    });
    raiders_rig::seat_npc(&mut rig, guard, npc);
    rationed(&mut rig, guard, 6);
    warm(&mut rig);
    // Everyone the raid may fire on is of its own organization.
    join(&mut rig, SYNDICATE, a);
    join(&mut rig, SYNDICATE, npc);
    let syndicate = faction(&rig, SYNDICATE);
    let mut rx = raiders_rig::egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // The director lands it, its leader declares, and the field opens.
    assert!(
        raiders_rig::until(&mut rig, &mut rx, &mut lines, 40, |rig, _| {
            raiders_rig::task(rig).is_some()
        })
        .await,
        "the director's raid opens a field"
    );
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), barge).expect("a leader");
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    assert!(
        live.parties
            .iter()
            .any(|p| p.key == PartyKey::Fixture { leader } && p.director),
        "the raid's party is marked the director's: {:?}",
        live.parties
    );
    rig.tw.world_mut().set_connected(a, false);

    // The raid fights, turns for home, and the field settles.
    let settled = raiders_rig::until(&mut rig, &mut rx, &mut lines, 400, |rig, _| {
        raiders_rig::site_record(rig).is_some_and(|record| record.promoted.is_none())
    })
    .await;
    assert!(settled, "the field settles");

    // The raid leader's changes: friendly fire, each voided by its own leader at the demotion.
    let prefix = cause_prefix(&cid(AHUNA), 1);
    let own: Vec<mush_faction::StandingReceipt> =
        mush_faction::standing_causes(rig.tw.world(), &prefix)
            .iter()
            .filter_map(|cause| mush_faction::standing_receipt(rig.tw.world(), cause))
            .filter(|receipt| receipt.subject == leader)
            .collect();
    let impacts: Vec<(ObjId, ObjId)> = lines
        .iter()
        .filter_map(|line| match line.kind {
            tacsim::arena::ledger::EventKind::Impact { unit, target, .. }
                if hulls.contains(&unit) =>
            {
                Some((unit, target))
            }
            _ => None,
        })
        .collect();
    assert!(
        !own.is_empty(),
        "the raid's fire reached a Syndicate member: causes {:?}; the raiders' impacts {impacts:?}",
        mush_faction::standing_causes(rig.tw.world(), &prefix)
    );
    for receipt in &own {
        assert_eq!(
            receipt.voidable_by,
            Some(leader),
            "friendly fire: {receipt:?}"
        );
        assert_eq!(
            receipt.voided_by,
            Some(leader),
            "voided by the raid's leader with no hand: {receipt:?}"
        );
    }
    assert_eq!(
        score(&rig, syndicate, leader),
        0,
        "nothing of the raid's friendly fire stands"
    );
}
