//! **Doc 45 unit AP.9: space reads the same posture** (Q-45-23, Q-45-24; Q-45-7, Q-45-8,
//! Q-45-12), on the fitted excursions rig, which flies the shipped lane: a passage from Ceres
//! to Stickney crosses the Kirkwood customs picket, whose bubble holds the customs corvette
//! and the belt raider beside every carrier that comes through.
//!
//! Every act is a registered command typed by a player who is not staff, and the world's own
//! ticks. Nothing is staged by fiat. What the stories cannot reach by a route in play is said
//! in the cut note (`docs/design/45-cut-notes.md`, AP.9): two hulls of one player in one
//! bubble, so the one case `GUARD` adds to `HOLD` is pinned at the seat
//! (`space::applier`, `space::backup`).
//!
//! **For doc 45 unit AP.11.** [`left_on_posture_to_the_picket`] puts one player's ship on a
//! posture into the picket's bubble with her crew's link down, the corvette and the raider
//! beside her (and any other pilot the story has sailed), and [`ship_at_the_picket`] reads
//! what she did: her lock, the ships her fire has arrived on, the ships whose fire has
//! arrived on her, what customs made of her and what the seat last did. [`until_ship`] steps
//! the world a second at a time until a reading holds.

use std::collections::BTreeSet;

use mush_core::ObjId;
use tacsim::posture::Posture;
use tacsim::posture::order::{
    POSTURE_KEPT, POSTURE_OOC, POSTURE_READ, POSTURE_SET, POSTURE_SHIP_ABOARD, POSTURE_SHIP_LINE,
    POSTURE_SHIP_USAGE, POSTURE_WORDS, line,
};
use tacsim::space::{BackupAct, PicketInspectionStatus, SpaceBubbleBook};

use crate::excursions::{Rig, docked_dray, run};

/// Drop every connection `who` holds, uncleanly, so the presence hook reads a lost link.
pub(crate) fn drop_link(rig: &mut Rig, who: ObjId) {
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
}

/// One wall second on, everything due fired and what it queued landed.
pub(crate) fn one_second(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = crate::excursions::drain(&mut rig.tw);
    crate::excursions::land_quietly(rig, &queued);
}

/// Jump past the link-loss grace at the play rate, and one second more for the mind's first
/// think to land, then go back to the stories' rate. The window is not waited out.
pub(crate) fn wait_out_the_grace(rig: &mut Rig) {
    crate::excursions::set_rate(&mut rig.tw, 24);
    let past = rig
        .tw
        .world()
        .now()
        .saturating_add(tacsim::space::SPACE_LINKDEAD_GRACE_WALL_S + 10);
    rig.tw.set_now(past);
    let _ = rig.tw.fire_due(past.saturating_mul(1_000));
    let queued = crate::excursions::drain(&mut rig.tw);
    crate::excursions::land_quietly(rig, &queued);
    one_second(rig);
    crate::excursions::set_rate(&mut rig.tw, 3_600);
}

/// The posture `unit`'s own record holds: the one read the field and the bubble both use.
pub(crate) fn posture_of(rig: &Rig, unit: ObjId) -> Posture {
    tacsim::posture::record::of(rig.tw.world(), unit)
}

/// **Her commander sets her posture from aboard her**, by `+ship/posture <word>`.
pub(crate) fn set_ship_posture(rig: &mut Rig, pilot: ObjId, hull: ObjId, posture: Posture) {
    let said = run(
        &mut rig.tw,
        pilot,
        &format!("+ship/posture {}", posture.as_str()),
    );
    assert!(
        said.contains(&format!("holds {}.", posture.label())),
        "the posture is set from aboard her: {said}"
    );
    assert_eq!(posture_of(rig, hull), posture);
}

/// **What a story reads of an absent player's ship at the Kirkwood picket.** Ships are named
/// by their ids in the bubble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShipAtPicket {
    /// Her own id in the bubble.
    pub(crate) ship: String,
    /// The belt raider's.
    pub(crate) raider: String,
    /// The customs corvette's.
    pub(crate) corvette: String,
    /// Her lock.
    pub(crate) lock: Option<String>,
    /// The ships her fire has arrived on since the bubble opened.
    pub(crate) answered: BTreeSet<String>,
    /// The ships whose fire has arrived on her since the bubble opened.
    pub(crate) fired_on_by: BTreeSet<String>,
    /// Her commanded thrust, mm/s².
    pub(crate) thrust: [i64; 3],
    /// What customs have made of her.
    pub(crate) status: PicketInspectionStatus,
    /// The last word her mind traced.
    pub(crate) trace: Option<String>,
    /// What the seat last did for her.
    pub(crate) act: Option<BackupAct>,
}

impl ShipAtPicket {
    /// Whether the raider's fire has arrived on her.
    pub(crate) fn hit_by_the_raider(&self) -> bool {
        self.fired_on_by.contains(&self.raider)
    }

    /// Whether customs have let her go.
    pub(crate) fn cleared(&self) -> bool {
        matches!(
            self.status,
            PicketInspectionStatus::Cleared
                | PicketInspectionStatus::ClearedAfterFine
                | PicketInspectionStatus::ClearedAfterSeizure
        )
    }
}

/// **Read `hull` at the picket**; `None` once she is in no picket's bubble.
pub(crate) fn ship_at_the_picket(rig: &Rig, hull: ObjId) -> Option<ShipAtPicket> {
    let book = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .expect("the space book");
    let active = book.active_for_hull(hull)?;
    let picket = active.picket.as_ref()?;
    let ship_id = active.carrier_ship_id_for(hull).to_owned();
    let ship = active.bubble.ships.get(&ship_id)?;
    let linkdead = book.linkdead_state(hull);
    Some(ShipAtPicket {
        raider: picket.raider_ship_id.clone(),
        corvette: picket.corvette_ship_id.clone(),
        lock: ship.locked_target_id.clone(),
        answered: active
            .bubble
            .fired_on
            .iter()
            .filter(|(_, by)| by.contains(&ship_id))
            .map(|(target, _)| target.clone())
            .collect(),
        fired_on_by: active
            .bubble
            .fired_on_by(&ship_id)
            .cloned()
            .unwrap_or_default(),
        thrust: ship.thrust_mmps2,
        status: picket.inspection_status_for(hull),
        trace: linkdead.and_then(|state| state.mind_last_trace.clone()),
        act: linkdead.map(|state| state.backup_act),
        ship: ship_id,
    })
}

/// Step a second at a time until `done` reads true of her at the picket, at most `seconds`.
/// Answers her last reading; `None` if she has left the picket.
pub(crate) fn until_ship(
    rig: &mut Rig,
    hull: ObjId,
    seconds: u32,
    mut done: impl FnMut(&ShipAtPicket) -> bool,
) -> Option<ShipAtPicket> {
    for _ in 0..seconds {
        let now = ship_at_the_picket(rig, hull)?;
        if done(&now) {
            return Some(now);
        }
        one_second(rig);
    }
    ship_at_the_picket(rig, hull)
}

/// **Sail `hull` for Stickney on `posture` and lose her crew's link on the corridor** (the
/// module head, "for doc 45 unit AP.11"): her commander sets the posture from aboard her and
/// files the passage; the link drops and the grace is jumped, so a mind has her helm. Answers
/// the IC instant her first leg ends, where the picket's bubble opens on her. A story that
/// wants another pilot beside her sails that pilot first.
fn sail_on_posture_and_lose_the_link(
    rig: &mut Rig,
    pilot: ObjId,
    hull: ObjId,
    posture: Posture,
) -> i64 {
    set_ship_posture(rig, pilot, hull, posture);
    crate::excursions::set_rate(&mut rig.tw, 3_600);
    let ticket = crate::excursions::file_and_launch(rig, pilot, hull, "+ship/undock stickney");
    let boundary = ticket.plan.depart_ic + i64::from(ticket.plan.legs[0].secs);
    drop_link(rig, pilot);
    wait_out_the_grace(rig);
    let held = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_state(hull))
        .cloned()
        .expect("her link-loss record");
    assert!(held.backup_has_helm(), "past the grace a mind has her helm");
    boundary
}

/// **One player's ship, left on `posture`, in the picket's bubble with her crew's link
/// down** (the module head, "for doc 45 unit AP.11"). `not_before` is an IC instant the
/// story must also reach, the end of another pilot's first leg when one sails beside her and
/// nought otherwise. Answers her first reading there.
pub(crate) fn left_on_posture_to_the_picket(
    rig: &mut Rig,
    pilot: ObjId,
    hull: ObjId,
    posture: Posture,
    not_before: i64,
) -> ShipAtPicket {
    let boundary = sail_on_posture_and_lose_the_link(rig, pilot, hull, posture);
    let _ = crate::excursions::to_ic(rig, boundary.max(not_before));
    ship_at_the_picket(rig, hull).expect("the Kirkwood picket's bubble opens on her")
}

/// **The verb, and one record for a ship and a rig** (the row's first clause, its record
/// half). One player on a pad with a Dray-P and a rig sets both to `HOLD`, the ship by
/// `+ship/posture` and the rig by `+tv/1 posture`, and both read the same word off the same
/// record. The ship's is read anywhere, set only from aboard her and never from an OOC
/// room; `+ship` shows it; a restart keeps both.
///
/// What flies each half: the rig's `HOLD` in a field is doc 45 units AP.6 and AP.7's stories
/// (`field_ap6`, `field_ap7`), which read this record; the ship's is the stories below.
#[test]
fn ap9_one_player_sets_a_ship_and_a_rig_to_hold_by_the_same_word_on_the_same_record() {
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row, tvs) = crate::excursions::on_pad_as(&mut rig, "Miner", 2, "Tally Moth", 1);
    let hull = ObjId(row.hull);
    let tv = tvs[0];
    let name = "The Tally Moth";

    // The default, read.
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture"),
        line(POSTURE_READ, name, Posture::Return)
    );
    let status = run(&mut rig.tw, pilot, "+ship");
    assert!(
        !status.contains("Posture:"),
        "the default is not shown: {status}"
    );

    // Words that are none, and too many.
    assert_eq!(run(&mut rig.tw, pilot, "+ship/posture flee"), POSTURE_WORDS);
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture hold fast"),
        POSTURE_SHIP_USAGE
    );
    assert_eq!(posture_of(&rig, hull), Posture::Return);

    // Set from aboard her, and the rig by its own verb: the same word on the same record.
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture hold"),
        line(POSTURE_SET, name, Posture::Hold)
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture HOLD"),
        line(POSTURE_KEPT, name, Posture::Hold)
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+tv/1 posture hold"),
        line(POSTURE_SET, "Rig 1", Posture::Hold)
    );
    assert_eq!(
        (posture_of(&rig, hull), posture_of(&rig, tv)),
        (Posture::Hold, Posture::Hold),
        "a ship and a rig of one player hold the same word"
    );
    let status = run(&mut rig.tw, pilot, "+ship");
    assert!(
        status.contains(&POSTURE_SHIP_LINE.replace("{posture}", "HOLD")),
        "her status shows it: {status}"
    );

    // A restart keeps both.
    let rig = crate::excursions::reboot_with(rig, &[pilot]);
    assert_eq!(
        (posture_of(&rig, hull), posture_of(&rig, tv)),
        (Posture::Hold, Posture::Hold)
    );

    // Off her, in the world: read, not set. A pilot docked at Ceres sets `GUARD` aboard,
    // cycles out into the berth gallery, and from there can read her word and not change it.
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row) = docked_dray(&mut rig, ("Pilot", 1, "Salt Moth"), (53, 19));
    let hull = ObjId(row.hull);
    let name = "The Salt Moth";
    set_ship_posture(&mut rig, pilot, hull, Posture::Guard);
    let left = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(
        tacsim::space::hull_aboard_for_actor(rig.tw.world(), pilot) != Some(hull),
        "she is off her ship: {left}"
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture hold"),
        POSTURE_SHIP_ABOARD.replace("{unit}", name)
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/posture"),
        line(POSTURE_READ, name, Posture::Guard)
    );
    // From an OOC room: refused as an order to a unit is.
    crate::excursions::teleport(&mut rig.tw, pilot, mush_core::LIMBO);
    assert_eq!(run(&mut rig.tw, pilot, "+ship/posture hold"), POSTURE_OOC);
    assert_eq!(posture_of(&rig, hull), Posture::Guard);
}

/// What became of a hull left at a cluster with her crew's link down.
#[derive(Debug)]
struct Adrift {
    act: BackupAct,
    trace: Option<String>,
    at_her_cluster: bool,
}

/// A prospector sets `posture`, hops to the Ceres halo, and loses the link there.
fn left_at_a_cluster(posture: Posture) -> Adrift {
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row) =
        crate::excursions::docked_prospector(&mut rig, ("Miner", 2, "Tally Moth"), (75, 90));
    let hull = ObjId(row.hull);
    set_ship_posture(&mut rig, pilot, hull, posture);
    crate::excursions::set_rate(&mut rig.tw, 3_600);
    let (filed, burn) =
        crate::excursions::command(&mut rig, pilot, "+ship/undock ceres_halo throttle=500");
    assert!(filed.contains("Clearance filed for"), "{filed}");
    crate::excursions::land_quietly(&mut rig, &burn);
    let (_, queued) = crate::excursions::fire(&mut rig);
    crate::excursions::land_quietly(&mut rig, &queued);
    let arrival = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.free_flights.get(&hull))
        .and_then(|flight| flight.cluster_arrival_ic_s);
    if let Some(arrival) = arrival {
        let _ = crate::excursions::to_ic(&mut rig, arrival);
        let (_, queued) = crate::excursions::fire(&mut rig);
        crate::excursions::land_quietly(&mut rig, &queued);
    }
    let at_cluster = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<SpaceBubbleBook>()
            .is_some_and(|book| book.cluster_sessions.contains_key(&hull))
    };
    assert!(
        at_cluster(&rig),
        "she is at her cluster with her pilot aboard"
    );
    drop_link(&mut rig, pilot);
    wait_out_the_grace(&mut rig);
    for _ in 0..5 {
        one_second(&mut rig);
    }
    let state = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_state(hull))
        .cloned()
        .expect("her link-loss record");
    assert!(state.backup_has_helm());
    Adrift {
        act: state.backup_act,
        trace: state.mind_last_trace,
        at_her_cluster: at_cluster(&rig),
    }
}

/// **A ship left adrift holds station on `HOLD` and on `GUARD`, and makes for port on
/// `RETURN`** (Q-45-23, Q-45-7). The same hull, at the same cluster, with the same link
/// loss: only the word her commander left on her differs.
#[test]
fn ap9_a_ship_left_at_a_cluster_holds_station_on_hold_and_guard_and_makes_for_port_on_return() {
    let home = left_at_a_cluster(Posture::Return);
    assert_eq!(home.act, BackupAct::MakingForPort, "{home:?}");
    assert_eq!(
        home.trace.as_deref(),
        Some("BACKUP_MAKE_FOR_PORT"),
        "{home:?}"
    );
    assert!(!home.at_her_cluster, "on RETURN she has left: {home:?}");

    for posture in [Posture::Hold, Posture::Guard] {
        let stays = left_at_a_cluster(posture);
        assert_eq!(stays.act, BackupAct::HoldingStation, "{posture}: {stays:?}");
        assert_eq!(
            stays.trace.as_deref(),
            Some("BACKUP_HOLD_STATION"),
            "{posture}: {stays:?}"
        );
        assert!(
            stays.at_her_cluster,
            "{posture}: she is where she was: {stays:?}"
        );
    }
}

/// What a ship left on a posture did at the picket when the raider's fire arrived on her.
#[derive(Debug)]
struct Answered {
    /// Whether she had locked or fired on anything before the raider's fire arrived.
    acted_first: bool,
    /// Her reading when the raider's fire first arrived on her.
    hit: ShipAtPicket,
    /// Her reading a minute later.
    after: ShipAtPicket,
    /// The stranger's ship id.
    stranger: String,
}

/// **The raider fires on her, with a stranger beside her.** A pilot leaves their ship on
/// `posture` and loses the link on the corridor; a second pilot sails beside them and does
/// nothing at the picket. The absent ship's mind heaves her to, customs clear her, the
/// raider lights and opens on her. Answers the rig as it stands a minute after.
fn hit_by_the_raider(posture: Posture) -> (Rig, ObjId, ObjId, Answered) {
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row) = docked_dray(&mut rig, ("Pilot", 1, "Salt Moth"), (53, 19));
    let (stranger, other) = docked_dray(&mut rig, ("Stranger", 2, "Tin Kettle"), (53, 19));
    let (hull, beside) = (ObjId(row.hull), ObjId(other.hull));

    // The stranger sails first, then the pilot, who loses the link on the corridor.
    crate::excursions::set_rate(&mut rig.tw, 3_600);
    let theirs =
        crate::excursions::file_and_launch(&mut rig, stranger, beside, "+ship/undock stickney");
    let their_boundary = theirs.plan.depart_ic + i64::from(theirs.plan.legs[0].secs);
    let first = left_on_posture_to_the_picket(&mut rig, pilot, hull, posture, their_boundary);
    let stranger_ship = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.active_for_hull(beside))
        .map(|active| active.carrier_ship_id_for(beside).to_owned())
        .expect("the stranger is in the same bubble");
    assert_ne!(stranger_ship, first.ship);

    let mut acted_first = false;
    let hit = until_ship(&mut rig, hull, 900, |now| {
        if !now.hit_by_the_raider() && (now.lock.is_some() || !now.answered.is_empty()) {
            acted_first = true;
        }
        now.hit_by_the_raider()
    })
    .expect("she is still at the picket");
    assert!(
        hit.hit_by_the_raider(),
        "the raider's fire arrives on her: {hit:?}"
    );
    // A minute more, or until the picket lets her go: her last reading there.
    let mut after = hit.clone();
    for _ in 0..60 {
        one_second(&mut rig);
        match ship_at_the_picket(&rig, hull) {
            Some(now) => after = now,
            None => break,
        }
    }
    (
        rig,
        pilot,
        hull,
        Answered {
            acted_first,
            hit,
            after,
            stranger: stranger_ship,
        },
    )
}

/// **The same ship, hit by the same hull, answers on `HOLD` and on `GUARD` and not on
/// `RETURN`; she never fires first, and never on the corvette or the stranger** (the row's
/// second and third clauses; Q-45-8, Q-45-12, Q-44-38).
///
/// Three parties beside her in the bubble: the customs corvette, the belt raider, which her
/// party holds hostile from the bubble's opening, and a second pilot's ship.
///
/// On `HOLD` the story goes on: she complied with customs before the raider lit (she hove
/// to by her mind's own burn and was cleared, and never ran), and once the picket lets her
/// go she is on her filed passage again, across a restart.
#[test]
fn ap9_at_the_picket_a_ship_answers_the_raider_that_fired_on_her_by_her_posture() {
    for posture in [Posture::Return, Posture::Guard, Posture::Hold] {
        let (mut rig, _pilot, hull, told) = hit_by_the_raider(posture);
        assert!(
            !told.acted_first,
            "{posture}: she locked and fired on nobody before she was fired on, though her \
             party holds the raider hostile: {told:?}"
        );
        assert!(
            told.hit.cleared(),
            "{posture}: she hove to and was cleared before the raider opened on her: {told:?}"
        );
        let raider = BTreeSet::from([told.hit.raider.clone()]);
        // Holding station inside a bubble: once customs have let her go her drive is cut,
        // under every word, and her mind's last word is its own.
        assert_eq!(told.after.thrust, [0, 0, 0], "{posture}: {told:?}");
        assert_eq!(
            told.after.trace.as_deref(),
            Some("BACKUP_COAST"),
            "{posture}: {told:?}"
        );
        if posture.is_return() {
            assert!(
                told.after.answered.is_empty() && told.after.lock.is_none(),
                "on RETURN she answers nobody: {told:?}"
            );
            continue;
        }
        assert_eq!(
            told.after.answered, raider,
            "{posture}: she answers the raider and nobody else: {told:?}"
        );
        assert_eq!(
            told.after.lock.as_deref(),
            Some(told.hit.raider.as_str()),
            "{posture}: {told:?}"
        );
        assert!(
            !told.after.answered.contains(&told.hit.corvette)
                && !told.after.answered.contains(&told.stranger),
            "{posture}: {told:?}"
        );
        assert_eq!(
            told.after.act,
            Some(BackupAct::ReturningFire),
            "{posture}: {told:?}"
        );
        assert_ne!(told.after.status, PicketInspectionStatus::RanThePicket);
        if posture != Posture::Hold {
            continue;
        }

        // On HOLD: the picket lets her go, and she is on her passage again.
        let mut left = false;
        for _ in 0..1_800 {
            if ship_at_the_picket(&rig, hull).is_none() {
                left = true;
                break;
            }
            one_second(&mut rig);
        }
        assert!(
            left,
            "the picket lets her go: {:?}",
            ship_at_the_picket(&rig, hull)
        );
        let under_way = |rig: &Rig| {
            matches!(
                tacsim::port::ship::berth_of(rig.tw.world(), hull),
                tacsim::port::ship::Berth::UnderWay
            )
        };
        assert!(under_way(&rig), "she is on her filed passage again");
        // A restart under way, after the picket: she is still on it, on the same word.
        let rig = crate::excursions::reboot_with(rig, &[]);
        assert!(under_way(&rig), "and after a restart");
        assert_eq!(posture_of(&rig, hull), Posture::Hold);
    }
}

/// **A ship left on `HOLD` or on `GUARD` does not answer for a stranger** (the row's second
/// clause; Q-45-8). The second pilot runs the picket as the bubble opens, so the raider
/// lights and opens on them while the absent ship is still heaving to. The raider is a party
/// her own party has held hostile since the bubble opened, and it has fired beside her: she
/// locks nothing and fires on nobody, under either word.
#[test]
fn ap9_a_ship_left_on_hold_or_guard_does_not_answer_for_a_stranger() {
    for posture in [Posture::Hold, Posture::Guard] {
        let mut rig = crate::excursions::stocked_fitted();
        let (pilot, row) = docked_dray(&mut rig, ("Pilot", 1, "Salt Moth"), (53, 19));
        let (stranger, other) = docked_dray(&mut rig, ("Stranger", 2, "Tin Kettle"), (53, 19));
        let (hull, beside) = (ObjId(row.hull), ObjId(other.hull));
        crate::excursions::set_rate(&mut rig.tw, 3_600);
        let theirs =
            crate::excursions::file_and_launch(&mut rig, stranger, beside, "+ship/undock stickney");
        let their_boundary = theirs.plan.depart_ic + i64::from(theirs.plan.legs[0].secs);
        let first = left_on_posture_to_the_picket(&mut rig, pilot, hull, posture, their_boundary);

        // The stranger runs.
        let ran = run(&mut rig.tw, stranger, "+helm/run");
        let queued = crate::excursions::drain(&mut rig.tw);
        crate::excursions::land_quietly(&mut rig, &queued);
        let stranger_ship = rig
            .tw
            .world()
            .module_state::<SpaceBubbleBook>()
            .and_then(|book| book.active_for_hull(beside))
            .map(|active| active.carrier_ship_id_for(beside).to_owned())
            .expect("the stranger is in the same bubble");
        let raider_hit_the_stranger = |rig: &Rig| {
            rig.tw
                .world()
                .module_state::<SpaceBubbleBook>()
                .and_then(|book| book.active_for_hull(hull))
                .and_then(|active| active.bubble.fired_on_by(&stranger_ship).cloned())
                .is_some_and(|by| by.contains(&first.raider))
        };
        let mut seconds = 0;
        let mut acted = false;
        let mut last = first.clone();
        while seconds < 600 && !raider_hit_the_stranger(&rig) {
            one_second(&mut rig);
            seconds += 1;
            if let Some(now) = ship_at_the_picket(&rig, hull) {
                acted |= now.lock.is_some() || !now.answered.is_empty();
                last = now;
            }
        }
        assert!(
            raider_hit_the_stranger(&rig),
            "{posture}: the raider's fire arrives on the stranger ({ran}): {last:?}"
        );
        // Two minutes more, with the raider's fire on the record beside her.
        for _ in 0..120 {
            one_second(&mut rig);
            if let Some(now) = ship_at_the_picket(&rig, hull) {
                acted |= now.lock.is_some() || !now.answered.is_empty();
                last = now;
            }
        }
        assert!(
            !last.hit_by_the_raider(),
            "{posture}: the story's premise, the raider fired on the stranger and not on \
             her: {last:?}"
        );
        assert!(
            !acted,
            "{posture}: a stranger's quarrel is not hers: {last:?}"
        );
        assert_ne!(last.status, PicketInspectionStatus::RanThePicket);
    }
}
